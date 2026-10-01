//! Live displacement and triangle safety. Clay commits its complete field in
//! `layer`, as does each Smooth and Relax pass; Flatten writes its positions
//! directly and has no layer rollback. Cumulative
//! shape, per-operation orientation, and the opposing-wall Remove reserve are
//! separate constraints on the same stored f32 surface.

use super::*;

/// Largest displacement step as a fraction of a group's inversion bound — the
/// smallest altitude from the group to the opposite edge of its incident
/// triangles. Half the altitude is the point where a straight-line move reaches
/// the line the triangle would fold through, so this share stops a dab well
/// inside the last safe position regardless of how thin the triangle is.
const MAX_STEP_FRACTION_OF_TRIANGLE: f64 = 0.5;
/// Waves of the tapered rollback before the layer guard restores the whole dab.
/// The reference stops at 8. More waves give the taper room to settle on a dense
/// patch before the reset throws the footprint away. The taper slope does not
/// depend on this number.
const MAX_ROLLBACK_ITERS: usize = 24;
/// Collapse threshold relative to a facet's own pre-dab area — scan meshes
/// contain legitimate very small facets.
const COLLAPSE_FRACTION_SQUARED: f64 = 1e-4;
/// An editable open edge may move, but repeated dabs may not progressively
/// crush a transition relative to the accepted session surface.
pub(super) const MIN_SESSION_AREA_RATIO: f64 = 0.15;
/// One-sided GPU draw floor. Live Smooth and Add share it: a dab that
/// shrinks a face through this area hides the triangle until pointer-up
/// cycle, and the next brush then treats the hole as a defect.
pub(crate) const LIVE_PAINTABLE_AREA: f64 = 1e-6;
/// Diagnostic threshold for counting an indexed face as edge-on in the live
/// trace. It does not participate in any brush or safety decision.
pub(crate) const EDGE_ON_COS: f64 = 0.15;
/// Per-move approach floor on the current edge, independent of the material
/// coordinates inherited from the opening surface. Remeshing changes edges.
const MIN_NEIGHBOUR_APPROACH_SHARE: f64 = 0.35;

/// Geometry shared by the live safety predicates for one triangle. Smooth and
/// clay ask the final and editable laws about the same candidate; keep
/// its cross, area and quality instead of rebuilding them in every question.
#[derive(Clone, Copy)]
struct TriangleMeasure {
    points: [DVec3; 3],
    cross: DVec3,
    area: f64,
    quality: f64,
}

impl TriangleMeasure {
    fn new(points: [DVec3; 3]) -> Self {
        let cross = triangle_cross(points);
        let area = cross.length();
        Self {
            points,
            cross,
            area,
            quality: triangle_quality(points, area),
        }
    }
}

impl SculptSession {
    /// How far this group may travel before it would invert or crush one of its
    /// own incident triangles, in millimetres.
    ///
    /// The measure is the smallest altitude from the group to the edge opposite
    /// it, over every incident triangle: vertex `v` pushes triangle `(v, p, q)`
    /// through zero area exactly when its normal displacement crosses the line
    /// `pq`, and the distance to that line is the altitude `2A / |pq|`.
    ///
    /// A shortest-edge bound does not capture the height of a thin triangle.
    /// Using the smallest incident altitude ties the displacement budget to
    /// the exact distance at which the triangle reaches zero area.
    ///
    /// The result is left uncapped at the top: the caller applies
    /// [`MAX_STEP_FRACTION_OF_TRIANGLE`] as the safety share, and a cap here
    /// would silently replace that share for large triangles. An isolated
    /// group with no incident triangle uses the 1 mm fallback.
    pub(super) fn compute_step_budget(&self) -> Vec<f32> {
        (0..self.topology.group_count() as u32)
            .map(|group| self.step_budget_for(group) as f32)
            .collect()
    }

    /// Smallest altitude from `group` to the opposite edge over its incident
    /// triangles, or 1 mm when it has none.
    pub(super) fn step_budget_for(&self, group: u32) -> f64 {
        let here = self.group_v(group);
        let mut smallest = f64::MAX;
        for &triangle in self.topology.incident_triangles(group) {
            let Some(corners) = self.topology.triangle(triangle) else {
                continue;
            };
            // The edge opposite `group`: the two corners that are not it.
            let mut opposite = [DVec3::ZERO; 2];
            let mut count = 0;
            for &corner in &corners {
                if corner != group {
                    if count < 2 {
                        opposite[count] = self.group_v(corner);
                    }
                    count += 1;
                }
            }
            if count != 2 {
                // A welded triangle naming this group twice has no such edge.
                continue;
            }
            let base = opposite[1] - opposite[0];
            let base_length = base.length();
            if !base_length.is_finite() || base_length <= 1e-12 {
                continue;
            }
            let area2 = (opposite[0] - here).cross(opposite[1] - here).length();
            let altitude = area2 / base_length;
            if altitude.is_finite() && altitude > 0.0 {
                smallest = smallest.min(altitude);
            }
        }
        if smallest == f64::MAX {
            return 1.0;
        }
        smallest
    }

    /// Kept for the callers that want the local edge scale rather than the
    /// inversion bound (the split target and the collapse threshold).
    pub(super) fn shortest_incident_edge(&self, group: u32) -> f64 {
        let here = self.group_v(group);
        let mut shortest = f64::MAX;
        for &neighbor in self.topology.neighbors(group) {
            let length = (self.group_v(neighbor) - here).length();
            if length.is_finite() && length > 0.0 {
                shortest = shortest.min(length);
            }
        }
        if shortest == f64::MAX {
            return 1.0;
        }
        shortest.min(1.0)
    }

    /// Recompute the budget for `groups` and their neighbors from current
    /// positions so the guard stays in step with the moved geometry.
    /// Ratchet upward only: budgets track stretching, which allows larger safe
    /// steps, without reducing the dose as local edges subdivide or compress.
    /// The per-vertex clamp and rollback guard still check live geometry.
    pub(super) fn refresh_step_budget(&mut self, scope: &[u32]) {
        // Callers pass the deduplicated changed groups plus their one-ring.
        #[cfg(feature = "parallel")]
        if scope.len() >= PAR_FLOOR {
            // Each group recomputes from live positions and writes only its
            // own slot, so any worker count collects the identical sequence.
            use rayon::prelude::*;
            let mut computed = std::mem::take(&mut self.budget_scratch);
            computed.resize(scope.len(), 0.0);
            computed
                .par_iter_mut()
                .zip(scope.par_iter())
                .for_each(|(slot, &group)| {
                    *slot = self.step_budget_for(group) as f32;
                });
            for (&group, &recomputed) in scope.iter().zip(&computed) {
                if recomputed > self.step_budget[group as usize] {
                    self.step_budget[group as usize] = recomputed;
                }
            }
            self.budget_scratch = computed;
            return;
        }
        for &group in scope {
            let recomputed = self.step_budget_for(group) as f32;
            if recomputed > self.step_budget[group as usize] {
                self.step_budget[group as usize] = recomputed;
            }
        }
    }

    pub(super) fn clamp_step_at(&self, group: u32, here: DVec3, proposed: DVec3) -> DVec3 {
        self.clamp_step_scaled(group, here, proposed, 1.0)
    }

    /// Per-group triangle-altitude displacement bound for operators that use
    /// a per-vertex step clamp. The swept remesher scales its own work budget
    /// separately; brush Smooth and Relax do not use this as a dose limit.
    pub(super) fn step_limit(&self, group: u32, dabs: f64) -> f64 {
        self.step_budget[group as usize] as f64 * MAX_STEP_FRACTION_OF_TRIANGLE * dabs.max(1.0)
    }

    /// Clamp a displacement to the per-group triangle-altitude bound scaled
    /// for `dabs`, then preserve current one-ring clearance. Native Knife,
    /// Flatten, and remesh relocation use this geometric clamp; it does not
    /// apply a product-specific constraint or control Smooth/Relax dose.
    pub(super) fn clamp_step_scaled(
        &self,
        group: u32,
        here: DVec3,
        proposed: DVec3,
        dabs: f64,
    ) -> DVec3 {
        let step = proposed - here;
        let budget = self.step_limit(group, dabs);
        if !budget.is_finite() || budget <= 0.0 {
            diag::bump(|diag| diag.clamp_zero += 1);
            return here;
        }
        let length = step.length();
        let clamped = if length <= budget || length <= 1e-12 {
            proposed
        } else {
            diag::bump(|diag| diag.clamp_truncated += 1);
            here + (step * (budget / length))
        };
        self.hold_neighbour_distance(group, here, clamped)
    }

    /// Shorten the requested segment at its first neighbour clearance contact.
    /// A guard must preserve identity and direction: projecting a point out of
    /// a sphere centred on an old edge can create a displacement of its own.
    fn hold_neighbour_distance(&self, group: u32, here: DVec3, proposed: DVec3) -> DVec3 {
        let step = proposed - here;
        let length_squared = step.dot(step);
        if length_squared <= 1e-24 {
            return here;
        }
        let mut fraction = 1.0f64;
        for &neighbor in self.topology.neighbors(group) {
            let away = here - self.group_v(neighbor);
            let approach = away.dot(step);
            if approach >= 0.0 {
                continue;
            }
            let clearance = away.dot(away)
                * (1.0 - MIN_NEIGHBOUR_APPROACH_SHARE * MIN_NEIGHBOUR_APPROACH_SHARE);
            let discriminant = approach * approach - length_squared * clearance;
            if discriminant > 0.0 {
                // This equivalent root avoids subtracting nearly equal values.
                let contact = clearance / (-approach + discriminant.sqrt());
                fraction = fraction.min(contact.clamp(0.0, 1.0));
            }
        }
        here + step * fraction
    }

    /// Pre-dab position of a group: the snapshot where stamped this dab,
    /// otherwise the (unmoved) current position.
    pub(super) fn pre_group(&self, group: u32) -> DVec3 {
        if self.snapshot_stamp[group as usize] == self.snapshot_generation {
            let p = self.pre_pos[group as usize];
            DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
        } else {
            self.group_v(group)
        }
    }

    /// Clamp an Erode target to its share of the immutable opposing-wall
    /// allowance. Each side receives half the measured material above the
    /// donor's 0.8 mm floor. Improving a point or moving it outward is free.
    pub(super) fn guard_remove_wall(
        &mut self,
        group: u32,
        current: DVec3,
        candidate: DVec3,
    ) -> DVec3 {
        let axis = if let Some(facing) = self.wall_facing {
            self.sheet_push(group, facing)
        } else {
            let mut normal = self.reference_group_n(group).normalize_or_zero();
            let live_normal = self.group_n(group).normalize_or_zero();
            if normal.length() <= 1e-12 {
                return current;
            }
            if live_normal.length() > 1e-12 && normal.dot(live_normal) < 0.0 {
                normal = -normal;
            }
            normal
        };
        if axis.length() <= 1e-12 {
            return current;
        }
        let reference = self.reference_group_v(group);
        let current_support = (current - reference).dot(axis);
        let candidate_support = (candidate - reference).dot(axis);
        if candidate_support >= 0.0 || candidate_support >= current_support {
            return candidate;
        }
        let reserve = (self.reference_group_wall_mm(group) - 0.8) * 0.5;
        let minimum_support = (-reserve.max(0.0)).min(current_support);
        if candidate_support >= minimum_support {
            return candidate;
        }
        let denominator = current_support - candidate_support;
        if !denominator.is_finite() || denominator <= 0.0 {
            return current;
        }
        let fraction = ((current_support - minimum_support) / denominator).clamp(0.0, 1.0);
        current + (candidate - current) * fraction
    }

    fn triangle_flipped(pre: TriangleMeasure, now: TriangleMeasure) -> bool {
        let pre_area_squared = pre.cross.dot(pre.cross);
        let collapsed = pre_area_squared > 1e-24
            && now.cross.dot(now.cross) <= pre_area_squared * COLLAPSE_FRACTION_SQUARED;
        // A coherent move can rotate a small triangle without tearing it. A
        // reversal below the -0.5 cosine threshold identifies a fold beyond
        // 120 degrees.
        let reversed = pre_area_squared > 1e-24
            && now.cross.dot(pre.cross)
                < -0.5 * pre_area_squared.sqrt() * now.cross.dot(now.cross).sqrt();
        collapsed || reversed
    }

    /// The geometric acceptance contract: may this face be published?
    ///
    /// Final geometry may be far from the starting triangle sizes, but it must
    /// retain winding and usable shape. The test is deliberately free of
    /// lifetime area and edge ratios, so it never resurrects the opening
    /// tessellation. Public because it is a shared guarantee: any consumer that
    /// writes geometry outside a session must be able to ask it too.
    pub fn triangle_final_is_safe(baseline: [DVec3; 3], candidate: [DVec3; 3]) -> bool {
        Self::triangle_final_is_safe_measured(
            TriangleMeasure::new(baseline),
            TriangleMeasure::new(candidate),
        )
    }

    fn triangle_final_is_safe_measured(
        baseline: TriangleMeasure,
        candidate: TriangleMeasure,
    ) -> bool {
        if !Self::triangle_shape_is_safe_measured(baseline, candidate) {
            return false;
        }
        if baseline.area > 1e-12
            && candidate.cross.dot(baseline.cross) < -0.5 * candidate.area * baseline.area
        {
            // The same -0.5 cosine threshold rejects a winding reversal beyond
            // 120 degrees while permitting gradual cumulative surface changes.
            // Unnormalized dots avoid unstable directions from near-degenerate
            // crosses.
            return false;
        }
        true
    }

    /// Shape is cumulative; orientation is checked per operation. Clay checks
    /// winding against the pre-dab face while this rule limits the
    /// final shape relative to the session baseline.
    fn triangle_shape_is_safe_measured(
        baseline: TriangleMeasure,
        candidate: TriangleMeasure,
    ) -> bool {
        const MIN_QUALITY: f64 = 0.01;
        const MIN_EDGE_MM: f64 = 1e-6;
        const MIN_DOUBLE_AREA_MM2: f64 = 1e-10;
        if !candidate.area.is_finite() || candidate.area <= MIN_DOUBLE_AREA_MM2 {
            return false;
        }
        // Some accepted analytic bands are already narrower than the generic
        // floor. Identity Apply must remain valid; sculpt may not make such a
        // face worse, while ordinary faces retain the absolute floor.
        if candidate.quality + 1e-12 < baseline.quality.min(MIN_QUALITY) {
            return false;
        }
        [(0, 1), (1, 2), (2, 0)]
            .into_iter()
            .all(|(a, b)| (candidate.points[b] - candidate.points[a]).length() > MIN_EDGE_MM)
    }

    /// Live Smooth, Erode and Deposit use this predicate: the move
    /// must satisfy the Apply contract against the pre-dab face, and it may
    /// not shrink a face below [`LIVE_PAINTABLE_AREA`] when its input area is
    /// above that floor.
    ///
    /// Triangle shape above the Apply floor is not a displacement limit. The
    /// same dab runs the live remesh right after it moves the surface, and the
    /// flip and collapse there repair a thin face. A shape floor here refused
    /// the field instead, and its rollback wave damped the dose over up to
    /// eight rings around every thin face, so an already remeshed patch took a
    /// weaker brush on every later stroke.
    fn triangle_editable_after_move_measured(
        baseline: TriangleMeasure,
        candidate: TriangleMeasure,
    ) -> bool {
        let baseline_is_valid = Self::triangle_final_is_safe_measured(baseline, baseline);
        if baseline_is_valid {
            if !Self::triangle_final_is_safe_measured(baseline, candidate) {
                return false;
            }
        } else {
            // Scans, and meshes already edited, can contain sub-micron
            // edges or faces below the generic area floor. Their exact
            // identity, and any move that improves all three local measures,
            // must remain editable; otherwise rollback can never converge and
            // one old sliver turns the connected brush into a dead zone.
            let tolerance = layer::clay_area_roundoff(baseline, candidate);
            let shortest = |measure: TriangleMeasure| {
                [(0, 1), (1, 2), (2, 0)]
                    .into_iter()
                    .map(|(a, b)| (measure.points[b] - measure.points[a]).length())
                    .fold(f64::INFINITY, f64::min)
            };
            if !candidate.area.is_finite()
                || candidate.area + tolerance < baseline.area
                || candidate.quality + 1e-12 < baseline.quality
                || shortest(candidate) + 1e-12 < shortest(baseline)
            {
                return false;
            }
        }
        if candidate.area + 1e-18 < LIVE_PAINTABLE_AREA && candidate.area + 1e-18 < baseline.area {
            return false;
        }
        true
    }
}

mod layer;

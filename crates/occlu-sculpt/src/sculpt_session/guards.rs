//! Live displacement and triangle safety. Clay commits its complete field in
//! `layer`, as does each Smooth pass; Flatten retains local rollback. Cumulative
//! shape, per-operation orientation, camera coverage, and the Remove wall
//! reserve are separate constraints on the same stored f32 surface.

use super::*;

/// Largest displacement step as a fraction of a group's inversion bound — the
/// smallest altitude from the group to the opposite edge of its incident
/// triangles. Half the altitude is the point where a straight-line move reaches
/// the line the triangle would fold through, so this share stops a dab well
/// inside the last safe position regardless of how thin the triangle is.
const MAX_STEP_FRACTION_OF_TRIANGLE: f64 = 0.5;
/// Passes of the post-dab inversion guard.
const MAX_ROLLBACK_ITERS: usize = 8;
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
/// Cosine of the angle between a face and the toward-camera axis. Below this
/// a large face is a slit: still in the index, drawn empty (or as the red
/// interior once it goes back). 0.15 is ~9° from edge-on.
pub(crate) const EDGE_ON_COS: f64 = 0.15;
/// Per-move approach floor on the current edge, independent of the material
/// coordinates inherited from the opening surface. Remeshing changes edges.
const MIN_NEIGHBOUR_APPROACH_SHARE: f64 = 0.35;

/// Geometry shared by the live safety predicates for one triangle. Smooth and
/// clay ask the final, editable and camera laws about the same candidate; keep
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
    /// The measure is the smallest ALTITUDE from the group to the edge opposite
    /// it, over every incident triangle: vertex `v` pushes triangle `(v, p, q)`
    /// through zero area exactly when its normal displacement crosses the line
    /// `pq`, and the distance to that line is the altitude `2A / |pq|`.
    ///
    /// This replaces `shortest_incident_edge * 0.5`, which was an invented
    /// bound. `h_min / 2` is not only arbitrary, it is unsafe for a thin
    /// triangle: a needle with a long opposite edge has an altitude far below
    /// half its shortest edge, so the old budget let a dab invert it, while a
    /// well-shaped triangle's altitude is roughly `h * sqrt(3) / 2` and the old
    /// budget was needlessly strict there. Reading the geometry directly
    /// removes the guess in both directions.
    ///
    /// The result is left uncapped at the top: the caller applies
    /// [`MAX_STEP_FRACTION_OF_TRIANGLE`] as the safety share, and a cap here
    /// would silently replace that share for large triangles. An isolated
    /// group with no incident triangle keeps the generous historical fallback.
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
    /// Ratchet-up only: budgets track stretching (which genuinely allows
    /// bigger safe steps) but never subdivision or compression. Shrinking
    /// every budget as the mesh refines silently halves the dab's dose
    /// stroke after stroke (measured: 17x collapse over 24 dabs); tight
    /// moves stay safe through the per-vertex clamp below and the rollback
    /// net, which both read live geometry rather than this heuristic.
    pub(super) fn refresh_step_budget(&mut self, scope: &[u32]) {
        // Callers pass the deduplicated changed groups plus their one-ring.
        #[cfg(feature = "parallel")]
        if scope.len() >= kernel::PAR_FLOOR {
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

    /// The single gain a clay dab spends its anti-inversion budget as.
    ///
    /// One budget for the whole footprint, so the falloff — not the local
    /// triangle size — decides the dab's shape. The per-vertex clamp remains as
    /// the last line of defence, but it does not decide the profile: a mesh
    /// whose shortest incident edge varies several-fold inside one footprint
    /// would otherwise make the dab follow its tessellation, with each vertex
    /// delivering its own fraction of the falloff (pinned by
    /// `one_dab_keeps_its_falloff_profile_on_an_anisotropic_lattice`).
    ///
    /// The budget is the median of the moving vertices' step budgets, so a
    /// single fine band under the brush cannot throttle the whole dab while a
    /// uniformly fine mesh still scales its dose down and keeps the profile.
    pub(super) fn dab_step_gain(&mut self, weighted: &[(u32, f64)], amplitude: f64) -> f64 {
        let mut peak = 0.0f64;
        for &(_, weight) in weighted {
            peak = peak.max(weight);
        }
        // A swept step's weight counts dabs. The budget binds each dab's
        // share, as it did when the dabs ran one by one, so a fast hand does
        // not deposit less per millimetre than a slow one.
        peak = peak.min(1.0) * amplitude;
        if peak <= 0.0 || !peak.is_finite() {
            return 1.0;
        }
        // Budgets ratchet up-only (see `refresh_step_budget`), so deformed
        // edges never shrink the median mid-stroke: the dose stays put while
        // each vertex stays individually clamped below.
        let mut edges = std::mem::take(&mut self.percentile_scratch);
        edges.clear();
        edges.extend(
            weighted
                .iter()
                .map(|&(group, _)| self.step_budget[group as usize]),
        );
        let gain = (|| {
            if edges.is_empty() {
                return 1.0;
            }
            let slot = (edges.len() - 1) / 2;
            let (_, edge, _) = edges.select_nth_unstable_by(slot, f32::total_cmp);
            let budget = *edge as f64 * MAX_STEP_FRACTION_OF_TRIANGLE;
            if !budget.is_finite() || budget <= 0.0 || budget >= peak {
                return 1.0;
            }
            let permille = ((budget / peak) * 1000.0).round().clamp(0.0, 1000.0) as u32;
            diag::bump(|diag| {
                diag.gain_scaled_dabs += 1;
                diag.gain_min_permille = diag.gain_min_permille.min(permille);
            });
            budget / peak
        })();
        self.percentile_scratch = edges;
        gain
    }

    pub(super) fn clamp_step_at(&self, group: u32, here: DVec3, proposed: DVec3) -> DVec3 {
        self.clamp_step_scaled(group, here, proposed, 1.0)
    }

    /// [`Self::clamp_step_at`] for a move that stands for `dabs` dab-equivalents
    /// of a swept step: the per-dab budget applies to each of them. The
    /// neighbour clearance and the product constraint are geometric limits of
    /// the final position and stay whole.
    pub(super) fn clamp_step_scaled(
        &self,
        group: u32,
        here: DVec3,
        proposed: DVec3,
        dabs: f64,
    ) -> DVec3 {
        let step = proposed - here;
        let budget =
            self.step_budget[group as usize] as f64 * MAX_STEP_FRACTION_OF_TRIANGLE * dabs.max(1.0);
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

    fn triangle_flipped(pre: TriangleMeasure, now: TriangleMeasure) -> bool {
        let pre_area_squared = pre.cross.dot(pre.cross);
        let collapsed = pre_area_squared > 1e-24
            && now.cross.dot(now.cross) <= pre_area_squared * COLLAPSE_FRACTION_SQUARED;
        // A coherent move can tumble very small source triangles without
        // tearing anything. A real fold
        // turns the normal most of the way around (cos near -1), so only
        // that votes: measured tumble sits at cos ~ -0.13, true folds at
        // cos ~ -1, and the margin between them is wide.
        let reversed = pre_area_squared > 1e-24
            && now.cross.dot(pre.cross)
                < -0.5 * pre_area_squared.sqrt() * now.cross.dot(now.cross).sqrt();
        collapsed || reversed
    }

    fn triangle_baseline_unsafe(
        baseline: TriangleMeasure,
        pre_dab: TriangleMeasure,
        candidate: TriangleMeasure,
        mode: BrushMode,
    ) -> bool {
        if mode == BrushMode::Deposit && baseline.area > 1e-12 {
            if candidate.area + 1e-12 < baseline.area * MIN_SESSION_AREA_RATIO {
                return true;
            }
            if candidate.cross.dot(baseline.cross) < 0.0 {
                // Building material may bend an open transition, but it
                // may not roll that surface through its accepted inside.
                return true;
            }
        }
        if !Self::triangle_final_is_safe_measured(baseline, candidate) {
            return true;
        }
        // The same Apply floor again, against the *pre-dab* face, plus the
        // paintable-area floor, so this dab cannot hide a triangle.
        if !Self::triangle_editable_after_move_measured(pre_dab, candidate) {
            return true;
        }
        false
    }

    fn triangle_is_unsafe(&self, triangle: u32, mode: BrushMode) -> bool {
        let Some(corners) = self.topology.triangle(triangle) else {
            return false;
        };
        let pre_dab = TriangleMeasure::new(corners.map(|group| self.pre_group(group)));
        let candidate =
            TriangleMeasure::new(corners.map(|group| stored_position(self.group_v(group))));
        if (0..3).all(|corner| {
            let a = pre_dab.points[corner];
            let b = candidate.points[corner];
            a.x == b.x && a.y == b.y && a.z == b.z
        }) {
            return false;
        }
        if Self::triangle_flipped(pre_dab, candidate) {
            return true;
        }
        let baseline = TriangleMeasure::new(corners.map(|group| self.reference_group_v(group)));
        Self::triangle_baseline_unsafe(baseline, pre_dab, candidate, mode)
            || Self::triangle_hides_from_camera_measured(pre_dab, candidate, self.camera_context())
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
            // Winding: a tear turns the normal most of the way around
            // (cos near -1), while legitimate reshaping rotates it
            // gradually — a carved wall legitimately rolls ~90deg from a
            // flat session baseline (measured: a healthy 0.33 mm groove
            // wall at 94deg). The per-dab flip predicate uses the same
            // >120deg tear standard, so the two never disagree about what
            // counts as torn. Uses unnormalized dots: `normalized()` on a
            // near-degenerate cross fabricates a direction from noise.
            return false;
        }
        true
    }

    /// Shape is a cumulative property; a rotation limit is a single-operation
    /// property. Clay proves winding against the previous dab and the camera,
    /// so gradual growth cannot hit a lifetime angle cap against the old scan.
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

    /// Live Smooth, Erode and no-camera Deposit use this predicate: the move
    /// must satisfy the Apply contract against the pre-dab face, and it may
    /// not shrink a face below [`LIVE_PAINTABLE_AREA`] unless the face was
    /// already that small.
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

    pub(super) fn stroke_view(&self) -> DVec3 {
        DVec3::new(
            self.live_kin.vx as f64,
            self.live_kin.vy as f64,
            self.live_kin.vz as f64,
        )
    }

    fn stroke_facing(&self) -> f64 {
        self.live_kin.facing as f64
    }

    fn camera_context(&self) -> Option<(DVec3, f64)> {
        let view = self.stroke_view();
        let facing = self.stroke_facing();
        (view.length() > 1e-12 && facing != 0.0).then(|| (view.normalize_or_zero(), facing))
    }

    /// A depth stroke may not erase a face's signed projected area.
    ///
    /// Add/Remove move along the view axis, so their screen-space triangle is
    /// invariant even when the 3-D triangle becomes a steep wall. A cosine
    /// floor mistakes that legitimate wall for a slit because its geometric
    /// area grows while its projected area stays fixed; it was also blind to
    /// an already-grazing face crossing through zero. Compare the signed
    /// projection directly instead. A healthy face keeps a paintable fraction
    /// of its former projection; a face already at the silhouette may not lose
    /// any more. This protects existing steep walls without pinning a depth
    /// layer merely because the layer changed the face normal.
    fn triangle_hides_from_camera_measured(
        baseline: TriangleMeasure,
        candidate: TriangleMeasure,
        camera: Option<(DVec3, f64)>,
    ) -> bool {
        let Some((view, facing)) = camera else {
            return false;
        };
        let pre_cross = baseline.cross * facing;
        let now_cross = candidate.cross * facing;
        let pre_geom = pre_cross.length();
        let now_geom = now_cross.length();
        if pre_geom <= 1e-18 {
            return false;
        }
        if now_geom <= 1e-18 {
            return true;
        }
        let pre_cam = -pre_cross.dot(view);
        let now_cam = -now_cross.dot(view);
        let tolerance = layer::clay_area_roundoff(baseline, candidate);
        if pre_cam > tolerance {
            let pre_cos = pre_cam / pre_geom;
            let floor = if pre_cos <= EDGE_ON_COS {
                pre_cam
            } else {
                (pre_geom * EDGE_ON_COS).min(pre_cam)
            };
            return now_cam + tolerance < floor;
        }
        // A numerically edge-on face has no measurable covering area to
        // preserve, but it still may not cross onto the hidden side.
        pre_cam >= -tolerance && now_cam < -tolerance
    }
}

mod layer;

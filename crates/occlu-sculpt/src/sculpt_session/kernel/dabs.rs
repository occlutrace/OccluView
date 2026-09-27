//! Clay, flatten and skirt solvers: the dab-shape family beside Smooth.
//!
//! `dab()` in the parent dispatches here. Region build, weights, budgets,
//! walls and undo stay in the parent; this module owns how selected groups
//! move: clay displacement, plane levelling, the preserve skirt, the shared-
//! kernel surface adapter and the Taubin relaxation pass.

use super::*;

/// Smooth the new material, not the shape that existed before this dab.
/// The live buffer is private staging until the whole layer is accepted.
/// Session groups behind the shared kernel's surface traits: positions and
/// normals read live, neighbour rings borrow the welded topology. One
/// adapter serves the flatten projector and the knot clamp alike, so the two
/// never disagree about what "the surface" is.
struct GroupKnot<'s> {
    session: &'s SculptSession,
}

impl crate::FlattenSurface for GroupKnot<'_> {
    fn flatten_position(&self, vertex: u32) -> DVec3 {
        self.session.group_v(vertex)
    }
}

impl crate::KnotSurface for GroupKnot<'_> {
    fn knot_position(&self, vertex: u32) -> DVec3 {
        self.session.group_v(vertex)
    }
    fn knot_normal(&self, vertex: u32) -> DVec3 {
        self.session.group_n(vertex)
    }
    fn knot_neighbors(&self, vertex: u32) -> &[u32] {
        self.session.topology.neighbors(vertex)
    }
}
/// Dijkstra frontier entry for the preserve skirt. Ordered by shortest-path
/// distance (then vertex id) only: distance and inherited magnitude ride
/// along unordered, because floats have no total order to give a heap.
#[derive(Clone, Copy)]
struct SkirtFrontier {
    order: (u64, u32),
    dist: f64,
    from: f64,
}

impl PartialEq for SkirtFrontier {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order
    }
}
impl Eq for SkirtFrontier {}
impl PartialOrd for SkirtFrontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SkirtFrontier {
    fn cmp(&self, other: &Self) -> Ordering {
        self.order.cmp(&other.order)
    }
}

impl SculptSession {
    /// Ctrl shape-preserve skirt: when armed, motion continues past the
    /// footprint with a smoothstep falloff instead of ending at the rim.
    /// Reach is PHYSICAL (millimetres past the selection, not hop count):
    /// a ring-count reach changes with source tessellation — three hops may
    /// span 3 mm on one mesh and 0.3 mm on another — so the skirt would
    /// otherwise have no stable physical width. Each skirt vertex
    /// continues its discoverer's own magnitude (`(1-alpha) * parent`), so
    /// the falloff has no step at the boundary; masked groups are barriers
    /// (operator-locked: neither moved nor walked through) and back-facing
    /// groups stay culled, exactly like the interior law. Skirt vertices
    /// join `weighted`, so the plane, the  the knot, the budget and
    /// the wall reserve all treat them as ordinary dab members.
    fn extend_preserve_skirt(
        &self,
        weighted: &mut Vec<(u32, f64)>,
        view: DVec3,
        facing: f64,
        radius: f64,
    ) {
        if !self.preserve_skirt || weighted.is_empty() || !(radius > 0.0) {
            return;
        }
        let rings = crate::DEFAULT_PRESERVE_RINGS;
        // Three-quarter radius past the selection edge: several rings at
        // target density, well under the brush scale so the skirt accents
        // the dab instead of becoming it.
        let reach = 0.75 * radius;
        // Dijkstra frontier: settling happens at POP time — the popped entry
        // is final only if no shorter path was recorded since it was
        // pushed. Marking vertices seen at PUSH time lets discovery order
        // win, and on a dense mesh the short way is one of many: a
        // longer first discovery would poison the vertex with an inflated
        // distance. So a shorter re-discovery re-pushes, and a stale pop
        // is skipped. BinaryHeap is a max-heap: the reversed key pops the
        // smallest distance first. (bits, id) is a total order — bit patterns
        // order like the non-negative floats they encode — so two runs agree
        // bit for bit. Each entry carries its distance and inherited
        // magnitude alongside the key.
        let mut frontier: BinaryHeap<std::cmp::Reverse<SkirtFrontier>> =
            BinaryHeap::with_capacity(weighted.len() * 2);
        let mut best: std::collections::HashMap<u32, (f64, f64)> =
            std::collections::HashMap::with_capacity(weighted.len() * 2);
        for &(group, weight) in weighted.iter() {
            // Every seed is settled at distance zero before the walk, finite
            // or not: a non-finite seed pushes nothing but still bars the
            // walk from reaching it from behind.
            best.insert(group, (0.0, weight));
            // A non-finite seed would launder NaN into the skirt: skip it.
            if weight.is_finite() {
                frontier.push(std::cmp::Reverse(SkirtFrontier {
                    order: (0u64, group),
                    dist: 0.0,
                    from: weight,
                }));
            }
        }
        while let Some(std::cmp::Reverse(entry)) = frontier.pop() {
            let group = (entry.order).1;
            let dist = entry.dist;
            let from = entry.from;
            // Stale pop: a shorter path to this group was recorded since.
            if best.get(&group).is_some_and(|&(d, _)| dist > d) {
                continue;
            }
            let here = self.group_v(group);
            for &neighbor in self.topology.neighbors(group) {
                let step = (self.group_v(neighbor) - here).length();
                if !step.is_finite() {
                    continue;
                }
                let ndist = dist + step;
                if !(ndist <= reach) {
                    continue;
                }
                // Ring 0 is the selection itself (`preserve_alpha` law).
                let share = (1.0
                    - crate::preserve_alpha(((ndist / reach) * rings as f64) as u32, rings))
                    * from;
                if !(share > 0.0) {
                    continue;
                }
                // Only a strictly shorter path re-pushes: equal paths agree
                // bit for bit already, and re-pushing them would only burn
                // the budget revisiting settled vertices.
                if best.get(&neighbor).is_some_and(|&(d, _)| ndist >= d) {
                    continue;
                }
                let front = frontface_weight(self.group_n(neighbor) * facing, view);
                if front > 0.0 {
                    weighted.push((neighbor, share * front));
                }
                best.insert(neighbor, (ndist, share));
                frontier.push(std::cmp::Reverse(SkirtFrontier {
                    order: (ndist.to_bits(), neighbor),
                    dist: ndist,
                    from: share,
                }));
            }
        }
    }

    /// Add/Remove moves along the surface normal under the brush with a
    /// signed layer. Taubin filters
    /// only that new displacement before the whole field is committed; a
    /// rejected layer cannot keep smoothing or pulling the old surface back.
    // the clay field, its denoise and its commit are one operator.
    #[allow(clippy::too_many_lines)]
    pub(super) fn dab_clay(&mut self, dab: &Dab, region: &[SurfacePoint], facing: f64, sign: f64) {
        let strength = dab.strength.clamp(0.0, 1.0);
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| {
            self.weight(point, dab, facing)
        });
        self.extend_preserve_skirt(&mut weighted, dab.view, facing, dab.radius);
        // A swept step's weights count dabs; the densest point of the trail
        // bounds how much dose the denoise below may carry.
        let dab_peak = weighted.iter().map(|&(_, w)| w).fold(1.0f64, f64::max);
        // Material grows out of the surface under the brush, not toward the
        // camera: one direction per step, the area-weighted normal of the
        // footprint as the operator sees it. A tilted view then builds on the
        // slope it touches instead of stacking material along the line of
        // sight. The layer guard still refuses a face the lift would fold or
        // hide, and the single direction keeps the dose denoise below exact.
        let knife = self.brush_tip == TipStamp::Knife;
        let view = dab.view.normalize_or_zero();
        let push = self.brush_normal(&weighted, dab.view, facing);
        let hit_n = self
            .hit_triangle
            .and_then(|triangle| self.triangle_normal(triangle))
            .unwrap_or(DVec3::ZERO);
        // Reference amplitude (`tests/brush.rs:one_full_strength_dab_...`):
        // radius-relative so the brush feels the same at every zoom; per-dab
        // small since a drag accumulates arc-length-spaced dabs.
        let amplitude = (dab.radius * ADD_REMOVE_GAIN * strength * self.dab_exposure).max(0.0);
        if push.length() <= 1e-12 || amplitude <= 0.0 {
            self.weights = weighted;
            return;
        }
        // The median-edge budget scales the entire footprint once.
        let gain = self.dab_step_gain(&weighted, amplitude);
        // Tip gain after the budget fit, never inside it: the fit preserves
        // the falloff profile against the step budget, and folding the tip
        // into the amplitude lets the fit normalize it straight back out
        // (folding a knife gain into the amplitude dug as deep as a ball). The
        // per-vertex clamp below still bounds every move, so safety never
        // depended on it.
        //
        // A knife is narrower, so it must bite harder at its centre to cut at
        // all — but the response is a RATE, and a stroke paints many dabs, so a
        // large centre gain does not buy reach: it only makes the first dab of a
        // fast stroke gouge. Measured at equal settings against the ball, the
        // previous 3.0 delivered 3.85x at a 0.5 mm radius and 3.05x at 4 mm, and
        // at the top of the range one dab took 0.5 mm off a surface whose
        // minimum permitted wall is 0.5 mm. 2.0 keeps the tip decisive — still
        // clearly deeper than the ball at every radius — with the disproportion
        // roughly halved. Pinned by `knife_cuts_deeper_than_ball`.
        let tip_gain = match self.brush_tip {
            TipStamp::Knife => 2.0,
            TipStamp::Ball | TipStamp::Cylinder => 1.0,
        };
        let erode = sign < 0.0;
        let mode = if erode {
            BrushMode::Erode
        } else {
            BrushMode::Deposit
        };
        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        let mut front_min = 1.0f64;
        let mut front_max = 0.0f64;
        let mut front_sum = 0.0f64;
        let mut toward = 0.0f64;
        let mut spread = 0.0f64;
        for &(group, weight) in &weighted {
            let here = self.group_v(group);
            let signed = self.group_n(group) * facing;
            let facing_w = self.facing_weight(group, dab.view, facing, dab.mode);
            front_min = front_min.min(facing_w);
            front_max = front_max.max(facing_w);
            front_sum += facing_w;
            if signed.length() > 1e-12
                && view.length() > 1e-12
                && signed.normalize_or_zero().dot(view) <= 0.0
            {
                toward += 1.0;
            }
            if push.length() > 1e-12 && signed.length() > 1e-12 {
                let aligned = signed.normalize_or_zero().dot(push).clamp(-1.0, 1.0);
                spread = spread.max(aligned.acos().to_degrees());
            }
            let dir = push;
            // Safety sees the complete displacement field after denoise.
            let target = here + (dir * (sign * weight * amplitude * gain * tip_gain));
            let clamped = if knife {
                // Each dab-equivalent of a swept step takes the tearing clamp
                // on its own share, exactly as consecutive dabs did.
                let dabs = weight.max(1.0);
                let share = crate::clamp_dab_displacement(
                    &GroupKnot { session: &*self },
                    group,
                    1.0,
                    (target - here) * (1.0 / dabs),
                );
                self.clamp_step_scaled(group, here, here + share * dabs, dabs)
            } else {
                // A per-vertex tangent or neighbour clamp changes the common
                // direction and creates a different layer at every edge size.
                // The simultaneous face guard below bounds the complete field.
                target
            };
            if (clamped - here).length() > 1e-15 {
                proposals.push((group, clamped));
            }
        }
        let count = weighted.len() as f64;
        self.live_kin.local_normal = false;
        self.live_kin.hit_sheet = false;
        self.live_kin.nx = push.x as f32;
        self.live_kin.ny = push.y as f32;
        self.live_kin.nz = push.z as f32;
        self.live_kin.facing = facing as f32;
        self.live_kin.amplitude = amplitude as f32;
        self.live_kin.gain = gain as f32;
        self.live_kin.weighted = weighted.len() as u32;
        self.live_kin.proposals = proposals.len() as u32;
        self.live_kin.front_min = if count > 0.0 { front_min as f32 } else { 0.0 };
        self.live_kin.front_max = front_max as f32;
        self.live_kin.front_mean = if count > 0.0 {
            (front_sum / count) as f32
        } else {
            0.0
        };
        self.live_kin.toward_frac = if count > 0.0 {
            (toward / count) as f32
        } else {
            0.0
        };
        self.live_kin.cx = dab.center.x as f32;
        self.live_kin.cy = dab.center.y as f32;
        self.live_kin.cz = dab.center.z as f32;
        self.live_kin.vx = view.x as f32;
        self.live_kin.vy = view.y as f32;
        self.live_kin.vz = view.z as f32;
        self.live_kin.hn_x = hit_n.x as f32;
        self.live_kin.hn_y = hit_n.y as f32;
        self.live_kin.hn_z = hit_n.z as f32;
        self.live_kin.target_mm = RemeshPolicy::standard()
            .target_for_radius(dab.radius)
            .unwrap_or(0.0) as f32;
        self.live_kin.flood_mm = dab.radius as f32;
        self.live_kin.n_spread_deg = self.live_kin.n_spread_deg.max(spread as f32);
        // Radial post-smoothing widens the knife's narrow trail and can erase
        // its depth. Ball and cylinder retain clay denoise; the knife keeps
        // its travel-aligned analytic footprint.
        if self.brush_tip == TipStamp::Knife {
            self.commit_even_layer(&proposals, mode);
            self.proposals = proposals;
            self.weights = weighted;
            return;
        }
        // Skirt members can lie outside the original flood. Capture them
        // before staging, so all field samples have a fixed zero displacement.
        for &(group, _) in &proposals {
            if self.snapshot_stamp[group as usize] != self.snapshot_generation {
                let p = self.group_v(group);
                self.pre_pos[group as usize] = [p.x as f32, p.y as f32, p.z as f32];
                self.snapshot_stamp[group as usize] = self.snapshot_generation;
                self.dab_groups.push(group);
            }
        }
        // Filter the new layer across the plateau and taper at the rim.
        // Masks and facing bound both the lift and its denoise support.
        weighted.clear();
        Self::weigh_region_into(region, &mut weighted, |point| {
            // The denoise plateau stays radial (ball) whatever the tip: it
            // removes grain, it does not shape, so it must not inherit the
            // knife edge's focus or the cylinder's plateau.
            let f = crate::ball_weight(point.distance, dab.radius);
            if f <= 0.0 {
                return 0.0;
            }
            let t = f.sqrt(); // t = 1 - distance/radius
            smoothstep(AUTOSMOOTH_RIM_TAPER, t)
                * self.facing_weight(point.group, dab.view, facing, dab.mode)
        });
        // The denoise runs on the DOSE, not on the surface.
        //
        // Every proposal is `pre + push * sign * amount` with one constant
        // `push` and one constant `sign` for the whole dab, so the along-push
        // scalar `amount` carries the entire dab and the Laplacian over it is
        // the Laplacian of the displacement field. The previous formulation
        // wrote the raw layer into the live mesh, re-read it four times as a
        // vector field, wrote each pass back, rebuilt the proposals from the
        // mesh and finally restored every position — five whole-region writes
        // per dab to compute a scalar. It is now computed where it lives and
        // the live mesh is never touched before the single commit.
        //
        // `amount` is seeded to zero over the snapshot, which is exactly what
        // the mesh formulation read as an unmoved increment, and the raw
        // proposals overwrite it. The pass is collected before it is applied,
        // so it stays order-independent.
        let mut amount = std::mem::take(&mut self.denoise_amount);
        for index in 0..self.dab_groups.len() {
            amount[self.dab_groups[index] as usize] = 0.0;
        }
        for &(group, target) in &proposals {
            amount[group as usize] = (target - self.pre_group(group)).dot(push) * sign;
        }
        // Denoise may redistribute this dab's dose, but cannot turn Add into
        // Remove or amplify it past its budget.
        let dose_cap = amplitude * gain * tip_gain * dab_peak;
        let mut pass = std::mem::take(&mut self.denoise_pass);
        for _ in 0..CLAY_AUTOSMOOTH_PASSES {
            for factor in [TAUBIN_LAMBDA, TAUBIN_MU] {
                pass.clear();
                for &(group, weight) in &weighted {
                    let ring = self.topology.neighbors(group);
                    if ring.len() < 2 {
                        continue;
                    }
                    let mut mean = 0.0;
                    for &neighbor in ring {
                        mean += amount[neighbor as usize];
                    }
                    mean /= ring.len() as f64;
                    let here = amount[group as usize];
                    let t = (factor * weight).clamp(-1.0, 1.0);
                    let relaxed = here + (mean - here) * t;
                    if (relaxed - here).abs() > 1e-15 {
                        pass.push((group, relaxed.clamp(0.0, dose_cap)));
                    }
                }
                for &(group, value) in &pass {
                    amount[group as usize] = value;
                }
            }
        }
        // The accepted field is committed once and is never locally tapered
        // later. The live mesh still holds the pre-dab positions, so the layer
        // guard reads them as the origins it expects.
        proposals.clear();
        for index in 0..self.dab_groups.len() {
            let group = self.dab_groups[index];
            let here = self.pre_group(group);
            let target = here + (push * (sign * amount[group as usize]));
            if (target - here).length() > 1e-15 {
                proposals.push((group, target));
            }
        }
        self.denoise_amount = amount;
        self.denoise_pass = pass;
        self.commit_even_layer(&proposals, mode);
        self.proposals = proposals;
        self.weights = weighted;
    }

    /// Flatten (Minus): level the selection toward its own plane — the
    /// weighted centroid along the camera-oriented brush normal. A bump
    /// sinks, a dent rises, an already-flat patch stays bit-exact. Like a
    /// carve it can thin a wall, so every proposal passes the same step
    /// budget and wall reserve; like every dab it cannot tear, so the knot
    /// clamp binds first. No denoise tail: levelling is the finish.
    pub(super) fn dab_flatten(&mut self, dab: &Dab, region: &[SurfacePoint], facing: f64) {
        let strength = dab.strength.clamp(0.0, 1.0);
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| {
            self.weight(point, dab, facing)
        });
        self.extend_preserve_skirt(&mut weighted, dab.view, facing, dab.radius);
        let mut centroid = DVec3::ZERO;
        let mut total = 0.0f64;
        for &(group, weight) in &weighted {
            centroid += self.group_v(group) * weight;
            total += weight;
        }
        let normal = self.brush_normal(&weighted, dab.view, facing);
        if total <= 1e-12 || normal.length() <= 1e-12 {
            self.weights = weighted;
            return;
        }
        let point = centroid * (1.0 / total);
        let mut gap = 0.0f64;
        for &(group, _) in &weighted {
            gap = gap.max((point - self.group_v(group)).dot(normal).abs());
        }
        // The dose is absolute here too: the per-vertex clamp below is the
        // only thing that bounds a move, and the flatten gap is already the
        // local depth the operator asked for.
        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        // Phase one reads only: the surface adapter borrows the session
        // shared, so budgets and walls (which need it mutable) wait for
        // phase two. One adapter serves both laws: the surface the
        // projector flattens is the surface the knot refuses to tear.
        let swept = self.path_active();
        let displacements: Vec<(u32, DVec3, f64)> = {
            let surface = GroupKnot { session: &*self };
            let mut out = Vec::with_capacity(weighted.len());
            for &(group, weight) in &weighted {
                // A swept step's weight counts dabs, each levelling `strength`
                // of the remaining way: compound them instead of multiplying,
                // which would overshoot through the plane. The fold clamps stay
                // one dab's: a flatten levels its whole swath to one plane and
                // commits without the layer guard.
                let (share, dabs) = if swept && strength > 0.0 {
                    (compounded_share(weight, strength) / strength, 1.0)
                } else {
                    (weight, 1.0)
                };
                let displacement =
                    crate::flatten_displacement(&surface, group, share, strength, point, normal);
                // Weight 1.0 — flatten already applied it, and held vertices
                // never reach this list.
                let displacement = crate::clamp_dab_displacement(
                    &surface,
                    group,
                    1.0,
                    displacement * (1.0 / dabs),
                ) * dabs;
                out.push((group, displacement, dabs));
            }
            out
        };
        for (group, displacement, dabs) in displacements {
            let here = self.group_v(group);
            let target = self.clamp_step_scaled(group, here, here + displacement, dabs);
            if (target - here).length() > 1e-15 {
                proposals.push((group, target));
            }
        }
        for &(group, target) in &proposals {
            self.write_group_position(group, target);
        }
        self.proposals = proposals;
        self.weights = weighted;
    }
}

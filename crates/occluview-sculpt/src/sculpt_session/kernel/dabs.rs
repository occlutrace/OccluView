//! Clay, flatten and skirt solvers: the dab-shape family beside Smooth.
//!
//! `dab()` in the parent dispatches here. Region build, weights, budgets,
//! walls and undo stay in the parent; this module owns how selected groups
//! move: clay displacement, plane levelling, the preserve skirt, the shared-
//! kernel surface adapter and the Taubin relaxation pass.

use super::*;
use crate::hash::FxHashMap;

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
    /// the falloff has no step at the boundary.
    /// Groups on the reverse sheet remain excluded, exactly like the interior
    /// law. Skirt vertices join `weighted`, so the shape operator and layer
    /// guard treat them as ordinary dab members.
    fn extend_preserve_skirt(&mut self, weighted: &mut Vec<(u32, f64)>, radius: f64) {
        if !self.preserve_skirt || weighted.is_empty() || !(radius > 0.0) {
            return;
        }
        let rings = crate::DEFAULT_PRESERVE_RINGS;
        // Three-quarter radius past the selection edge: several rings at
        // target density, well under the brush scale so the skirt accents
        // the dab instead of becoming it.
        let reach = 0.75 * radius;
        // Dijkstra frontier settles at pop time. A popped entry is current
        // only when the best map has no shorter path. Marking vertices at
        // push time makes discovery order affect distance, so shorter
        // rediscoveries re-enter the heap and stale pops are skipped.
        // BinaryHeap is a max-heap: the reversed key pops the
        // smallest distance first. (bits, id) is a total order — bit patterns
        // order like the non-negative floats they encode — so two runs agree
        // bit for bit. Each entry carries its distance and inherited
        // magnitude alongside the key.
        let mut frontier: BinaryHeap<std::cmp::Reverse<SkirtFrontier>> =
            BinaryHeap::with_capacity(weighted.len() * 2);
        let mut best: FxHashMap<u32, (f64, f64)> = FxHashMap::default();
        best.reserve(weighted.len() * 2);
        let mut slots: FxHashMap<u32, usize> = weighted
            .iter()
            .enumerate()
            .map(|(slot, &(group, _))| (group, slot))
            .collect();
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
            // Skip an entry when the best map contains a shorter path.
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
                // The footprint owns its assigned spine axis. A group beyond
                // it borrows the axis from the neighbor which reached it.
                if self.sheet_axis_mark[neighbor as usize] != self.sheet_axis_epoch {
                    self.sheet_axis[neighbor as usize] = self
                        .sheet_axis_of(group)
                        .to_array()
                        .map(|component| component as f32);
                    self.sheet_axis_mark[neighbor as usize] = self.sheet_axis_epoch;
                }
                let sheet = self.sheet_share(neighbor);
                if sheet > 0.0 {
                    if let Some(&slot) = slots.get(&neighbor) {
                        weighted[slot].1 = share * sheet;
                    } else {
                        slots.insert(neighbor, weighted.len());
                        weighted.push((neighbor, share * sheet));
                    }
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

    /// Add/Remove moves along each group's transported sheet axis. Taubin
    /// filters only that new displacement before the whole field is committed.
    // the clay field, its denoise and its commit are one operator.
    #[allow(clippy::too_many_lines)]
    pub(super) fn dab_clay(&mut self, dab: &Dab, region: &[SurfacePoint], facing: f64, sign: f64) {
        let strength = dab.strength.clamp(0.0, 1.0);
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| self.weight(point, dab));
        self.extend_preserve_skirt(&mut weighted, dab.radius);
        if !(self.dab_dose > 0.0) {
            self.weights = weighted;
            return;
        }
        // The densest point of this step bounds how much dose the denoise may
        // carry after averaging the stamp along travel.
        let dab_peak = weighted.iter().map(|&(_, w)| w).fold(0.0f64, f64::max);
        let knife = self.brush_tip == TipStamp::Knife;
        let hit_n = self
            .hit_triangle
            .and_then(|triangle| self.triangle_normal(triangle))
            .unwrap_or(DVec3::ZERO);
        // One call lays its share of 120 ms of brush time. Travel averaging is
        // in the per-group weights, so the same elapsed interval is applied
        // once across the swept path.
        let amplitude = layer_depth(dab.radius, strength) * self.dab_dose;
        if amplitude <= 0.0 {
            self.weights = weighted;
            return;
        }
        // Every vertex keeps the requested dose along the same depth axis;
        // per-vertex tearing limits and the simultaneous layer guard bound it.
        //
        // The knife's narrower support needs a higher centre gain to remain
        // distinct from Ball. The per-vertex clamp and simultaneous face guard
        // still bound every move.
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
        if erode {
            self.wall_facing = Some(facing);
        }
        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        let mut sheet_min = 1.0f64;
        let mut sheet_max = 0.0f64;
        let mut sheet_sum = 0.0f64;
        let mut sheet_full = 0.0f64;
        let mut spread = 0.0f64;
        for &(group, weight) in &weighted {
            let here = self.group_v(group);
            let push = self.sheet_push(group, facing);
            if push.length_squared() <= 1e-24 {
                continue;
            }
            let sheet = self.sheet_share(group);
            sheet_min = sheet_min.min(sheet);
            sheet_max = sheet_max.max(sheet);
            sheet_sum += sheet;
            if sheet >= 1.0 - 1e-12 {
                sheet_full += 1.0;
            }
            let signed = self.group_n(group) * facing;
            if signed.length() > 1e-12 {
                let aligned = signed
                    .normalize_or_zero()
                    .dot(push.normalize_or_zero())
                    .clamp(-1.0, 1.0);
                spread = spread.max(aligned.acos().to_degrees());
            }
            // Safety sees the complete displacement field after denoise.
            let target = here + (push * (sign * weight * amplitude * tip_gain));
            let clamped = if knife {
                let share = crate::clamp_dab_displacement(
                    &GroupKnot { session: &*self },
                    group,
                    1.0,
                    target - here,
                );
                self.clamp_step_at(group, here, here + share)
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
        let pointer_push = self.pointer_sheet_axis() * facing;
        self.live_kin.local_normal = true;
        self.live_kin.hit_sheet = self.hit_triangle.is_some();
        self.live_kin.nx = pointer_push.x as f32;
        self.live_kin.ny = pointer_push.y as f32;
        self.live_kin.nz = pointer_push.z as f32;
        self.live_kin.facing = facing as f32;
        self.live_kin.amplitude = amplitude as f32;
        self.live_kin.gain = 1.0;
        self.live_kin.weighted = weighted.len() as u32;
        self.live_kin.proposals = proposals.len() as u32;
        self.live_kin.front_min = if count > 0.0 { sheet_min as f32 } else { 0.0 };
        self.live_kin.front_max = sheet_max as f32;
        self.live_kin.front_mean = if count > 0.0 {
            (sheet_sum / count) as f32
        } else {
            0.0
        };
        self.live_kin.toward_frac = if count > 0.0 {
            (sheet_full / count) as f32
        } else {
            0.0
        };
        self.live_kin.cx = dab.center.x as f32;
        self.live_kin.cy = dab.center.y as f32;
        self.live_kin.cz = dab.center.z as f32;
        self.live_kin.vx = dab.view.x as f32;
        self.live_kin.vy = dab.view.y as f32;
        self.live_kin.vz = dab.view.z as f32;
        self.live_kin.hn_x = hit_n.x as f32;
        self.live_kin.hn_y = hit_n.y as f32;
        self.live_kin.hn_z = hit_n.z as f32;
        self.live_kin.target_mm = RemeshPolicy::standard()
            .target_for_radius(dab.radius)
            .unwrap_or(0.0) as f32;
        self.live_kin.flood_mm = dab.radius as f32;
        self.live_kin.n_spread_deg = self.live_kin.n_spread_deg.max(spread as f32);
        // Skirt proposals can lie outside the ray footprint. Add them to the
        // immutable dab snapshot before any tip-specific early return so
        // maintenance refreshes their normals and spatial rows as well.
        for &(group, _) in &proposals {
            if self.snapshot_stamp[group as usize] != self.snapshot_generation {
                let p = self.group_v(group);
                self.pre_pos[group as usize] = [p.x as f32, p.y as f32, p.z as f32];
                self.snapshot_stamp[group as usize] = self.snapshot_generation;
                self.dab_groups.push(group);
            }
        }
        // Radial post-smoothing widens the knife's narrow trail and can erase
        // its depth. Ball and cylinder retain clay denoise; the knife keeps
        // its travel-aligned analytic footprint.
        if self.brush_tip == TipStamp::Knife {
            if erode {
                for (group, target) in &mut proposals {
                    *target = self.guard_remove_wall(*group, self.group_v(*group), *target);
                }
            }
            self.commit_even_layer(&proposals, mode);
            self.proposals = proposals;
            self.weights = weighted;
            return;
        }
        // Filter the new layer across the plateau and taper at the rim.
        // The sheet law bounds both the lift and its denoise support.
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
            smoothstep(AUTOSMOOTH_RIM_TAPER, t) * self.sheet_share(point.group)
        });
        // Denoise the scalar dose along each group's local sheet axis.
        // Applying each pass after collecting its values keeps the field
        // independent of iteration order, and the live surface stays untouched
        // until the guarded commit. Untouched snapshot groups contribute zero
        // displacement; proposals overwrite their dose entries first.
        let mut amount = std::mem::take(&mut self.denoise_amount);
        for index in 0..self.dab_groups.len() {
            amount[self.dab_groups[index] as usize] = 0.0;
        }
        for &(group, target) in &proposals {
            amount[group as usize] =
                (target - self.pre_group(group)).dot(self.sheet_push(group, facing)) * sign;
        }
        // Denoise may redistribute this dab's dose, but cannot turn Add into
        // Remove or amplify it past its budget.
        let dose_cap = amplitude * tip_gain * dab_peak;
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
            let mut target =
                here + self.sheet_push(group, facing) * (sign * amount[group as usize]);
            if erode {
                target = self.guard_remove_wall(group, here, target);
            }
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

    /// Flatten (Minus): level the selection toward its own plane along the
    /// weighted sheet axis. No denoise tail: levelling is the finish.
    pub(super) fn dab_flatten(&mut self, dab: &Dab, region: &[SurfacePoint]) {
        let strength = dab.strength.clamp(0.0, 1.0);
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| self.weight(point, dab));
        self.extend_preserve_skirt(&mut weighted, dab.radius);
        let mut centroid = DVec3::ZERO;
        let mut total = 0.0f64;
        for &(group, weight) in &weighted {
            centroid += self.group_v(group) * weight;
            total += weight;
        }
        let normal = self.brush_normal(&weighted);
        if total <= 1e-12 || normal.length() <= 1e-12 {
            self.weights = weighted;
            return;
        }
        let point = centroid * (1.0 / total);
        // Time compounds as repeated partial travel toward the same plane.
        // The per-vertex clamp below bounds each call, and path-mean weights
        // distribute one interval over the swept segment.
        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        // Phase one reads only: the surface adapter borrows the session
        // shared, so budgets and walls (which need it mutable) wait for
        // phase two. One adapter serves both laws: the surface the
        // projector flattens is the surface the knot refuses to tear.
        let displacements: Vec<(u32, DVec3)> = {
            let surface = GroupKnot { session: &*self };
            let mut out = Vec::with_capacity(weighted.len());
            for &(group, weight) in &weighted {
                // Compound the stamp-weighted time share so partial calls
                // reach the same plane fraction as their total brush time.
                let share = if strength > 0.0 {
                    compounded_share(weight * self.dab_dose, strength) / strength
                } else {
                    0.0
                };
                let displacement =
                    crate::flatten_displacement(&surface, group, share, strength, point, normal);
                // Weight 1.0 — flatten already applied it, and held vertices
                // never reach this list.
                let displacement =
                    crate::clamp_dab_displacement(&surface, group, 1.0, displacement);
                out.push((group, displacement));
            }
            out
        };
        for (group, displacement) in displacements {
            let here = self.group_v(group);
            let target = self.clamp_step_at(group, here, here + displacement);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_skirt_keeps_one_weight_for_a_shorter_rediscovered_path() {
        let mut session = SculptSession::new(
            vec![-2.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0],
            vec![0, 1, 2],
        );
        session.set_preserve_skirt(true);
        let mut weighted = vec![(0, 1.0), (1, 1.0)];
        session.extend_preserve_skirt(&mut weighted, 4.0);
        assert_eq!(weighted.len(), 3, "a group must receive exactly one weight");
        assert_eq!(weighted[0], (0, 1.0));
        assert_eq!(weighted[1], (1, 1.0));
        assert_eq!(weighted[2].0, 2);
        assert!((weighted[2].1 - 20.0 / 27.0).abs() < 1e-12);
    }
}

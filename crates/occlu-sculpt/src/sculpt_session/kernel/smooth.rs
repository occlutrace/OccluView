//! Implicit shape smoothing. Live remesh separately preserves the resulting surface.

use super::*;

/// Solves one swept Smooth step may run. A full pass over a vertex lays about
/// four and a half dab spacings of stamp, so a full-strength brush needs five;
/// the cap bounds a call that crosses the same patch more than once.
const MAX_SWEPT_SMOOTH_PASSES: usize = 8;

/// Borrowed view of the live welded surface for the shared uniform operator:
/// one vertex per welded group, neighbours borrowed from the topology.
///
/// The mass matrix is the group's share of surface area, which the session
/// already maintains incrementally for the stochastic-normal weighting.
struct GroupSurface<'a> {
    session: &'a SculptSession,
}

impl crate::FairingSurface for GroupSurface<'_> {
    fn vertex_count(&self) -> usize {
        self.session.topology.group_count()
    }

    fn position(&self, vertex: u32) -> DVec3 {
        self.session.group_v(vertex)
    }

    fn vertex_area(&self, vertex: u32) -> f64 {
        self.session
            .group_area
            .get(vertex as usize)
            .copied()
            .unwrap_or(0.0) as f64
    }

    fn neighbors(&self, vertex: u32) -> &[u32] {
        self.session.topology.neighbors(vertex)
    }
}

impl SculptSession {
    // the smooth field and its commit are one operator.
    #[allow(clippy::too_many_lines)]
    pub(super) fn dab_smooth(&mut self, dab: &Dab, region: &[SurfacePoint], facing: f64) {
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| {
            self.weight(point, dab, facing)
        });
        let hit_n = self
            .hit_triangle
            .and_then(|t| self.triangle_normal(t))
            .unwrap_or(DVec3::ZERO);
        let strength = dab.strength.clamp(0.0, 1.0) * self.dab_exposure;
        self.live_kin.facing = facing as f32;
        self.live_kin.gain = strength as f32;
        self.live_kin.weighted = weighted.len() as u32;
        self.live_kin.hit_sheet = self.hit_triangle.is_some();
        self.live_kin.cx = dab.center.x as f32;
        self.live_kin.cy = dab.center.y as f32;
        self.live_kin.cz = dab.center.z as f32;
        self.live_kin.vx = dab.view.x as f32;
        self.live_kin.vy = dab.view.y as f32;
        self.live_kin.vz = dab.view.z as f32;
        self.live_kin.hn_x = hit_n.x as f32;
        self.live_kin.hn_y = hit_n.y as f32;
        self.live_kin.hn_z = hit_n.z as f32;
        if weighted.is_empty() {
            self.weights = weighted;
            return;
        }
        let swept = self.path_active();
        let feature_size_mm = crate::smoothing_scale_mm(dab.radius, dab.strength);
        if !(feature_size_mm > 0.0) {
            self.weights = weighted;
            return;
        }
        // A swept step stands for several dabs. Each full-strength share of
        // them solves again on the surface the previous share left, as
        // consecutive dabs did; a single solve moved a fast trail only part of
        // the way a dab trail went.
        let passes = if swept {
            let peak = weighted.iter().map(|&(_, w)| w).fold(0.0f64, f64::max);
            ((peak * strength).ceil() as usize).clamp(1, MAX_SWEPT_SMOOTH_PASSES)
        } else {
            1
        };
        let painted: Vec<u32> = if passes > 1 {
            weighted.iter().map(|&(group, _)| group).collect()
        } else {
            Vec::new()
        };
        let mut selection: Vec<(u32, f64)> = Vec::with_capacity(weighted.len() * 3);
        let mut targets: Vec<(u32, DVec3)> = Vec::new();
        let mut proposals = std::mem::take(&mut self.proposals);
        for pass in 0..passes {
            if pass > 0 {
                // The next solve and its normal projection read the surface
                // the previous pass left.
                let scope = self.collect_normal_scope(&painted);
                self.refresh_scope_normals(&scope);
                self.normal_scope = scope;
            }
            // Include a complete held one-ring so the brush rim does not
            // truncate free vertices' stencils. Open mesh boundaries remain
            // fixed. Each free row receives its painted falloff times this
            // dab's strength; the normal component of the solution changes
            // shape below, while live remesh owns redistribution along the
            // surface.
            selection.clear();
            {
                // Keep selection stamps separate from `group_stamp`, which
                // tracks the per-dab snapshot consumed by `pre_group` and
                // `moved_any`.
                let generation = self.next_selection_stamp();
                for &(group, weight) in &weighted {
                    self.selection_stamp[group as usize] = generation;
                    let share = weight / passes as f64;
                    // The per-dab clamp below reads how many dabs this
                    // vertex's share of the pass stands for.
                    self.denoise_amount[group as usize] = share.max(1.0);
                    let weight = if self.group_is_boundary(group) {
                        0.0
                    } else if swept {
                        // A swept step's weight counts dabs, each moving
                        // `strength` of the remaining way to the solved surface.
                        compounded_share(share, strength)
                    } else {
                        weight * strength
                    };
                    selection.push((group, weight));
                }
                // The held ring: every live neighbour of a painted vertex that
                // is not itself painted. Iterating over the ALREADY-PAINTED
                // prefix (the population is counted before this loop starts)
                // means the vertices just added as ring are not expanded again,
                // so exactly one ring is added rather than flooding the whole
                // sheet.
                let painted_rows = selection.len();
                for index in 0..painted_rows {
                    let group = selection[index].0;
                    for &neighbor in self.topology.neighbors(group) {
                        if neighbor as usize >= self.selection_stamp.len() {
                            continue;
                        }
                        if self.selection_stamp[neighbor as usize] == generation {
                            continue;
                        }
                        if !self.group_is_live(neighbor) {
                            continue;
                        }
                        self.selection_stamp[neighbor as usize] = generation;
                        selection.push((neighbor, 0.0));
                    }
                }
            }
            let mut scratch = std::mem::take(&mut self.fairing_scratch);
            crate::fair_selection(
                &GroupSurface { session: &*self },
                &selection,
                feature_size_mm,
                &mut scratch,
                &mut targets,
            );
            self.fairing_scratch = scratch;
            // The solved correction goes through the same guards a clay
            // proposal does: the per-vertex step clamp bounds one dab, and the
            // local layer guard rejects a field that would invert or crush a
            proposals.clear();
            for &(group, target) in &targets {
                let here = self.group_v(group);
                // Smooth changes shape. Tangential motion belongs to the
                // projected remesh stage; feeding umbrella drift into both
                // stages contracts irregular sampling and leaves constrained
                // corners as needles.
                let normal = self.group_n(group).normalize_or_zero();
                let target = here + (normal * (target - here).dot(normal));
                let dabs = if swept {
                    self.denoise_amount[group as usize]
                } else {
                    1.0
                };
                let target = self.clamp_step_scaled(group, here, target, dabs);
                if (target - here).length() > 1e-15 {
                    proposals.push((group, target));
                }
            }
            self.commit_even_layer(&proposals, BrushMode::Smooth);
        }
        self.proposals = proposals;
        self.weights = weighted;
    }
}

//! Implicit shape smoothing. Live remesh separately preserves the resulting surface.

use super::*;

/// Borrowed view of the live welded surface for the uniform operator: one
/// vertex per welded group, neighbours borrowed from the topology.
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
    pub(super) fn dab_smooth(&mut self, dab: &Dab, region: &[SurfacePoint]) {
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| self.weight(point, dab));
        let hit_n = self
            .hit_triangle
            .and_then(|triangle| self.triangle_normal(triangle))
            .unwrap_or(DVec3::ZERO);
        let rate = smooth_rate(dab.strength);
        self.live_kin.gain = (rate * self.dab_dose) as f32;
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
        let feature_size_mm = crate::smoothing_scale_mm(dab.radius, dab.strength);
        if weighted.is_empty() || !(feature_size_mm > 0.0) || !(self.dab_dose > 0.0) {
            self.weights = weighted;
            return;
        }

        let mut selection: Vec<(u32, f64)> = Vec::with_capacity(weighted.len() * 3);
        // Selection stamps are separate from `group_stamp`, the per-dab
        // snapshot ledger read by `pre_group` and `moved_any`.
        let generation = self.next_selection_stamp();
        for &(group, weight) in &weighted {
            self.selection_stamp[group as usize] = generation;
            let share = if self.group_is_boundary(group) {
                0.0
            } else {
                compounded_share(weight * self.dab_dose, rate)
            };
            selection.push((group, share));
        }
        // Hold one live ring around painted vertices so the brush rim does
        // not truncate free vertices' fairing stencils. Open boundaries stay
        // fixed; newly added ring vertices are not expanded again.
        let painted_rows = selection.len();
        for index in 0..painted_rows {
            let group = selection[index].0;
            for &neighbor in self.topology.neighbors(group) {
                if neighbor as usize >= self.selection_stamp.len()
                    || self.selection_stamp[neighbor as usize] == generation
                    || !self.group_is_live(neighbor)
                {
                    continue;
                }
                self.selection_stamp[neighbor as usize] = generation;
                selection.push((neighbor, 0.0));
            }
        }

        let mut targets: Vec<(u32, DVec3)> = Vec::new();
        let mut scratch = std::mem::take(&mut self.fairing_scratch);
        crate::fair_selection_preserving(
            &GroupSurface { session: &*self },
            &selection,
            feature_size_mm,
            &mut scratch,
            &mut targets,
        );
        self.fairing_scratch = scratch;

        // Smooth changes shape. Tangential motion belongs to live remesh;
        // the native common layer guard decides which field is safe to write.
        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        for &(group, target) in &targets {
            let here = self.group_v(group);
            let normal = self.group_n(group).normalize_or_zero();
            let target = here + normal * (target - here).dot(normal);
            if (target - here).length() > 1e-15 {
                proposals.push((group, target));
            }
        }
        self.commit_even_layer(&proposals, BrushMode::Smooth);
        self.proposals = proposals;
        self.weights = weighted;
    }
}

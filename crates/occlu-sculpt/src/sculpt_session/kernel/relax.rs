//! Gentle, shape-preserving local smoothing: Ctrl+Shift while adding or
//! removing material.

use super::*;
use crate::hash::FxHashMap;

/// Pass gain of the bilaplacian pair: pull by λ, then push by λ.
const RELAX_LAMBDA: f64 = 0.5;

struct RelaxPass<'a> {
    gain: f64,
    here: DVec3,
    first: &'a FxHashMap<u32, DVec3>,
    second: &'a FxHashMap<u32, DVec3>,
}

impl SculptSession {
    pub(super) fn dab_relax(&mut self, dab: &Dab, region: &[SurfacePoint]) {
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| self.weight(point, dab));
        let rate = smooth_rate(dab.strength);
        self.live_kin.gain = (rate * self.dab_dose) as f32;
        self.live_kin.weighted = weighted.len() as u32;
        self.live_kin.cx = dab.center.x as f32;
        self.live_kin.cy = dab.center.y as f32;
        self.live_kin.cz = dab.center.z as f32;
        if weighted.is_empty() || !(rate > 0.0) || !(self.dab_dose > 0.0) {
            self.weights = weighted;
            return;
        }

        let mut work: FxHashMap<u32, DVec3> = FxHashMap::default();
        let mut shares: Vec<(u32, f64, DVec3)> = Vec::with_capacity(weighted.len());
        for &(group, weight) in &weighted {
            if !self.group_is_live(group) || self.group_is_boundary(group) {
                continue;
            }
            let normal = self.group_n(group).normalize_or_zero();
            if normal.length_squared() <= 1e-24 {
                continue;
            }
            let share = compounded_share(weight * self.dab_dose, rate);
            if !(share > 0.0) {
                continue;
            }
            work.insert(group, self.group_v(group));
            shares.push((group, share, normal));
        }
        let no_positions: FxHashMap<u32, DVec3> = FxHashMap::default();
        let mut pulled: FxHashMap<u32, DVec3> = FxHashMap::default();
        for &(group, _, normal) in &shares {
            let start = work[&group];
            let pass = RelaxPass {
                gain: RELAX_LAMBDA,
                here: start,
                first: &work,
                second: &no_positions,
            };
            if let Some(next) = self.relax_pass(group, normal, pass) {
                pulled.insert(group, next);
            }
        }

        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        for &(group, share, normal) in &shares {
            let here = work[&group];
            let lifted = pulled.get(&group).copied().unwrap_or(here);
            let pass = RelaxPass {
                gain: -RELAX_LAMBDA,
                here: lifted,
                first: &pulled,
                second: &work,
            };
            let pushed = self.relax_pass(group, normal, pass).unwrap_or(lifted);
            let target = here + (pushed - here) * share;
            if (target - here).length() > 1e-15 {
                proposals.push((group, target));
            }
        }
        self.live_kin.proposals = proposals.len() as u32;
        self.commit_even_layer(&proposals, BrushMode::Relax);
        self.proposals = proposals;
        self.weights = weighted;
    }

    /// One normal-direction pass of `group` from `here` toward (positive
    /// `gain`) or away from its one-ring centroid. Each neighbour is read from
    /// `first`, then `second`, then the live surface.
    fn relax_pass(&self, group: u32, normal: DVec3, pass: RelaxPass<'_>) -> Option<DVec3> {
        let mut mean = DVec3::ZERO;
        let mut count = 0usize;
        for &neighbor in self.topology.neighbors(group) {
            if !self.group_is_live(neighbor) {
                continue;
            }
            let position = pass
                .first
                .get(&neighbor)
                .or_else(|| pass.second.get(&neighbor))
                .copied()
                .unwrap_or_else(|| self.group_v(neighbor));
            mean += position;
            count += 1;
        }
        if count < 2 {
            return None;
        }
        let along = (mean / count as f64 - pass.here).dot(normal);
        Some(pass.here + normal * (along * pass.gain))
    }
}

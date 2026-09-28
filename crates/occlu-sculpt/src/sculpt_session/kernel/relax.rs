//! Gentle surface filtering that preserves the broad shape.

use super::*;
use crate::hash::FxHashMap;

const RELAX_LAMBDA: f64 = 0.5;
const MAX_RELAX_PAIRS: usize = 8;

struct RelaxShare {
    group: u32,
    weight: f64,
    normal: DVec3,
}

#[derive(Clone, Copy)]
struct RelaxVertex {
    group: u32,
    normal: DVec3,
}

struct RelaxPositions<'a> {
    first: &'a FxHashMap<u32, DVec3>,
    second: &'a FxHashMap<u32, DVec3>,
}

impl SculptSession {
    pub(super) fn dab_relax(&mut self, dab: &Dab, region: &[SurfacePoint], facing: f64) {
        let mut weighted = std::mem::take(&mut self.weights);
        Self::weigh_region_into(region, &mut weighted, |point| {
            self.weight(point, dab, facing)
        });
        let strength = dab.strength.clamp(0.0, 1.0) * self.dab_exposure;
        self.live_kin.gain = strength as f32;
        self.live_kin.weighted = weighted.len() as u32;
        if weighted.is_empty() || !strength.is_finite() || strength <= 0.0 {
            self.weights = weighted;
            return;
        }

        let pairs = self.relax_pair_count(&weighted);
        let (shares, mut work) = self.relax_working_set(&weighted, strength, pairs);
        if shares.is_empty() {
            self.weights = weighted;
            return;
        }
        self.run_relax_pairs(&shares, &mut work, pairs);

        let mut proposals = std::mem::take(&mut self.proposals);
        proposals.clear();
        for share in &shares {
            let here = self.group_v(share.group);
            let target =
                self.clamp_step_scaled(share.group, here, work[&share.group], pairs as f64);
            if (target - here).length() > 1e-15 {
                proposals.push((share.group, target));
            }
        }
        self.live_kin.proposals = proposals.len() as u32;
        self.commit_even_layer(&proposals, BrushMode::Relax);
        self.proposals = proposals;
        self.weights = weighted;
    }

    fn relax_pair_count(&self, weighted: &[(u32, f64)]) -> usize {
        if !self.path_active() {
            return 1;
        }
        let peak = weighted
            .iter()
            .map(|&(_, weight)| weight)
            .fold(0.0, f64::max);
        (peak.ceil().clamp(1.0, MAX_RELAX_PAIRS as f64)) as usize
    }

    fn relax_working_set(
        &self,
        weighted: &[(u32, f64)],
        strength: f64,
        pairs: usize,
    ) -> (Vec<RelaxShare>, FxHashMap<u32, DVec3>) {
        let mut shares = Vec::with_capacity(weighted.len());
        let mut work = FxHashMap::default();
        for &(group, weight) in weighted {
            if !self.group_is_live(group) || self.group_is_boundary(group) {
                continue;
            }
            let normal = self.group_n(group).normalize_or_zero();
            let share = (weight / pairs as f64).min(1.0) * strength;
            if normal.length_squared() <= 1e-24 || share <= 0.0 {
                continue;
            }
            work.insert(group, self.group_v(group));
            shares.push(RelaxShare {
                group,
                weight: share,
                normal,
            });
        }
        (shares, work)
    }

    fn run_relax_pairs(
        &self,
        shares: &[RelaxShare],
        work: &mut FxHashMap<u32, DVec3>,
        pairs: usize,
    ) {
        let empty = FxHashMap::default();
        let mut pulled = FxHashMap::default();
        let mut pass = Vec::with_capacity(shares.len());
        for _ in 0..pairs {
            pulled.clear();
            for share in shares {
                let start = work[&share.group];
                if let Some(next) = self.relax_pass(
                    RelaxVertex {
                        group: share.group,
                        normal: share.normal,
                    },
                    RELAX_LAMBDA,
                    start,
                    RelaxPositions {
                        first: work,
                        second: &empty,
                    },
                ) {
                    pulled.insert(share.group, next);
                }
            }
            pass.clear();
            for share in shares {
                let start = work[&share.group];
                let lifted = pulled.get(&share.group).copied().unwrap_or(start);
                let next = self
                    .relax_pass(
                        RelaxVertex {
                            group: share.group,
                            normal: share.normal,
                        },
                        -RELAX_LAMBDA,
                        lifted,
                        RelaxPositions {
                            first: &pulled,
                            second: work,
                        },
                    )
                    .unwrap_or(lifted);
                pass.push((share.group, start + (next - start) * share.weight));
            }
            for &(group, position) in &pass {
                work.insert(group, position);
            }
        }
    }

    fn relax_pass(
        &self,
        vertex: RelaxVertex,
        gain: f64,
        here: DVec3,
        positions: RelaxPositions<'_>,
    ) -> Option<DVec3> {
        let mut mean = DVec3::ZERO;
        let mut count = 0usize;
        for &neighbor in self.topology.neighbors(vertex.group) {
            if !self.group_is_live(neighbor) {
                continue;
            }
            let position = positions
                .first
                .get(&neighbor)
                .or_else(|| positions.second.get(&neighbor))
                .copied()
                .unwrap_or_else(|| self.group_v(neighbor));
            mean += position;
            count += 1;
        }
        (count >= 2).then(|| {
            let offset = (mean / count as f64 - here).dot(vertex.normal);
            here + vertex.normal * (offset * gain)
        })
    }
}

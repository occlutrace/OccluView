//! One swept stroke step.
//!
//! A pointer call covers the short path from the previous step's end to the
//! pointer (at most `MAX_STEP_TRAVEL_SHARE` radii, see `stroke.rs`). The step
//! builds one footprint along that path, gives each vertex the tip stamp
//! integrated along it, counted in dabs of the tip's own spacing, then deforms
//! and remeshes once. The dose per millimetre is the one a trail of spaced
//! dabs gives, and every call ends with the brush under the pointer instead
//! of a backlog of dabs behind it.

use super::*;
use crate::RemeshPolicy;

/// Dab budgets one swept step may spend on its remesh. The travel bound
/// already keeps a step within about fourteen ball spacings; this caps the
/// denser knife trail at the same order.
const MAX_STEP_DAB_BUDGETS: f64 = 16.0;

/// The share of the way to its target a vertex has travelled after `dabs`
/// dab-equivalents, each moving `strength` of the remaining way. One full dab
/// gives exactly `strength`; a fractional weight gives the matching fraction
/// of a dab. A full-strength brush keeps its falloff up to one dab.
pub(super) fn compounded_share(dabs: f64, strength: f64) -> f64 {
    if !(dabs > 0.0) || !(strength > 0.0) || !dabs.is_finite() {
        return 0.0;
    }
    if strength >= 1.0 {
        return dabs.min(1.0);
    }
    (1.0 - (1.0 - strength).powf(dabs)).clamp(0.0, 1.0)
}

fn segment_distance(point: DVec3, a: DVec3, b: DVec3) -> f64 {
    let segment = b - a;
    let length_squared = segment.dot(segment);
    let t = if length_squared > 1e-24 {
        ((point - a).dot(segment) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point - (a + segment * t)).length()
}

impl SculptSession {
    pub(super) fn path_active(&self) -> bool {
        self.dab_path.len() >= 2
    }

    /// Distance of `position` from the footprint's spine: the swept path, or
    /// the dab centre for a press or a hold.
    pub(super) fn support_distance(&self, position: DVec3, center: DVec3) -> f64 {
        if self.path_active() {
            self.path_distance(position)
        } else {
            (position - center).length()
        }
    }

    /// The remesh policy of the current step. A swept step stands for the
    /// dabs its path would have laid, and takes their operation, candidate
    /// and journal budgets, so one pass does the work their cycles did.
    pub(super) fn step_remesh_policy(&self) -> RemeshPolicy {
        let mut policy = RemeshPolicy::standard();
        let dabs = self.step_dabs();
        policy.max_operations_per_dab *= dabs;
        policy.max_candidates_per_dab *= dabs;
        policy.max_journal_bytes_per_dab *= dabs;
        policy
    }

    /// How many dabs the current step stands for: its path in dab spacings,
    /// at least one, at most the step's budget cap. A press or a hold is one.
    pub(super) fn step_dabs(&self) -> usize {
        if !self.path_active() || !(self.dab_path_spacing > 0.0) {
            return 1;
        }
        (self.path_length() / self.dab_path_spacing)
            .ceil()
            .clamp(1.0, MAX_STEP_DAB_BUDGETS) as usize
    }

    pub(in crate::sculpt_session) fn path_length(&self) -> f64 {
        self.dab_path
            .windows(2)
            .map(|pair| (pair[1].point - pair[0].point).length())
            .sum()
    }

    /// Shortest distance from `point` to the swept path.
    pub(super) fn path_distance(&self, point: DVec3) -> f64 {
        self.dab_path
            .windows(2)
            .map(|pair| segment_distance(point, pair[0].point, pair[1].point))
            .fold(f64::INFINITY, f64::min)
    }

    /// The tip stamp integrated along the swept path at `point`, in dabs.
    pub(super) fn path_stamp_weight(&self, point: DVec3, radius: f64) -> f64 {
        if !(self.dab_path_spacing > 0.0) {
            return 0.0;
        }
        let mut travel = 0.0;
        for pair in self.dab_path.windows(2) {
            travel += crate::segment_stamp_weight(
                self.brush_tip,
                point - pair[0].point,
                pair[1].point - pair[0].point,
                radius,
            );
        }
        travel / self.dab_path_spacing
    }

    /// Every group within `radius` of the swept path, each once. A point that
    /// close to a segment is within `radius` plus half the segment of one of
    /// its ends, so one grid query per sample covers the whole footprint.
    pub(super) fn path_candidates(&mut self, radius: f64, out: &mut Vec<u32>) {
        out.clear();
        let longest = self
            .dab_path
            .windows(2)
            .map(|pair| (pair[1].point - pair[0].point).length())
            .fold(0.0f64, f64::max);
        let reach = radius + longest * 0.5;
        let generation = self.next_stamp();
        let mut found = std::mem::take(&mut self.path_scratch);
        for index in 0..self.dab_path.len() {
            let point = self.dab_path[index].point;
            self.brush_grid.query_radius(point, reach, &mut found);
            for &group in &found {
                if self.group_stamp[group as usize] != generation {
                    self.group_stamp[group as usize] = generation;
                    out.push(group);
                }
            }
        }
        self.path_scratch = found;
    }
}

//! One swept stroke step and the dose laws shared by each brush.
//!
//! A pointer call covers the short path from the previous step's end to the
//! pointer (at most `step_reach`, see `stroke.rs`). The step builds one
//! footprint along that path, deforms and remeshes once, and ends with the
//! brush under the pointer instead of a backlog of dabs behind it.
//!
//! Brush dose is elapsed time, capped to one full interval by the caller.
//! Each vertex takes the tip stamp averaged over the path, then receives its
//! share of that call's dose. A still hand concentrates the dose under the
//! pointer; a fast pass spreads it along the path.

use super::*;
use crate::RemeshPolicy;

/// Dab budgets one swept step may spend on its remesh. The travel bound keeps
/// one step within four radii; this caps the denser knife trail's remesh work
/// at sixteen dab-equivalents.
const MAX_STEP_DAB_BUDGETS: f64 = 16.0;

/// Per-dab share of Smooth and Relax at the requested strength.
pub(super) fn smooth_rate(strength: f64) -> f64 {
    (strength.clamp(0.0, 1.0) * 0.55).min(1.0)
}

/// Add/Remove material laid at the brush centre by one full-strength dab.
pub(super) fn layer_depth(radius: f64, strength: f64) -> f64 {
    (radius * 0.12 * strength.clamp(0.0, 1.0)).max(0.0)
}

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

    /// The tip stamp averaged over the swept path at `point`.
    pub(super) fn path_mean_stamp(&self, point: DVec3, radius: f64) -> f64 {
        let mut stamped = 0.0;
        let mut length = 0.0;
        for pair in self.dab_path.windows(2) {
            let segment = pair[1].point - pair[0].point;
            stamped +=
                crate::segment_stamp_weight(self.brush_tip, point - pair[0].point, segment, radius);
            length += segment.length();
        }
        if length > 1e-12 {
            (stamped / length).clamp(0.0, 1.0)
        } else {
            0.0
        }
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

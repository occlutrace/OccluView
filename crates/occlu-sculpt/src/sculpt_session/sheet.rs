//! Camera-independent sheet axes for brush weights and displacement.
//!
//! The pointer ray selects a point and which side the operator works from.
//! The local surface, captured at first touch in a stroke, controls every
//! brush's sheet support. Add and Remove use the smoothly transported axis
//! along the step spine instead of camera depth.

use super::*;

const SHEET_FULL_COS: f64 = 0.0;
const SHEET_REVERSE_COS: f64 = -0.5;

/// One point of a step's path spine and its local sheet axis.
#[derive(Clone, Copy)]
pub(super) struct SpineSample {
    point: DVec3,
    axis: DVec3,
}

impl SculptSession {
    /// Assign stable local axes to this step's footprint.
    pub(super) fn assign_sheet_axes(&mut self, dab: &Dab, region: &[SurfacePoint]) {
        self.sheet_axis_epoch = self.sheet_axis_epoch.wrapping_add(1);
        if self.sheet_axis_epoch == 0 {
            self.sheet_axis_mark.fill(u32::MAX);
            self.sheet_axis_epoch = 1;
        }

        let mut spine = std::mem::take(&mut self.spine);
        spine.clear();
        if self.dab_path.len() >= 2 {
            for sample in &self.dab_path {
                spine.push(SpineSample {
                    point: sample.point,
                    axis: self
                        .shading_normal(sample.triangle, sample.point)
                        .unwrap_or(DVec3::ZERO),
                });
            }
        } else if let Some(triangle) = self.hit_triangle {
            spine.push(SpineSample {
                point: dab.center,
                axis: self
                    .shading_normal(triangle, dab.center)
                    .unwrap_or(DVec3::ZERO),
            });
        }

        let mut votes = std::mem::take(&mut self.sheet_votes);
        votes.clear();
        for point in region {
            let group = point.group;
            let normal = self.stroke_start_normal(group);
            let area = self.group_area[group as usize] as f64;
            votes.push((self.group_v(group), normal, area));
        }

        for sample in &mut spine {
            let pointer = sample.axis.normalize_or_zero();
            let mut sum = DVec3::ZERO;
            for &(position, normal, area) in &votes {
                if pointer.length_squared() > 0.25 && normal.dot(pointer) <= 0.0 {
                    continue;
                }
                let share = crate::ball_weight((position - sample.point).length(), dab.radius);
                if share > 0.0 && area.is_finite() && area > 0.0 {
                    sum += normal * (share * area);
                }
            }
            let axis = sum.normalize_or_zero();
            if axis.length_squared() > 0.25 {
                sample.axis = axis;
            } else {
                sample.axis = pointer;
            }
        }

        let generation = self.sheet_axis_epoch;
        for (point, &(position, _, _)) in region.iter().zip(&votes) {
            let axis = spine_axis_at(&spine, position);
            let slot = point.group as usize;
            self.sheet_axis[slot] = axis.to_array().map(|component| component as f32);
            self.sheet_axis_mark[slot] = generation;
        }
        self.sheet_votes = votes;
        self.spine = spine;
    }

    /// The shading normal when this stroke first reached `group`.
    fn stroke_start_normal(&mut self, group: u32) -> DVec3 {
        let index = group as usize;
        if self.stroke_normal_mark[index] == self.stroke_epoch {
            return DVec3::from_array(self.stroke_normal[index].map(f64::from));
        }
        let normal = self.group_n(group).normalize_or_zero();
        self.stroke_normal[index] = normal.to_array().map(|component| component as f32);
        self.stroke_normal_mark[index] = self.stroke_epoch;
        normal
    }

    /// A remesh split inherits its parents' stroke-start normal.
    pub(super) fn inherit_stroke_normal(&mut self, group: u32, a: u32, b: u32) {
        let epoch = self.stroke_epoch;
        if self.stroke_normal_mark[a as usize] != epoch
            || self.stroke_normal_mark[b as usize] != epoch
        {
            return;
        }
        let read =
            |index: u32| DVec3::from_array(self.stroke_normal[index as usize].map(f64::from));
        let normal = (read(a) + read(b)).normalize_or_zero();
        if normal.length_squared() > 0.25 {
            self.stroke_normal[group as usize] =
                normal.to_array().map(|component| component as f32);
            self.stroke_normal_mark[group as usize] = epoch;
        }
    }

    /// Share of the brush admitted on this sheet: full through a quarter turn
    /// and smoothly zero at the reverse-sheet fold.
    pub(super) fn sheet_share(&self, group: u32) -> f64 {
        let normal = self.group_n(group);
        let axis = self.sheet_axis_of(group);
        let lengths = normal.length() * axis.length();
        if !(lengths > 1e-12) || !lengths.is_finite() {
            return 0.0;
        }
        let cosine = normal.dot(axis) / lengths;
        smoothstep(
            1.0,
            (cosine - SHEET_REVERSE_COS) / (SHEET_FULL_COS - SHEET_REVERSE_COS),
        )
    }

    /// Push direction at a footprint group, turned toward the clicked side.
    pub(super) fn sheet_push(&self, group: u32, facing: f64) -> DVec3 {
        self.sheet_axis_of(group) * facing
    }

    /// Axis under the pointer for live-dab diagnostics.
    pub(super) fn pointer_sheet_axis(&self) -> DVec3 {
        self.spine.last().map_or(DVec3::ZERO, |sample| sample.axis)
    }

    /// Invalidate geometry-derived state when a stroke or history operation
    /// closes the step which produced it.
    pub(super) fn invalidate_sheet_state(&mut self, invalidate_stroke_normals: bool) {
        self.sheet_axis_epoch = self.sheet_axis_epoch.wrapping_add(1);
        if self.sheet_axis_epoch == 0 {
            self.sheet_axis_mark.fill(u32::MAX);
            self.sheet_axis_epoch = 1;
        }
        self.spine.clear();
        self.sheet_votes.clear();
        if invalidate_stroke_normals {
            self.stroke_normal_mark.fill(u32::MAX);
        }
    }

    pub(super) fn sheet_axis_of(&self, group: u32) -> DVec3 {
        let slot = group as usize;
        if self.sheet_axis_mark.get(slot) == Some(&self.sheet_axis_epoch) {
            DVec3::from_array(self.sheet_axis[slot].map(f64::from))
        } else {
            let transported = spine_axis_at(&self.spine, self.group_v(group));
            if transported.length_squared() > 1e-24 {
                transported
            } else {
                self.group_n(group)
            }
        }
    }

    fn shading_normal(&self, triangle: u32, point: DVec3) -> Option<DVec3> {
        let [a, b, c] = self.topology.triangle(triangle)?;
        let (pa, pb, pc) = (self.group_v(a), self.group_v(b), self.group_v(c));
        let face = (pb - pa).cross(pc - pa);
        let area = face.length_squared();
        if !(area > 1e-24) {
            return None;
        }
        let face_normal = face / area.sqrt();
        let wa = (pc - pb).cross(point - pb).dot(face) / area;
        let wb = (pa - pc).cross(point - pc).dot(face) / area;
        let wc = 1.0 - wa - wb;
        let smooth = (self.group_n(a) * wa + self.group_n(b) * wb + self.group_n(c) * wc)
            .normalize_or_zero();
        Some(
            if smooth.length_squared() > 0.25 && smooth.dot(face_normal) > 0.2 {
                smooth
            } else {
                face_normal
            },
        )
    }
}

fn spine_axis_at(spine: &[SpineSample], position: DVec3) -> DVec3 {
    match spine {
        [] => DVec3::ZERO,
        [only] => only.axis,
        _ => {
            let mut best = (f64::INFINITY, spine[0].axis);
            for pair in spine.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let segment = b.point - a.point;
                let length_squared = segment.length_squared();
                let t = if length_squared > 1e-24 {
                    ((position - a.point).dot(segment) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = (position - (a.point + segment * t)).length();
                if distance < best.0 {
                    let blend = (a.axis * (1.0 - t) + b.axis * t).normalize_or_zero();
                    let axis = if blend.length_squared() > 0.25 {
                        blend
                    } else if t < 0.5 {
                        a.axis
                    } else {
                        b.axis
                    };
                    best = (distance, axis);
                }
            }
            best.1
        }
    }
}

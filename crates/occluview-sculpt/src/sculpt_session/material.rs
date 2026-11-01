//! Material coordinates: the point of the opening surface each live vertex
//! stands for.
//!
//! The wall reserve and session-baseline guards measure vertices against this
//! point. When remeshing slides a vertex along the surface without adding or
//! removing material, the material point follows it. Otherwise tangential
//! movement consumes wall reserve and can block later inward strokes.

use super::kernel::topology_journal::{MaterialEdit, TopoJournal};
use super::*;

/// Barycentric weights of `point` projected into the plane of `triangle`, or
/// `None` for a degenerate triangle. Components may be negative outside it.
fn plane_barycentric(point: DVec3, triangle: [DVec3; 3]) -> Option<[f64; 3]> {
    let (v0, v1, v2) = (
        (triangle[1] - triangle[0]),
        (triangle[2] - triangle[0]),
        (point - triangle[0]),
    );
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let (d20, d21) = (v2.dot(v0), v2.dot(v1));
    let denominator = d00 * d11 - d01 * d01;
    if !denominator.is_finite() || denominator.abs() <= 1e-24 {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denominator;
    let w = (d00 * d21 - d01 * d20) / denominator;
    let weights = [1.0 - v - w, v, w];
    weights
        .iter()
        .all(|weight| weight.is_finite())
        .then_some(weights)
}

impl SculptSession {
    /// The material point under `target` when `group` moves there across its
    /// current star: the live barycentric weights of `target` in the incident
    /// face that holds it, applied to that face's material corners. This is the
    /// piecewise-linear correspondence between the live and the opening
    /// surface. The relaxation step clamp keeps a move inside the star; a
    /// target just outside it takes the clamped weights of the nearest face.
    pub(super) fn star_material(&self, group: u32, target: DVec3) -> Option<DVec3> {
        let mut best: Option<(f64, [u32; 3], [f64; 3])> = None;
        for &triangle in self.topology.incident_triangles(group) {
            let Some(corners) = self.topology.triangle(triangle) else {
                continue;
            };
            let Some(weights) = plane_barycentric(target, corners.map(|g| self.group_v(g))) else {
                continue;
            };
            let inside = weights[0].min(weights[1]).min(weights[2]);
            if best.is_none_or(|(score, _, _)| inside > score) {
                best = Some((inside, corners, weights));
            }
        }
        let (_, corners, weights) = best?;
        let clamped = weights.map(|weight| weight.max(0.0));
        let total = clamped[0] + clamped[1] + clamped[2];
        if !(total > 1e-12) {
            return None;
        }
        let mut material = DVec3::ZERO;
        for (corner, weight) in corners.into_iter().zip(clamped) {
            material += self.reference_group_v(corner) * (weight / total);
        }
        material.is_finite().then_some(material)
    }

    /// Move every member of `group` to `material`, journaling each member's
    /// value before its first move in this stroke so undo, redo and abort
    /// return the material point together with the position.
    pub(super) fn set_group_material(
        &mut self,
        journal: &mut TopoJournal,
        group: u32,
        material: DVec3,
    ) {
        let stored = [material.x as f32, material.y as f32, material.z as f32];
        for index in 0..self.topology.members(group).len() {
            let vertex = self.topology.members(group)[index];
            let offset = vertex as usize * 3;
            let current = [
                self.reference_verts[offset],
                self.reference_verts[offset + 1],
                self.reference_verts[offset + 2],
            ];
            if current == stored {
                continue;
            }
            if self.material_mark[vertex as usize] != self.stroke_epoch {
                self.material_mark[vertex as usize] = self.stroke_epoch;
                journal.push_material(MaterialEdit {
                    vertex,
                    before: current,
                    after: current,
                });
            }
            self.reference_verts[offset..offset + 3].copy_from_slice(&stored);
        }
    }

    /// Give every material edit of the closing stroke its final value.
    pub(super) fn seal_material_edits(&self, journal: &mut TopoJournal) {
        let live = self.verts.len() / 3;
        for edit in &mut journal.material {
            let vertex = edit.vertex as usize;
            if vertex < live {
                edit.after = [
                    self.reference_verts[vertex * 3],
                    self.reference_verts[vertex * 3 + 1],
                    self.reference_verts[vertex * 3 + 2],
                ];
            }
        }
    }

    /// Write a history step's material points back: the values before the
    /// stroke for undo, after it for redo. Vertices the topology replay has
    /// already removed are skipped. A group whose material point moved takes a
    /// fresh wall reading at the restored point the next time a guard asks.
    pub(super) fn restore_material(&mut self, journal: &TopoJournal, redo: bool) {
        let live = self.verts.len() / 3;
        for edit in &journal.material {
            let vertex = edit.vertex as usize;
            if vertex >= live {
                continue;
            }
            let value = if redo { edit.after } else { edit.before };
            self.reference_verts[vertex * 3..vertex * 3 + 3].copy_from_slice(&value);
        }
        // A restored material point invalidates every wall reading: the memo is
        // keyed on where the point stood when it was measured. Clearing the
        // whole table costs one pass and cannot leave a stale reserve behind.
        self.reference_wall_mm.fill(f32::NAN);
    }
}

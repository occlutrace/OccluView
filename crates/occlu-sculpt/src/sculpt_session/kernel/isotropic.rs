//! Live isotropic remeshing of the shape produced by the current dab.
//! Every proposed relocation lands on the surface the edit replaces and is
//! checked against that local surface before commit. Release records history
//! only; it cannot start another geometry operation.

use super::*;
use crate::RemeshPolicy;

const LIVE_RESPACE_PASSES: usize = 2;
/// Relaxation passes one swept step may run: two per dab it stands for, as a
/// dab trail relaxed, up to this ceiling. The settle exit ends most early.
const MAX_LIVE_RESPACE_PASSES: usize = 8;
const LIVE_RESPACE_SETTLED_SHARE: f64 = 0.02;
/// A vertex whose relaxation pull is under this share of the target spacing
/// is already in place and skips the landing, cover and material work. A
/// swept step overlaps the previous one by most of its footprint, so without
/// it every vertex under the brush was re-landed a score of times per pass of
/// the brush for a correction measured in microns.
const LIVE_RESPACE_VERTEX_SETTLED_SHARE: f64 = 0.05;

impl SculptSession {
    pub(super) fn live_topology_open(&self) -> bool {
        self.remesh_armed && self.topo_budget_open && !self.region_points.is_empty()
    }

    pub(super) fn isotropic_cycle(&mut self, dab: &Dab, policy: &RemeshPolicy, target: f64) {
        if !self.live_topology_open() || !(target > 0.0) {
            return;
        }
        let tolerance = Self::remesh_tolerance_mm(target);
        let share = policy.max_operations_per_dab / 3;
        let mut journal = std::mem::take(&mut self.topo_journal);
        self.begin_op_stage(share);
        self.densify_footprint_inner(dab, policy, &mut journal);
        let append_base = self.live_tris;
        let region = std::mem::take(&mut self.region_points);
        self.begin_op_stage(share);
        self.collapse_footprint_inner(
            &region,
            policy,
            target,
            append_base,
            tolerance,
            &mut journal,
        );
        self.begin_op_stage(
            policy
                .max_operations_per_dab
                .saturating_sub(self.dab_topo_ops),
        );
        self.flip_footprint_inner(&region, policy, target, tolerance, &mut journal);
        self.end_op_stage();
        self.region_points = region;
        self.refresh_rewired_normals();
        self.respace_dab(target, tolerance, &mut journal);
        self.topo_journal = journal;
        let bytes = self.topo_journal.encoded_size_words() as u64 * 4;
        self.stroke_topo_bytes = bytes;
        self.topo_budget_open = bytes < policy.max_journal_bytes_per_stroke as u64;
    }

    /// Merges and flips rewire faces without touching positions, and their
    /// corners' normals are refreshed only by the post-dab upkeep. The
    /// relaxation slides along those normals, so it reads current ones.
    fn refresh_rewired_normals(&mut self) {
        if self.topo_touched.is_empty() {
            return;
        }
        let touched = std::mem::take(&mut self.topo_touched);
        let scope = self.collect_normal_scope(&touched);
        self.refresh_scope_normals(&scope);
        self.normal_scope = scope;
        self.topo_touched = touched;
    }

    // the tangential settle loop is one set of fixed-point passes.
    #[allow(clippy::too_many_lines)]
    fn respace_dab(&mut self, target_mm: f64, tolerance: f64, journal: &mut TopoJournal) {
        if self.region_points.is_empty() {
            return;
        }
        // Footprint membership by stamp: one flag per group, no hashing.
        let inside = self.next_selection_stamp();
        let mut groups: Vec<u32> = Vec::with_capacity(self.region_points.len());
        for index in 0..self.region_points.len() {
            let group = self.region_points[index].group;
            if self.selection_stamp[group as usize] != inside {
                self.selection_stamp[group as usize] = inside;
                groups.push(group);
            }
        }
        groups.sort_unstable();
        let settled_scale = if target_mm.is_finite() && target_mm > 0.0 {
            target_mm * LIVE_RESPACE_SETTLED_SHARE
        } else {
            0.0
        };
        let vertex_settled = if target_mm.is_finite() && target_mm > 0.0 {
            target_mm * LIVE_RESPACE_VERTEX_SETTLED_SHARE
        } else {
            0.0
        };
        let passes = (LIVE_RESPACE_PASSES * self.step_dabs()).min(MAX_LIVE_RESPACE_PASSES);
        for _pass in 0..passes {
            let mut steps: Vec<(u32, DVec3)> = Vec::with_capacity(groups.len());
            {
                let surface = LiveSpacing { session: &*self };
                for &group in &groups {
                    // The topology's own domain, verbatim: the same predicate
                    // the collapse and flip use. A vertex whose triangles this
                    // cycle may rewrite must also be a vertex whose position it
                    // may correct, or the band is rebuilt around the same
                    // uneven spacing on every pass and looks frozen.
                    if !self.group_is_editable(group) {
                        continue;
                    }
                    if self.group_is_boundary(group) {
                        continue;
                    }
                    let neighbors = self.topology.neighbors(group);
                    if neighbors.len() < 2
                        || neighbors
                            .iter()
                            .any(|&n| self.selection_stamp[n as usize] != inside)
                    {
                        continue;
                    }
                    let Some(target) =
                        crate::tangential_respace_target(&surface, group, crate::RESPACE_GAIN, 1.0)
                    else {
                        continue;
                    };
                    if (target - self.group_v(group)).length() <= vertex_settled {
                        continue;
                    }
                    steps.push((group, target));
                }
            }
            let mut largest_pull = 0.0f64;
            let mut pass_moved = 0usize;
            for (group, target) in steps {
                let here = self.group_v(group);
                let target = self.clamp_step_at(group, here, target);
                // Land on the surface the vertex slides across: its own star
                // as it stands before the move.
                let Some(star) = self.local_surface(&[group]) else {
                    continue;
                };
                let Some((landing, off_surface)) = star.nearest(target) else {
                    continue;
                };
                if off_surface > tolerance {
                    continue;
                }
                let target = stored_position(landing);
                let step = (target - here).length();
                if step <= 1e-12 {
                    continue;
                }
                // A slide must preserve every incident live face, and every
                // face must stay on the star it replaces. A new face differs
                // from its old one in the moved corner only, so each point the
                // cover test samples lies within half the step of the same
                // point on the old face, which is on the star: a step inside
                // the tolerance cannot leave it, and needs no test.
                let covered = step <= tolerance;
                let mut safe = true;
                for &tri in self.topology.incident_triangles(group) {
                    let Some(triple) = self.topology.triangle(tri) else {
                        continue;
                    };
                    let mut candidate = [DVec3::ZERO; 3];
                    let mut baseline = [DVec3::ZERO; 3];
                    for (i, &corner) in triple.iter().enumerate() {
                        candidate[i] = if corner == group {
                            target
                        } else {
                            self.group_v(corner)
                        };
                        baseline[i] = self.group_v(corner);
                    }
                    if !Self::triangle_final_is_safe(baseline, candidate)
                        || (!covered && !star.covers(candidate, tolerance, Some(tri)))
                    {
                        safe = false;
                        break;
                    }
                }
                if !safe {
                    continue;
                }
                // Read before the move: the correspondence is the star the
                // vertex slides across.
                let material = self.star_material(group, target);
                self.write_group_position(group, target);
                if let Some(material) = material {
                    self.set_group_material(journal, group, material);
                }
                pass_moved += 1;
                largest_pull = largest_pull.max(step);
            }
            if pass_moved == 0 || largest_pull <= settled_scale {
                break;
            }
            let touched = self.collect_normal_scope(&groups);
            self.refresh_scope_normals(&touched);
            self.normal_scope = touched;
        }
    }
}

/// Borrowed view of the live welded surface for the shared spacing operator.
struct LiveSpacing<'a> {
    session: &'a SculptSession,
}

impl crate::RespaceSurface for LiveSpacing<'_> {
    fn respace_position(&self, vertex: u32) -> DVec3 {
        self.session.group_v(vertex)
    }

    fn respace_normal(&self, vertex: u32) -> DVec3 {
        self.session.group_n(vertex)
    }

    fn respace_neighbors(&self, vertex: u32) -> &[u32] {
        self.session.topology.neighbors(vertex)
    }
}

//! Per-dab bookkeeping for [`super::BrushSession`], split out of `brush.rs` to
//! hold the workspace's 800-line file budget: the reusable generation stamps,
//! the spatial-index upkeep, the connected-component restriction, the post-dab
//! inversion rollback, and the localized normal recompute.
//!
//! These are the parts of a dab that touch session STATE rather than shape.
//! They live in a child module (not a sibling) so they can keep reading
//! `BrushSession`'s private fields directly, and are `pub(super)` so both
//! `brush.rs` and the densification module can drive them.

use glam::Vec3;
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

use super::{
    cancellation_requested, BrushSession, GRID_CELLS_ACROSS_RADIUS, MAX_LOCAL_ROLLBACK_ITERS,
    MAX_ROLLBACK_ITERS,
};
use crate::brush_index::VertexGrid;
use crate::brush_math::{on_flipped_triangle, scope_area_normals};

impl BrushSession {
    /// Keep only candidates in the same connected component as the vertex
    /// nearest the dab center, by flooding welded rings (and soup siblings)
    /// through the in-disc set. A Euclidean radius query can pull in a
    /// spatially-close but topologically SEPARATE surface (a dropout island,
    /// the opposing arch behind the cursor); this stops a dab from dragging
    /// two disjoint sheets together, for Add/Remove as well as Smooth.
    pub(super) fn restrict_to_component(
        &mut self,
        weighted: Vec<(usize, f32)>,
        center: Vec3,
        cancel: Option<&AtomicBool>,
    ) -> Option<Vec<(usize, f32)>> {
        if weighted.len() <= 1 {
            return Some(weighted);
        }
        if cancellation_requested(cancel) {
            return None;
        }
        let Some(seed) = weighted
            .iter()
            .min_by(|a, b| {
                let da = self.position(a.0).distance(center);
                let db = self.position(b.0).distance(center);
                da.total_cmp(&db)
            })
            .map(|&(id, _)| id)
        else {
            return Some(weighted);
        };
        // Two generations off one reusable stamp buffer, no per-dab allocation:
        // `in_disc` marks the candidate set, `reached` marks the flood fill.
        let in_disc = self.next_stamp();
        for &(id, _) in &weighted {
            self.component_stamp[id] = in_disc;
        }
        let reached = self.next_stamp();
        let mut stack = vec![seed];
        self.component_stamp[seed] = reached;
        // The CSR rows borrow `self.adjacency`/`self.position_siblings`, disjoint
        // from the `self.component_stamp` we stamp, so a plain iterator is fine.
        while let Some(vertex_id) = stack.pop() {
            if cancellation_requested(cancel) {
                return None;
            }
            for &neighbor in self.adjacency.row(vertex_id) {
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] == in_disc {
                    self.component_stamp[neighbor] = reached;
                    stack.push(neighbor);
                }
            }
            for &neighbor in self.position_siblings.row(vertex_id) {
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] == in_disc {
                    self.component_stamp[neighbor] = reached;
                    stack.push(neighbor);
                }
            }
        }
        Some(
            weighted
                .into_iter()
                .filter(|&(id, _)| self.component_stamp[id] == reached)
                .collect(),
        )
    }

    /// Hand out the next stamp generation, resetting the stamp buffer on the
    /// rare `u32` wrap so a stale stamp can never masquerade as the current one.
    pub(super) fn next_stamp(&mut self) -> u32 {
        self.stamp_generation = self.stamp_generation.wrapping_add(1);
        if self.stamp_generation == 0 {
            self.component_stamp.iter_mut().for_each(|s| *s = 0);
            self.stamp_generation = 1;
        }
        self.stamp_generation
    }

    /// Keep the spatial grid usable for a dab of `radius`: rebuild it (from
    /// live positions, cell size matched to radius) only when the brush radius
    /// changed enough to make the old cell size too coarse or fine — sized to
    /// radius so a big brush never scans millions of empty cells.
    pub(super) fn sync_grid(&mut self, radius: f32, cancel: Option<&AtomicBool>) -> bool {
        if cancellation_requested(cancel) {
            return false;
        }
        // ONLY a brush-radius change (which changes cell size) forces a
        // rebuild — a rare, deliberate size-slider move. Motion during a
        // stroke is tracked incrementally, not by a per-dab O(n) rebuild
        // (the stall a big scan showed).
        if self.grid_radius <= 0.0 {
            self.grid_radius = radius;
            return true;
        }
        if self.grid_radius > 0.0 && (0.6..=1.7).contains(&(radius / self.grid_radius)) {
            return true;
        }
        let desired_cell = (radius / GRID_CELLS_ACROSS_RADIUS).max(f32::MIN_POSITIVE);
        let rebuilt = VertexGrid::build_with_cell_size(&self.positions, desired_cell);
        if cancellation_requested(cancel) {
            return false;
        }
        self.grid = rebuilt;
        self.grid_radius = radius;
        true
    }

    /// Record each movable vertex's pre-dab position (weighted candidates + soup
    /// siblings) into `grid_dirty` and `pre_position`, deduped via a stamp.
    pub(super) fn snapshot_grid_region(
        &mut self,
        weighted: &[(usize, f32)],
        cancel: Option<&AtomicBool>,
    ) -> bool {
        if cancellation_requested(cancel) {
            return false;
        }
        self.grid_dirty.clear();
        let generation = self.next_stamp();
        for &(vertex_id, _) in weighted {
            if cancellation_requested(cancel) {
                return false;
            }
            if self.component_stamp[vertex_id] != generation {
                self.component_stamp[vertex_id] = generation;
                self.pre_position[vertex_id] = self.positions[vertex_id];
                self.grid_dirty.push((vertex_id, self.positions[vertex_id]));
            }
            for i in 0..self.position_siblings.row_len(vertex_id) {
                if cancellation_requested(cancel) {
                    return false;
                }
                let sibling = self.position_siblings.row(vertex_id)[i] as usize;
                if self.component_stamp[sibling] != generation {
                    self.component_stamp[sibling] = generation;
                    self.pre_position[sibling] = self.positions[sibling];
                    self.grid_dirty.push((sibling, self.positions[sibling]));
                }
            }
        }
        true
    }

    /// Relocate every snapshotted vertex from its pre-dab cell to its final one
    /// in a single pass (a within-cell move is a cheap no-op), so the grid is
    /// exact for the next dab's query without a per-pass or O(n) rebuild.
    pub(super) fn apply_grid_maintenance(&mut self, cancel: Option<&AtomicBool>) -> bool {
        let dirty = std::mem::take(&mut self.grid_dirty);
        for &(vertex_id, previous) in &dirty {
            if cancellation_requested(cancel) {
                self.grid_dirty = dirty;
                return false;
            }
            self.grid
                .relocate(vertex_id, previous, self.positions[vertex_id]);
        }
        self.grid_dirty = dirty;
        true
    }

    /// Keep a dab valid without turning one bad facet into a dead brush area.
    /// First back off the whole region coherently, then revert only vertices
    /// still involved in invalid triangles. Soup siblings are included in the
    /// same dirty set, so a local rollback cannot leave a split corner behind.
    pub(super) fn rollback_inversions(&mut self, cancel: Option<&AtomicBool>) -> bool {
        let generation = self.stamp_generation;
        let dirty = std::mem::take(&mut self.grid_dirty);
        if cancellation_requested(cancel) {
            self.grid_dirty = dirty;
            return false;
        }
        for _ in 0..MAX_ROLLBACK_ITERS {
            let invalid = dirty.par_iter().any(|&(vertex_id, _)| {
                if cancellation_requested(cancel) {
                    return false;
                }
                on_flipped_triangle(
                    vertex_id,
                    generation,
                    &self.incident_triangles,
                    &self.indices,
                    &self.positions,
                    &self.pre_position,
                    &self.component_stamp,
                )
            });
            if cancellation_requested(cancel) {
                self.grid_dirty = dirty;
                return false;
            }
            if !invalid {
                break;
            }
            for &(vertex_id, _) in &dirty {
                if cancellation_requested(cancel) {
                    self.grid_dirty = dirty;
                    return false;
                }
                let before = self.pre_position[vertex_id];
                let current = self.positions[vertex_id];
                self.positions[vertex_id] = before.lerp(current, 0.5);
            }
        }
        for _ in 0..MAX_LOCAL_ROLLBACK_ITERS {
            let invalid: Vec<usize> = dirty
                .par_iter()
                .filter(|&&(vertex_id, _)| {
                    if cancellation_requested(cancel) {
                        return false;
                    }
                    on_flipped_triangle(
                        vertex_id,
                        generation,
                        &self.incident_triangles,
                        &self.indices,
                        &self.positions,
                        &self.pre_position,
                        &self.component_stamp,
                    )
                })
                .map(|&(vertex_id, _)| vertex_id)
                .collect();
            if cancellation_requested(cancel) {
                self.grid_dirty = dirty;
                return false;
            }
            if invalid.is_empty() {
                break;
            }
            for vertex_id in invalid {
                if cancellation_requested(cancel) {
                    self.grid_dirty = dirty;
                    return false;
                }
                self.positions[vertex_id] = self.pre_position[vertex_id];
            }
        }
        self.grid_dirty = dirty;
        true
    }

    /// Recompute normals for the touched vertices and their one-ring, each
    /// affected vertex reading its own incident faces in parallel (the
    /// parallel-safe strategy sculpting engines use — no single-threaded face
    /// dedup).
    pub(super) fn recompute_normals_near(
        &mut self,
        touched: &[usize],
        cancel: Option<&AtomicBool>,
    ) -> Option<Vec<usize>> {
        if cancellation_requested(cancel) {
            return None;
        }
        // Build the scope (touched + welded rings + soup siblings) deduped via a
        // stamp — index loops, no sort, no allocation churn on a big brush.
        let scope_generation = self.next_stamp();
        let mut scope: Vec<usize> = Vec::with_capacity(touched.len() * 4);
        for &vertex_id in touched {
            if cancellation_requested(cancel) {
                return None;
            }
            if self.component_stamp[vertex_id] != scope_generation {
                self.component_stamp[vertex_id] = scope_generation;
                scope.push(vertex_id);
            }
            for &neighbor in self.adjacency.row(vertex_id) {
                if cancellation_requested(cancel) {
                    return None;
                }
                let neighbor = neighbor as usize;
                if self.component_stamp[neighbor] != scope_generation {
                    self.component_stamp[neighbor] = scope_generation;
                    scope.push(neighbor);
                }
            }
            for &sibling in self.position_siblings.row(vertex_id) {
                if cancellation_requested(cancel) {
                    return None;
                }
                let sibling = sibling as usize;
                if self.component_stamp[sibling] != scope_generation {
                    self.component_stamp[sibling] = scope_generation;
                    scope.push(sibling);
                }
            }
        }

        // Conflict-free parallel recompute (see `scope_area_normals`), then the
        // trivial serial normalize + write-back.
        let new_normals = scope_area_normals(
            &scope,
            &self.incident_triangles,
            &self.indices,
            &self.positions,
            cancel,
        )?;
        for (offset, &vertex_id) in scope.iter().enumerate() {
            if cancellation_requested(cancel) {
                return None;
            }
            let sum = new_normals[offset];
            // Match the full normal rebuild contract: a cancelled/degenerate
            // incident fan has no meaningful accumulated direction, so it
            // gets the deterministic fallback instead of retaining a normal
            // that described the pre-dab geometry.
            self.vertices[vertex_id].normal =
                if sum.is_finite() && sum.length_squared() > f32::EPSILON {
                    sum.normalize().to_array()
                } else {
                    Vec3::Z.to_array()
                };
        }
        Some(scope)
    }
}

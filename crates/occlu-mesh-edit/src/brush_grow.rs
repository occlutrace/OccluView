//! Dynamic-topology densification under the brush — the "the mesh gets denser
//! where you smooth" half of exocad-style freeforming.
//!
//! # Why a brush must add vertices
//!
//! A relaxer can only move vertices it already has. Drag Smooth along an edge
//! shared by two large triangles and NOTHING happens: there is no vertex on
//! that edge to pull toward its ring centroid. exocad, `ZBrush`'s Sculptris Pro
//! and Blender's Dyntopo all solve this the same way — the stroke retopologises
//! the surface under the cursor, so the relaxer gets the degrees of freedom it
//! needs and the resulting surface is clean.
//!
//! # What this module does
//!
//! One refinement pass per Smooth dab, before any displacement:
//!
//! * **Region**: the triangles that intersect the dab sphere, found by flooding
//!   from the vertex nearest the dab center through accepted triangles
//!   (Blender's `edge_queue_tri_in_sphere`). A vertex query is not enough — the
//!   whole point is the case where a triangle is much larger than the brush and
//!   NONE of its corners are inside the disc.
//! * **Criterion**: split a welded edge when it is longer than
//!   `radius * DETAIL_FRACTION_OF_RADIUS * SPLIT_HYSTERESIS`. Sizing detail off
//!   the brush radius is Blender's "Brush Detail" mode: the same gesture gives
//!   the same visual density at any zoom, and the operator's existing size
//!   slider doubles as the detail slider. The 4/3 band is Botsch & Kobbelt's
//!   incremental remesher (SGP 2004): halves of a split edge land above 2/3 of
//!   target, so they are never immediately re-split and the density converges.
//! * **Operation**: MIDPOINT edge split only. No collapse, no edge flip, no
//!   tangential relaxation. A midpoint split is the one refinement that leaves
//!   the piecewise-linear surface EXACTLY where it was (the new vertex lies on
//!   the edge, which lies in both incident triangles' planes), so densification
//!   can never drift the scan — the "keep the surface from moving while
//!   retopologising" problem the literature spends projection steps on simply
//!   does not arise. Smoothing afterwards is what moves the surface, under the
//!   existing falloff, boundary pinning and anti-inversion guards.
//! * **Placement**: an edge is only split when its MIDPOINT is inside the dab
//!   sphere, so a new vertex only ever appears where the brush actually passed.
//!
//! # Bound
//!
//! Growth is bounded three ways, tightest first:
//!
//! 1. The 4/3 criterion is a fixed point: once every edge under the disc is at
//!    or below target, further dabs at the same spot split nothing. A held
//!    stroke converges instead of exploding.
//! 2. [`MAX_SPLITS_PER_DAB`] caps one dab, so an enormous coarse facet cannot
//!    stall the interactive worker.
//! 3. [`super::BrushSession::added_vertex_budget`] caps the whole session, so
//!    a long stroke over a large coarse mesh cannot grow it without limit.
//!
//! # Soup
//!
//! The session smooths over WELDED topology but owns the original STL soup
//! index array. A split therefore works on the welded edge and rewires every
//! incident SOUP triangle to one shared new vertex, so the split corner is
//! welded by construction and no sibling propagation is needed for it.

use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashSet;

use super::BrushSession;
use crate::brush_math::refresh_step_budget;
use crate::EditVertex;

/// Target edge length under the brush, as a fraction of the dab radius —
/// Blender's "Brush Detail". A sixth of the radius puts roughly a dozen edges
/// across the brush diameter: dense enough that Smooth has real degrees of
/// freedom, coarse enough to stay interactive on a dental scan.
const DETAIL_FRACTION_OF_RADIUS: f32 = 1.0 / 6.0;

/// Split only above this multiple of the target length (Botsch & Kobbelt's 4/3
/// rule). The halves then land above 2/3 of target, so a refined region is a
/// fixed point rather than a split/re-split treadmill.
const SPLIT_HYSTERESIS: f32 = 4.0 / 3.0;

/// Refinement sweeps per dab. Each sweep halves every over-long edge, so six
/// take a 64x-too-long edge down to target inside a single dab.
const MAX_REFINE_SWEEPS: usize = 6;

/// Hard cap on splits performed by one dab. The worker runs off the UI thread
/// but the operator still feels a dab that takes too long.
const MAX_SPLITS_PER_DAB: usize = 4096;

/// Hard cap on triangles visited by one dab's region flood, so a brush far
/// larger than the mesh cannot turn the flood into a whole-mesh walk.
const MAX_REGION_TRIANGLES: usize = 1 << 17;

/// With at least this many vertices already inside the disc, the cheap
/// "longest incident edge" probe is trusted to decide that the region is
/// already fine enough to skip the region flood entirely. Below it the disc is
/// sparse — exactly the case where a giant triangle straddles the brush with no
/// corner inside — so the flood runs.
const REFINE_PROBE_MIN_VERTICES: usize = 32;

/// Radius doublings the seed search will try before giving up on finding any
/// vertex near the dab center.
const SEED_SEARCH_DOUBLINGS: usize = 8;

/// One welded edge queued for splitting.
struct SplitCandidate {
    /// Welded representative endpoints, ascending.
    key: (u32, u32),
    /// Squared length, for the longest-first ordering.
    length_squared: f32,
}

impl BrushSession {
    /// Densify the mesh under one dab and return how many vertices were added.
    ///
    /// Topology only: positions of existing vertices are untouched and the
    /// surface stays exactly where it was (see the module docs). Callers that
    /// want the full Smooth behaviour go through
    /// [`BrushSession::apply_stroke`], which runs this first.
    pub(crate) fn refine_dab(&mut self, center: Vec3, radius: f32) -> usize {
        let target = radius * DETAIL_FRACTION_OF_RADIUS;
        if !(target.is_finite() && target > 0.0 && center.is_finite()) {
            return 0;
        }
        let split_above = target * SPLIT_HYSTERESIS;
        let split_above_squared = split_above * split_above;
        let radius_squared = radius * radius;
        self.sync_grid(radius);

        let Some(seed) = self.refinement_seed(center, radius, split_above) else {
            return 0;
        };

        let started_with = self.vertices.len();
        let mut fresh: Vec<usize> = Vec::new();
        for _ in 0..MAX_REFINE_SWEEPS {
            if fresh.len() >= MAX_SPLITS_PER_DAB || self.growth_budget_spent() {
                break;
            }
            let triangles = self.dab_region_triangles(seed, center, radius);
            let candidates =
                self.split_candidates(&triangles, center, radius_squared, split_above_squared);
            if candidates.is_empty() {
                break;
            }
            let sweep_start = fresh.len();
            for candidate in &candidates {
                if fresh.len() >= MAX_SPLITS_PER_DAB || self.growth_budget_spent() {
                    break;
                }
                if let Some(new_id) = self.split_welded_edge(candidate) {
                    fresh.push(new_id);
                }
            }
            if fresh.len() == sweep_start {
                break;
            }
        }
        if fresh.is_empty() {
            return 0;
        }
        self.settle_new_vertices(&fresh);
        self.vertices.len() - started_with
    }

    /// Whether the session has used up its growth allowance.
    fn growth_budget_spent(&self) -> bool {
        self.vertices.len()
            >= self
                .prepared_vertices
                .saturating_add(self.added_vertex_budget)
    }

    /// A vertex to flood the dab region from, or `None` when refinement cannot
    /// help: no vertex anywhere near the dab, or a disc that is already dense
    /// AND fine-grained, where the flood would cost a millisecond to find
    /// nothing. The density precondition matters — a sparse disc is exactly the
    /// giant-triangle case whose edges never show up in a vertex query.
    fn refinement_seed(&self, center: Vec3, radius: f32, split_above: f32) -> Option<usize> {
        let inside: Vec<usize> = self
            .grid
            .query_radius(center, radius)
            .into_iter()
            .filter(|&id| self.positions[id].distance_squared(center) <= radius * radius)
            .collect();
        if inside.len() >= REFINE_PROBE_MIN_VERTICES {
            let longest = inside
                .par_iter()
                .map(|&id| self.longest_ring_edge(id))
                .reduce(|| 0.0_f32, f32::max);
            if longest <= split_above {
                return None;
            }
        }
        if let Some(&nearest) = inside.iter().min_by(|&&a, &&b| {
            self.positions[a]
                .distance_squared(center)
                .total_cmp(&self.positions[b].distance_squared(center))
                .then(a.cmp(&b))
        }) {
            return Some(nearest);
        }
        // Nothing inside the disc: the coarse case. Widen the search until a
        // corner of the straddling triangle turns up.
        let mut reach = radius;
        for _ in 0..SEED_SEARCH_DOUBLINGS {
            reach *= 2.0;
            if !reach.is_finite() {
                break;
            }
            let found = self.grid.query_radius(center, reach);
            let nearest = found.into_iter().min_by(|&a, &b| {
                self.positions[a]
                    .distance_squared(center)
                    .total_cmp(&self.positions[b].distance_squared(center))
                    .then(a.cmp(&b))
            });
            if nearest.is_some() {
                return nearest;
            }
        }
        None
    }

    /// Longest welded edge incident to `vertex_id` (zero for a bare soup
    /// duplicate, which carries no ring of its own).
    fn longest_ring_edge(&self, vertex_id: usize) -> f32 {
        let here = self.positions[vertex_id];
        self.adjacency
            .row(vertex_id)
            .iter()
            .filter_map(|&neighbour| self.positions.get(neighbour as usize))
            .map(|&position| position.distance(here))
            .filter(|length| length.is_finite())
            .fold(0.0_f32, f32::max)
    }

    /// Triangles intersecting the dab sphere, flooded outward from `seed`
    /// through accepted triangles. Rejected triangles are visited but not
    /// expanded, so the walk stays inside the brush footprint.
    fn dab_region_triangles(&mut self, seed: usize, center: Vec3, radius: f32) -> Vec<usize> {
        let generation = self.next_triangle_stamp();
        // The flood reads adjacency/incidence while stamping visited triangles;
        // lifting the stamp buffer out keeps those borrows disjoint.
        let mut stamp = std::mem::take(&mut self.triangle_stamp);
        let mut stack: Vec<usize> = Vec::new();
        let mut accepted: Vec<usize> = Vec::new();
        self.push_cluster_triangles(seed, &mut stack);
        while let Some(triangle) = stack.pop() {
            let Some(slot) = stamp.get_mut(triangle) else {
                continue;
            };
            if *slot == generation {
                continue;
            }
            *slot = generation;
            if !self.triangle_reaches(triangle, center, radius) {
                continue;
            }
            accepted.push(triangle);
            if accepted.len() >= MAX_REGION_TRIANGLES {
                break;
            }
            let base = triangle * 3;
            let Some(corners) = self.indices.get(base..base + 3) else {
                continue;
            };
            for &corner in &[corners[0], corners[1], corners[2]] {
                self.push_cluster_triangles(corner as usize, &mut stack);
            }
        }
        self.triangle_stamp = stamp;
        accepted.sort_unstable();
        accepted
    }

    /// Push every triangle incident to `vertex_id` OR to any of its soup
    /// siblings. Soup neighbours share a position, not an index, so a walk that
    /// only followed the exact corner id would never leave the first triangle.
    fn push_cluster_triangles(&self, vertex_id: usize, stack: &mut Vec<usize>) {
        stack.extend(
            self.incident_triangles
                .row(vertex_id)
                .iter()
                .map(|&t| t as usize),
        );
        for &sibling in self.position_siblings.row(vertex_id) {
            stack.extend(
                self.incident_triangles
                    .row(sibling as usize)
                    .iter()
                    .map(|&t| t as usize),
            );
        }
    }

    /// Whether any point of `triangle` lies within `radius` of `center`.
    fn triangle_reaches(&self, triangle: usize, center: Vec3, radius: f32) -> bool {
        let base = triangle * 3;
        let Some(corners) = self.indices.get(base..base + 3) else {
            return false;
        };
        let Some(a) = self.positions.get(corners[0] as usize).copied() else {
            return false;
        };
        let Some(b) = self.positions.get(corners[1] as usize).copied() else {
            return false;
        };
        let Some(c) = self.positions.get(corners[2] as usize).copied() else {
            return false;
        };
        closest_point_on_triangle(center, a, b, c).distance_squared(center) <= radius * radius
    }

    /// Welded edges of the region that are over-long AND whose midpoint is
    /// inside the dab sphere, longest first. The ordering is a total one (the
    /// endpoint pair breaks ties), so the split sequence — and every vertex id
    /// it mints — is identical run to run and thread count to thread count.
    fn split_candidates(
        &self,
        triangles: &[usize],
        center: Vec3,
        radius_squared: f32,
        split_above_squared: f32,
    ) -> Vec<SplitCandidate> {
        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        let mut edges: Vec<SplitCandidate> = Vec::new();
        for &triangle in triangles {
            let base = triangle * 3;
            let Some(corners) = self.indices.get(base..base + 3) else {
                continue;
            };
            let corners = [corners[0], corners[1], corners[2]];
            for slot in 0..3usize {
                let first = self.representative(corners[slot]);
                let second = self.representative(corners[(slot + 1) % 3]);
                if first == second {
                    continue;
                }
                let key = if first <= second {
                    (first, second)
                } else {
                    (second, first)
                };
                if !seen.insert(key) {
                    continue;
                }
                let (Some(&a), Some(&b)) = (
                    self.positions.get(key.0 as usize),
                    self.positions.get(key.1 as usize),
                ) else {
                    continue;
                };
                let length_squared = a.distance_squared(b);
                let midpoint = (a + b) * 0.5;
                if length_squared.is_finite()
                    && length_squared > split_above_squared
                    && midpoint.is_finite()
                    && midpoint.distance_squared(center) <= radius_squared
                {
                    edges.push(SplitCandidate {
                        key,
                        length_squared,
                    });
                }
            }
        }
        edges.sort_by(|a, b| {
            b.length_squared
                .total_cmp(&a.length_squared)
                .then(a.key.cmp(&b.key))
        });
        edges
    }

    /// Every triangle incident to the welded edge `first`—`second`, ascending.
    /// A welded edge is shared by exactly two triangles on a closed interior
    /// surface and one on an open border, even when the STL soup spells its two
    /// endpoints with different index pairs in each triangle.
    fn welded_edge_triangles(&self, first: u32, second: u32) -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        let mut heads: Vec<u32> = vec![first];
        heads.extend(self.position_siblings.row(first as usize).iter().copied());
        for head in heads {
            for &triangle in self.incident_triangles.row(head as usize) {
                let base = triangle as usize * 3;
                let Some(corners) = self.indices.get(base..base + 3) else {
                    continue;
                };
                if corners
                    .iter()
                    .any(|&corner| self.representative(corner) == second)
                {
                    found.push(triangle as usize);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The welded representative of a soup vertex id.
    fn representative(&self, vertex_id: u32) -> u32 {
        self.representative_of
            .get(vertex_id as usize)
            .copied()
            .unwrap_or(vertex_id)
    }

    /// Split one welded edge at its midpoint, rewiring every incident soup
    /// triangle to one shared new vertex. Returns the new vertex id, or `None`
    /// if the edge went stale (an earlier split in the same sweep already
    /// rewired it) or the mesh cannot hold another vertex.
    ///
    /// The incident triangles are recomputed from the incidence map rather than
    /// taken from the dab's region walk: half a split edge would be a T-junction
    /// crack in the surface, so a truncated or one-sided walk must never be able
    /// to cause one.
    fn split_welded_edge(&mut self, candidate: &SplitCandidate) -> Option<usize> {
        let (first, second) = candidate.key;
        // (triangle, corner on `first` side, corner on `second` side, opposite).
        let mut rewires: Vec<(usize, u32, u32, u32)> = Vec::new();
        for triangle in self.welded_edge_triangles(first, second) {
            let base = triangle * 3;
            let Some(corners) = self.indices.get(base..base + 3) else {
                continue;
            };
            let corners = [corners[0], corners[1], corners[2]];
            for slot in 0..3usize {
                let head = corners[slot];
                let tail = corners[(slot + 1) % 3];
                let opposite = corners[(slot + 2) % 3];
                let (head_rep, tail_rep) = (self.representative(head), self.representative(tail));
                if (head_rep, tail_rep) == (first, second)
                    || (head_rep, tail_rep) == (second, first)
                {
                    rewires.push((triangle, head, tail, opposite));
                    break;
                }
            }
        }
        if rewires.is_empty() {
            return None;
        }
        let (Some(&a), Some(&b)) = (
            self.positions.get(first as usize),
            self.positions.get(second as usize),
        ) else {
            return None;
        };
        let midpoint = (a + b) * 0.5;
        if !midpoint.is_finite() {
            return None;
        }
        let new_id = self.positions.len();
        let Ok(new_index) = u32::try_from(new_id) else {
            return None;
        };
        // An interior welded edge carries exactly two triangles; anything else
        // is an open border or a non-manifold flap, and its midpoint inherits
        // the same pinning `boundary_mask` would have given it.
        let on_boundary = rewires.len() != 2;
        let attributes = self.blend_endpoints(first, second);
        self.append_split_vertex(midpoint, new_index, attributes, on_boundary);

        for &(triangle, head, tail, opposite) in &rewires {
            let base = triangle * 3;
            // (head, tail, opposite) is a rotation of the stored corners, so
            // writing (head, new, opposite) + (new, tail, opposite) preserves
            // the winding and therefore the facet normals.
            if let Some(stored) = self.indices.get_mut(base..base + 3) {
                stored[0] = head;
                stored[1] = new_index;
                stored[2] = opposite;
            }
            let added_triangle = self.indices.len() / 3;
            let Ok(added_index) = u32::try_from(added_triangle) else {
                continue;
            };
            self.indices.extend_from_slice(&[new_index, tail, opposite]);
            self.triangle_stamp.push(0);
            let Ok(triangle_index) = u32::try_from(triangle) else {
                continue;
            };
            self.incident_triangles
                .remove_neighbour(tail as usize, triangle_index);
            self.incident_triangles
                .add_neighbour(new_id, triangle_index);
            self.incident_triangles.add_neighbour(new_id, added_index);
            self.incident_triangles
                .add_neighbour(tail as usize, added_index);
            self.incident_triangles
                .add_neighbour(opposite as usize, added_index);
            let opposite_rep = self.representative(opposite);
            self.adjacency
                .add_neighbour(opposite_rep as usize, new_index);
            self.adjacency.add_neighbour(new_id, opposite_rep);
        }
        self.adjacency.remove_neighbour(first as usize, second);
        self.adjacency.remove_neighbour(second as usize, first);
        self.adjacency.add_neighbour(first as usize, new_index);
        self.adjacency.add_neighbour(second as usize, new_index);
        self.adjacency.add_neighbour(new_id, first);
        self.adjacency.add_neighbour(new_id, second);
        Some(new_id)
    }

    /// Append the vertex a split mints, extending every per-vertex array the
    /// session keeps in lockstep with the vertex count.
    fn append_split_vertex(
        &mut self,
        midpoint: Vec3,
        new_index: u32,
        attributes: EditVertex,
        boundary: bool,
    ) {
        let new_id = self.positions.len();
        self.positions.push(midpoint);
        self.vertices.push(EditVertex {
            position: midpoint.to_array(),
            ..attributes
        });
        // A vertex that is never displaced must roll back onto itself, and it
        // must already be findable by the next dab's radius query.
        self.pre_position.push(midpoint);
        self.component_stamp.push(0);
        self.is_boundary.push(boundary);
        self.max_step.push(0.0);
        // A minted vertex is welded by construction: every incident soup
        // triangle shares this one id, so it represents itself.
        self.representative_of.push(new_index);
        self.adjacency.push_row();
        self.incident_triangles.push_row();
        self.position_siblings.push_row();
        self.grid.insert(new_id, midpoint);
    }

    /// Midpoint attributes: normal averaged, color and UV blended 50/50 from
    /// the welded endpoints, matching Dyntopo's `BM_data_interp_from_verts`.
    fn blend_endpoints(&self, first: u32, second: u32) -> EditVertex {
        let (Some(a), Some(b)) = (
            self.vertices.get(first as usize),
            self.vertices.get(second as usize),
        ) else {
            return EditVertex::at([0.0; 3]);
        };
        let normal = (Vec3::from_array(a.normal) + Vec3::from_array(b.normal)).normalize_or_zero();
        let blend_channel = |left: u8, right: u8| -> u8 {
            #[allow(clippy::cast_possible_truncation)]
            {
                ((u16::from(left) + u16::from(right)) / 2) as u8
            }
        };
        EditVertex {
            position: [0.0; 3],
            normal: normal.to_array(),
            color: [
                blend_channel(a.color[0], b.color[0]),
                blend_channel(a.color[1], b.color[1]),
                blend_channel(a.color[2], b.color[2]),
                blend_channel(a.color[3], b.color[3]),
            ],
            uv: [(a.uv[0] + b.uv[0]) * 0.5, (a.uv[1] + b.uv[1]) * 0.5],
        }
    }

    /// Bring the derived per-vertex state back in step after a dab's splits:
    /// the anti-inversion step budget over the new vertices and their rings,
    /// and the normals of everything whose incident faces changed.
    fn settle_new_vertices(&mut self, fresh: &[usize]) {
        let generation = self.next_stamp();
        let mut scope: Vec<usize> = Vec::with_capacity(fresh.len() * 5);
        for &vertex_id in fresh {
            if self.component_stamp[vertex_id] != generation {
                self.component_stamp[vertex_id] = generation;
                scope.push(vertex_id);
            }
            for &neighbour in self.adjacency.row(vertex_id) {
                let neighbour = neighbour as usize;
                if self.component_stamp[neighbour] != generation {
                    self.component_stamp[neighbour] = generation;
                    scope.push(neighbour);
                }
            }
        }
        refresh_step_budget(
            &scope,
            &self.positions,
            &self.adjacency,
            &self.position_siblings,
            &mut self.max_step,
        );
        self.recompute_normals_near(&scope);
    }

    /// Hand out the next triangle-stamp generation, resetting on the rare wrap.
    fn next_triangle_stamp(&mut self) -> u32 {
        self.triangle_stamp_generation = self.triangle_stamp_generation.wrapping_add(1);
        if self.triangle_stamp_generation == 0 {
            self.triangle_stamp.iter_mut().for_each(|slot| *slot = 0);
            self.triangle_stamp_generation = 1;
        }
        self.triangle_stamp_generation
    }
}

/// Closest point to `point` on triangle `a b c` (Ericson, *Real-Time Collision
/// Detection*, 5.1.5) — the exact test for "does this triangle reach into the
/// brush sphere", which corner distances alone get wrong for a triangle far
/// larger than the brush.
pub(crate) fn closest_point_on_triangle(point: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let denominator = d1 - d3;
        let along = if denominator.abs() > f32::MIN_POSITIVE {
            d1 / denominator
        } else {
            0.0
        };
        return a + ab * along;
    }
    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let denominator = d2 - d6;
        let along = if denominator.abs() > f32::MIN_POSITIVE {
            d2 / denominator
        } else {
            0.0
        };
        return a + ac * along;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let denominator = (d4 - d3) + (d5 - d6);
        let along = if denominator.abs() > f32::MIN_POSITIVE {
            (d4 - d3) / denominator
        } else {
            0.0
        };
        return b + (c - b) * along;
    }
    let denominator = va + vb + vc;
    if denominator.abs() <= f32::MIN_POSITIVE {
        return a;
    }
    let bary_v = vb / denominator;
    let bary_w = vc / denominator;
    a + ab * bary_v + ac * bary_w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_lands_inside_the_face_for_a_point_above_it() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(4.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 4.0, 0.0);
        let got = closest_point_on_triangle(Vec3::new(1.0, 1.0, 5.0), a, b, c);
        assert!((got - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn closest_point_clamps_to_a_corner_and_to_an_edge() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(4.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 4.0, 0.0);
        let corner = closest_point_on_triangle(Vec3::new(-3.0, -3.0, 0.0), a, b, c);
        assert!((corner - a).length() < 1e-5);
        let edge = closest_point_on_triangle(Vec3::new(2.0, -3.0, 0.0), a, b, c);
        assert!((edge - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn a_degenerate_triangle_still_answers_with_a_point_on_it() {
        let a = Vec3::ZERO;
        let got = closest_point_on_triangle(Vec3::new(1.0, 1.0, 1.0), a, a, a);
        assert!(got.is_finite());
    }
}

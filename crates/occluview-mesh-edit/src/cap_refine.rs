//! Cap refinement: interior vertices for a rim-only cap, at the rim's density.
//!
//! A cap triangulated on its rim alone is a fan of long slivers with nothing
//! inside to carry a shape. This pass densifies it until its edges match the
//! rim's (Liepa 2003, with Rivara-style splitting): longest-interior-edge
//! bisection of every edge longer than the local target edge scale, with
//! Lawson flips between passes.
//!
//! Where the rim projects onto a plane without folding, the connectivity work
//! is done in that plane, where a Delaunay triangulation is well defined and
//! unique. This matters: refining an ear-clip fan in 3D and flipping toward
//! "max-min angle" leaves a high-valence hub of radiating sliver triangles (a
//! visible starburst). In 2D, iterating Lawson edge flips to convergence
//! reaches the (constrained) Delaunay triangulation — bounded valence, no hub,
//! no slivers. A rim whose projection folds has no such plane; its cap is
//! refined on the triangulation's own 3D geometry, each flip judged in the
//! tangent frame of its quad.
//!
//! The refinement decides connectivity only. A generated vertex is placed
//! midway between the two it was split from, and the cap is recorded after
//! every pass; `cap_fair` takes those levels and gives the cap its shape.
//!
//! Rim vertices are never moved and rim edges are never split, so the cap stays
//! a drop-in watertight patch with no T-junctions.

use super::cap_lawson::CapMesh;
use super::cap_minweight::TakenTriangles;
use super::{EditVertex, GeneratedVertexPolicy};
use crate::numeric::{basis_from_normal, count_as_f32};
use glam::{Vec2, Vec3};
use std::collections::BTreeSet;

/// Liepa's density factor (√2): an interior edge is bisected while it is
/// longer than this factor times the local target edge scale.
const ALPHA: f32 = std::f32::consts::SQRT_2;
/// Refinement rounds; each round bisects every over-long interior edge once,
/// halving the worst edge, so the pass count needed is logarithmic in
/// (cap diameter / target scale) — 16 covers every practical hole.
const MAX_REFINE_PASSES: usize = 16;
/// Hard cap on generated vertices per hole, as a multiple of the rim length.
/// Density is set by the rim edge scale; this is only a runaway safety valve.
const MAX_GENERATED_PER_RIM: usize = 32;
/// Absolute interior-vertex budget per hole. Rim-density refinement of a large
/// hole needs `O(rim_len^2)` interior vertices. When the density estimate
/// exceeds this budget, the target edge scale is raised so refinement
/// terminates at a uniformly coarser (still even) sampling instead of
/// exhausting the per-hole vertex budget.
const CAP_INTERIOR_BUDGET: usize = 12_000;

/// Where a cap's edges are measured and its flips judged.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum CapDomain {
    /// The rim's own plane: for a triangulation that is valid in it.
    Plane,
    /// The triangulation's 3D geometry: for a rim whose projection folds.
    Space,
}

/// The cap after one refinement pass: the vertices it has so far (the rim
/// first, then generated ones in the order they were made) and its triangles.
pub(super) struct CapLevel {
    pub(super) vertex_count: usize,
    pub(super) triangles: Vec<[usize; 3]>,
}

/// A refined cap in local indices (`0..rim_len` = rim order, `rim_len..` =
/// generated).
pub(super) struct RefinedCap {
    /// Generated interior vertices: blended rim attributes, each positioned
    /// midway between its parents.
    pub(super) generated: Vec<EditVertex>,
    /// The two vertices each generated vertex was split from; both precede it.
    pub(super) parents: Vec<[usize; 2]>,
    /// The cap after each pass, coarse to fine. The first is the rim-only
    /// triangulation; the last is the cap.
    pub(super) levels: Vec<CapLevel>,
}

impl RefinedCap {
    /// The cap's triangles.
    pub(super) fn triangles(&self) -> &[[usize; 3]] {
        self.levels
            .last()
            .map_or(&[], |level| level.triangles.as_slice())
    }
}

/// Refine a rim-only cap over `rim` (ring order). `initial` holds the cap's
/// triangles in local rim indices with the final winding already applied by the
/// caller: an ear clip for [`CapDomain::Plane`], a minimum-area membrane for
/// [`CapDomain::Space`]. No flip produces a `taken` triangle.
pub(super) fn refine_cap(
    rim: &[EditVertex],
    initial: Vec<[usize; 3]>,
    domain: CapDomain,
    taken: &TakenTriangles,
    policy: GeneratedVertexPolicy,
) -> RefinedCap {
    let rim_len = rim.len();
    let positions: Vec<Vec3> = rim
        .iter()
        .map(|vertex| Vec3::from_array(vertex.position))
        .collect();
    let mut patch = CapPatch {
        uv: (domain == CapDomain::Plane).then(|| project_to_rim_plane(&positions)),
        positions,
        scale: Vec::new(),
        attrs: rim.to_vec(),
        parents: Vec::new(),
    };
    // Target edge scale per vertex: rim vertices average their two rim edges.
    #[expect(
        clippy::manual_midpoint,
        reason = "preserve established last-bit cap geometry"
    )]
    let scale = (0..rim_len)
        .map(|index| {
            let prev = (index + rim_len - 1) % rim_len;
            let next = (index + 1) % rim_len;
            (patch.length(index, prev) + patch.length(index, next)) * 0.5
        })
        .collect();
    patch.scale = scale;
    patch.rescale_for_budget(rim_len, &initial);

    // A first Lawson repair turns the rim-only fan into the Delaunay
    // triangulation of the rim before any splitting, so we densify a clean
    // base. The edge→owner map built here stays live through every bisection
    // and flip below (`CapMesh`), so each repair touches only rewritten edges
    // instead of sweeping the whole cap.
    let mut cap_mesh = CapMesh::avoiding(initial, taken.clone());
    let all_edges: BTreeSet<(usize, usize)> = cap_mesh.edges_sorted().into_iter().collect();
    patch.repair(&mut cap_mesh, all_edges);
    let mut levels = vec![CapLevel {
        vertex_count: rim_len,
        triangles: cap_mesh.triangles().to_vec(),
    }];

    // Density refinement by longest-interior-edge bisection (Rivara-style),
    // with incremental Lawson repair between passes. Bisection is the
    // sliver-proof choice: the rim-only base of a many-thousand-edge rim is a
    // fan of long slivers that flips alone cannot fully regularize, and
    // centroid (1:3) splits of slivers cascade — an 8000-edge rim runs
    // straight to the runaway valve.
    // Halving the longest edge attacks exactly the sliver axis, provably
    // terminates (each split halves one edge, lengths are bounded below by
    // the target scale), and the per-pass repairs restore Delaunay quality —
    // seeded only by the edges the pass's splits actually rewrote.
    for _ in 0..MAX_REFINE_PASSES {
        if !patch.bisect_pass(&mut cap_mesh, rim_len, policy) {
            break;
        }
        levels.push(CapLevel {
            vertex_count: patch.positions.len(),
            triangles: cap_mesh.triangles().to_vec(),
        });
    }

    RefinedCap {
        generated: patch.attrs.split_off(rim_len),
        parents: patch.parents,
        levels,
    }
}

/// The rim's vertices in its own plane: the plane through the rim's centroid
/// with the rim's Newell normal.
fn project_to_rim_plane(rim: &[Vec3]) -> Vec<Vec2> {
    let rim_len = rim.len();
    let mut centroid = Vec3::ZERO;
    for &point in rim {
        centroid += point;
    }
    centroid /= count_as_f32(rim_len.max(1));

    // Newell's method: robust polygon normal for a non-planar rim. Vertices
    // are taken relative to the centroid: Newell is translation-invariant in
    // exact arithmetic, and centering avoids the catastrophic f32 cancellation
    // a small far-from-origin rim would otherwise hit.
    let mut normal = Vec3::ZERO;
    for index in 0..rim_len {
        let current = rim[index] - centroid;
        let next = rim[(index + 1) % rim_len] - centroid;
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    let normal = if normal.is_finite() && normal.length_squared() > f32::EPSILON {
        normal.normalize()
    } else {
        Vec3::Z
    };
    let (tangent_u, tangent_v) = basis_from_normal(normal);
    rim.iter()
        .map(|&point| {
            let relative = point - centroid;
            Vec2::new(relative.dot(tangent_u), relative.dot(tangent_v))
        })
        .collect()
}

/// The growable cap state shared by the refinement passes. The triangulation
/// itself lives in [`CapMesh`], which keeps its edge→owner map live across
/// passes.
struct CapPatch {
    /// Planar coordinates, for a cap refined in its rim's plane.
    uv: Option<Vec<Vec2>>,
    /// 3D positions: the rim's own, and each generated vertex midway between
    /// its parents.
    positions: Vec<Vec3>,
    /// Target edge scale per vertex.
    scale: Vec<f32>,
    attrs: Vec<EditVertex>,
    /// Parents of each generated vertex.
    parents: Vec<[usize; 2]>,
}

impl CapPatch {
    /// Length of edge `(u, v)` in the cap's domain.
    fn length(&self, u: usize, v: usize) -> f32 {
        match &self.uv {
            Some(uv) => uv[u].distance(uv[v]),
            None => self.positions[u].distance(self.positions[v]),
        }
    }

    /// Area of a triangle in the cap's domain.
    fn area(&self, [a, b, c]: [usize; 3]) -> f64 {
        let doubled = if let Some(uv) = &self.uv {
            (uv[b] - uv[a]).perp_dot(uv[c] - uv[a]).abs()
        } else {
            let (pa, pb, pc) = (self.positions[a], self.positions[b], self.positions[c]);
            (pb - pa).cross(pc - pa).length()
        };
        f64::from(doubled * 0.5)
    }

    /// Lawson repair from a seed set, judged in the cap's domain.
    fn repair(&self, cap_mesh: &mut CapMesh, suspects: BTreeSet<(usize, usize)>) {
        match &self.uv {
            Some(uv) => cap_mesh.lawson(uv, suspects),
            None => cap_mesh.lawson_3d(&self.positions, suspects),
        }
    }

    /// One bisection pass: every interior edge longer than `ALPHA` times its
    /// local target scale is split at its midpoint with a conforming 2:4
    /// rewrite of both owner triangles (edge→owner map updated in place), then
    /// one incremental Lawson repair seeded by the rewritten edges. Returns
    /// whether anything split. Rim edges (one owner) are never touched.
    /// Deterministic: the edge snapshot is processed in sorted order, and
    /// edges a split removed disappear from the live map, so stale snapshot
    /// entries skip themselves.
    fn bisect_pass(
        &mut self,
        cap_mesh: &mut CapMesh,
        rim_len: usize,
        policy: GeneratedVertexPolicy,
    ) -> bool {
        let generated_limit = rim_len.saturating_mul(MAX_GENERATED_PER_RIM);
        let mut split_any = false;
        let mut suspects: BTreeSet<(usize, usize)> = BTreeSet::new();
        for key in cap_mesh.edges_sorted() {
            if self.positions.len() - rim_len >= generated_limit {
                break;
            }
            // Rim edges (one owner), non-manifold noise, and snapshot entries a
            // split already removed all fail the live owner-pair lookup.
            let Some(owners) = cap_mesh.owner_pair(key) else {
                continue;
            };
            let (u, v) = key;
            #[expect(
                clippy::manual_midpoint,
                reason = "preserve established last-bit cap geometry"
            )]
            let target = (self.scale[u] + self.scale[v]) * 0.5;
            if self.length(u, v) <= ALPHA * target {
                continue;
            }
            let midpoint_index = self.positions.len();
            let midpoint = (self.positions[u] + self.positions[v]) * 0.5;
            if let Some(uv) = self.uv.as_mut() {
                let planar = (uv[u] + uv[v]) * 0.5;
                uv.push(planar);
            }
            self.attrs
                .push(midpoint_vertex(midpoint, &self.attrs, [u, v], policy));
            self.positions.push(midpoint);
            self.scale.push(target);
            self.parents.push([u, v]);
            cap_mesh.bisect(key, owners, midpoint_index, &mut suspects);
            split_any = true;
        }
        self.repair(cap_mesh, suspects);
        split_any
    }

    /// Raise the target edge scale so the estimated interior vertex count
    /// stays within [`CAP_INTERIOR_BUDGET`]. Refinement density is quadratic
    /// in the rim length for round holes; without this, a 20 000-edge rim can
    /// exceed the per-hole vertex budget. Rims small enough to fit the budget
    /// (~250 edges for a round hole) are left byte-for-byte unchanged.
    fn rescale_for_budget(&mut self, rim_len: usize, initial: &[[usize; 3]]) {
        if rim_len < 3 {
            return;
        }
        // The rim-only triangulation covers the cap, so its area is the
        // cap's; summed in f64 for stable summation.
        let area: f64 = initial.iter().map(|&triangle| self.area(triangle)).sum();
        let mut mean_scale = 0.0_f64;
        for &s in self.scale.iter().take(rim_len) {
            mean_scale += f64::from(s);
        }
        mean_scale /= f64::from(count_as_f32(rim_len));
        // Equilateral-triangle area at the target edge scale.
        let per_triangle = 3.0_f64.sqrt() / 4.0 * mean_scale * mean_scale;
        if !(per_triangle.is_finite() && per_triangle > 0.0) {
            return;
        }
        // Interior vertices approach half the triangle count for a dense patch;
        // bisection stops at edges up to ALPHA times the target scale, which
        // doubles the realized density versus the equilateral estimate (ALPHA^2),
        // so the two 2x factors cancel: estimate = area / per_triangle * 0.5 * 2.
        let estimated = area / per_triangle;
        let budget = f64::from(u32::try_from(CAP_INTERIOR_BUDGET).unwrap_or(u32::MAX));
        if estimated <= budget {
            return;
        }
        let factor = (estimated / budget).sqrt();
        if !factor.is_finite() {
            return;
        }
        // f64 -> f32: factor is in (1, sqrt(area/budget)]; well within f32 range.
        #[allow(clippy::cast_possible_truncation)]
        let factor = factor.min(f64::from(f32::MAX)) as f32;
        for s in &mut self.scale {
            *s *= factor;
        }
    }
}

/// Attributes for a bisection midpoint: the average of its edge endpoints.
fn midpoint_vertex(
    position: Vec3,
    attrs: &[EditVertex],
    endpoints: [usize; 2],
    policy: GeneratedVertexPolicy,
) -> EditVertex {
    match policy {
        GeneratedVertexPolicy::InterpolateBoundary => {
            let mut color = [0u16; 4];
            let mut uv = [0.0f32; 2];
            for &endpoint in &endpoints {
                let vertex = &attrs[endpoint];
                for (sum, &channel) in color.iter_mut().zip(&vertex.color) {
                    *sum += u16::from(channel);
                }
                uv[0] += vertex.uv[0];
                uv[1] += vertex.uv[1];
            }
            let mut vertex = EditVertex::at(position.to_array());
            for (channel, &sum) in vertex.color.iter_mut().zip(&color) {
                // Average of two u8 channels always fits back into u8.
                *channel = u8::try_from(sum / 2).unwrap_or(u8::MAX);
            }
            vertex.uv = [uv[0] / 2.0, uv[1] / 2.0];
            vertex
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A coarse fan membrane over a small rim must densify through 3D
    /// bisection, keep every edge's incidence exact (rim one owner, interior
    /// two), produce only finite positions, and leave no degenerate triangle.
    /// This is the path a rim whose projection folds takes.
    #[test]
    fn spatial_refinement_densifies_a_coarse_membrane_and_stays_manifold() {
        let rim_len = 8usize;
        let radius = 5.0_f32;
        let rim: Vec<EditVertex> = (0..rim_len)
            .map(|index| {
                let theta = std::f32::consts::TAU * (index as f32) / (rim_len as f32);
                EditVertex::at([
                    radius * theta.cos(),
                    radius * theta.sin(),
                    0.4 * (2.0 * theta).sin(),
                ])
            })
            .collect();
        // A coarse fan membrane: one hub at rim vertex 0.
        let membrane: Vec<[usize; 3]> = (1..rim_len - 1).map(|i| [0, i, i + 1]).collect();

        let cap = refine_cap(
            &rim,
            membrane,
            CapDomain::Space,
            &TakenTriangles::new(),
            GeneratedVertexPolicy::InterpolateBoundary,
        );

        assert!(
            !cap.generated.is_empty(),
            "a coarse fan over an 8-gon must densify"
        );
        assert!(
            cap.generated
                .iter()
                .all(|vertex| vertex.position.iter().all(|c| c.is_finite())),
            "generated positions stay finite"
        );

        let is_rim_edge = |u: usize, v: usize| {
            u < rim_len && v < rim_len && ((u + 1) % rim_len == v || (v + 1) % rim_len == u)
        };
        let mut incidence: HashMap<(usize, usize), usize> = HashMap::new();
        for &[a, b, c] in cap.triangles() {
            for (u, v) in [(a, b), (b, c), (c, a)] {
                *incidence.entry((u.min(v), u.max(v))).or_default() += 1;
            }
        }
        for (&(u, v), &owners) in &incidence {
            let expected = if is_rim_edge(u, v) { 1 } else { 2 };
            assert_eq!(
                owners, expected,
                "edge ({u}, {v}): expected {expected} owners, got {owners}"
            );
        }

        let position = |index: usize| -> Vec3 {
            if index < rim_len {
                Vec3::from_array(rim[index].position)
            } else {
                Vec3::from_array(cap.generated[index - rim_len].position)
            }
        };
        for &[a, b, c] in cap.triangles() {
            let (pa, pb, pc) = (position(a), position(b), position(c));
            assert!(
                (pb - pa).cross(pc - pa).length() > 1e-9,
                "refinement left a degenerate cap triangle"
            );
        }
    }

    /// Every level is the cap as one pass left it: its triangles name only the
    /// vertices that existed then, and each generated vertex sits midway
    /// between two earlier ones. The shape solve walks the levels on that.
    #[test]
    fn levels_grow_from_the_rim_and_parents_precede_their_midpoints() {
        let rim_len = 24usize;
        let rim: Vec<EditVertex> = (0..rim_len)
            .map(|index| {
                let theta = std::f32::consts::TAU * (index as f32) / (rim_len as f32);
                EditVertex::at([4.0 * theta.cos(), 4.0 * theta.sin(), 0.0])
            })
            .collect();
        let fan: Vec<[usize; 3]> = (1..rim_len - 1).map(|i| [0, i + 1, i]).collect();
        for domain in [CapDomain::Plane, CapDomain::Space] {
            let cap = refine_cap(
                &rim,
                fan.clone(),
                domain,
                &TakenTriangles::new(),
                GeneratedVertexPolicy::InterpolateBoundary,
            );
            assert_eq!(cap.levels[0].vertex_count, rim_len);
            assert!(cap.levels.len() > 1, "{domain:?}: a 24-gon must densify");
            assert_eq!(
                cap.levels.last().map(|level| level.vertex_count),
                Some(rim_len + cap.generated.len())
            );
            for pair in cap.levels.windows(2) {
                assert!(pair[0].vertex_count < pair[1].vertex_count);
            }
            for level in &cap.levels {
                assert!(level
                    .triangles
                    .iter()
                    .flatten()
                    .all(|&vertex| vertex < level.vertex_count));
            }
            let position = |index: usize| -> Vec3 {
                if index < rim_len {
                    Vec3::from_array(rim[index].position)
                } else {
                    Vec3::from_array(cap.generated[index - rim_len].position)
                }
            };
            for (offset, &[a, b]) in cap.parents.iter().enumerate() {
                let index = rim_len + offset;
                assert!(a < index && b < index);
                assert!(position(index).distance((position(a) + position(b)) * 0.5) < 1e-5);
            }
        }
    }
}

//! Interpolated hole caps (exocad-style "close holes"): refine a planar
//! ear-clip cap with interior vertices until it matches the rim's edge
//! density, then drape the interior onto a smooth surface fitted to the rim so
//! the patch follows the surrounding shape instead of denting inward.
//!
//! All connectivity work is done in the cap's 2D tangent plane, where a
//! Delaunay triangulation is well defined and unique. This matters: refining an
//! ear-clip fan in 3D and flipping toward "max-min angle" leaves a high-valence
//! hub of radiating sliver triangles (a visible starburst). In 2D, iterating
//! Lawson edge flips to convergence reaches the (constrained) Delaunay
//! triangulation — bounded valence, no hub, no slivers.
//!
//! Two-part scheme:
//!  1. Size-driven refinement (Liepa 2003, with Rivara-style splitting):
//!     longest-interior-edge bisection of every triangle larger than the
//!     local target edge scale, with Lawson flips between passes.
//!  2. Curvature-following lift: a quadric height field is least-squares fitted
//!     to the rim AND a band of surface samples just outside it (the rim ring
//!     alone is nearly planar and carries no curvature). Interior vertices are
//!     distributed evenly in-plane, then lifted onto that quadric. Pure
//!     umbrella relaxation in 3D converges to the MINIMAL surface, which sinks
//!     into a visible dent on any curved hole; lifting onto the fitted surface
//!     removes the dent.
//!
//! Rim vertices are never moved and rim edges are never split, so the cap stays
//! a drop-in watertight patch with no T-junctions.

use super::cap_delaunay::relax_uv;
use super::cap_fit::fit_cap_surface;
use super::cap_lawson::CapMesh;
use super::{EditVertex, GeneratedVertexPolicy};
use glam::{Vec2, Vec3};
use std::collections::BTreeSet;

/// Liepa's density factor (√2): an interior edge is bisected while it is
/// longer than this factor times the local target edge scale.
const ALPHA: f32 = std::f32::consts::SQRT_2;
/// Refinement rounds; each round bisects every over-long interior edge once,
/// halving the worst edge, so the pass count needed is logarithmic in
/// (cap diameter / target scale) — 16 covers every practical hole.
const MAX_REFINE_PASSES: usize = 16;
/// Harmonic sweeps that blend the rim residual into the cap interior. The rim
/// residual decays over a few vertex rings, so this needs to be generous enough
/// to propagate across a large cap.
const HARMONIC_ITERATIONS: usize = 128;
/// Hard cap on generated vertices per hole, as a multiple of the rim length.
/// Density is set by the rim edge scale; this is only a runaway safety valve.
const MAX_GENERATED_PER_RIM: usize = 32;
/// Absolute interior-vertex budget per hole. Rim-density refinement of a large
/// hole needs `O(rim_len^2)` interior vertices, which made a 1000-edge rim take
/// seconds and a 20 000-edge rim minutes. When the density estimate exceeds
/// this budget, the target edge scale is raised so refinement terminates at a
/// uniformly coarser (still even) sampling instead of stalling mid-pass.
const CAP_INTERIOR_BUDGET: usize = 12_000;

/// A refined cap: generated interior vertices plus the full cap triangulation
/// in LOCAL indices (`0..rim_len` = rim order, `rim_len..` = generated).
pub(super) struct RefinedCap {
    pub(super) generated: Vec<EditVertex>,
    pub(super) triangles: Vec<[usize; 3]>,
}

/// Refine and relax an ear-clip cap over `rim` (ring order, 3D positions).
/// `support` / `support_distances` hold surface samples just outside the rim
/// (curvature pinning) with their walked distance back to the rim.
/// `initial` holds ear-clip triangles in local rim indices with the final
/// winding already applied by the caller.
pub(super) fn refine_and_relax(
    rim: &[EditVertex],
    support: &[[f32; 3]],
    support_distances: &[f32],
    initial: Vec<[usize; 3]>,
    policy: GeneratedVertexPolicy,
) -> RefinedCap {
    let rim_len = rim.len();
    let rim_positions: Vec<Vec3> = rim
        .iter()
        .map(|vertex| Vec3::from_array(vertex.position))
        .collect();
    let surface = fit_cap_surface(&rim_positions, support, support_distances);

    // Work in the cap's tangent plane. Rim vertices keep their exact projected
    // coordinates; generated vertices are created and relaxed here.
    let uv: Vec<Vec2> = rim_positions.iter().map(|&p| surface.local_ab(p)).collect();
    let attrs: Vec<EditVertex> = rim.to_vec();
    // Target edge scale per vertex: rim vertices average their two rim edges.
    let mut scale: Vec<f32> = (0..rim_len)
        .map(|index| {
            let prev = uv[(index + rim_len - 1) % rim_len];
            let next = uv[(index + 1) % rim_len];
            let here = uv[index];
            (here.distance(prev) + here.distance(next)) * 0.5
        })
        .collect();
    rescale_for_budget(&uv, rim_len, &mut scale);

    // A first Lawson repair turns the ear-clip fan into the Delaunay
    // triangulation of the rim before any splitting, so we densify a clean
    // base. The edge→owner map built here stays LIVE through every bisection
    // and flip below (`CapMesh`), replacing the retired whole-cap flip sweeps
    // that made a ~1000-edge rim cost seconds (issue #9).
    let mut cap_mesh = CapMesh::new(initial);
    let all_edges: BTreeSet<(usize, usize)> = cap_mesh.edges_sorted().into_iter().collect();
    cap_mesh.lawson(&uv, all_edges);

    // Density refinement by LONGEST-INTERIOR-EDGE bisection (Rivara-style),
    // with incremental Lawson repair between passes. Bisection is the
    // sliver-proof choice: the ear-clip base of a many-thousand-edge rim is a
    // fan of long slivers that flips alone cannot fully regularize, and
    // centroid (1:3) splits of slivers cascade — a 8000-edge rim used to blow
    // straight to the runaway valve (256k vertices, minutes of work).
    // Halving the longest edge attacks exactly the sliver axis, provably
    // terminates (each split halves one edge, lengths are bounded below by
    // the target scale), and the per-pass repairs restore Delaunay quality —
    // seeded ONLY by the edges the pass's splits actually rewrote.
    let mut patch = CapPatch { uv, scale, attrs };
    for _ in 0..MAX_REFINE_PASSES {
        let split_any = bisect_pass(
            &mut patch,
            &mut cap_mesh,
            rim_len,
            |ab| surface.lift(ab),
            policy,
        );
        if !split_any {
            break;
        }
    }
    let triangles = cap_mesh.into_triangles();
    let CapPatch {
        mut uv,
        scale: _,
        attrs,
    } = patch;

    relax_uv(&mut uv, rim_len, &triangles);

    // Blend the quadric base with the rim's residual (its deviation from the
    // quadric). The rim of a wavy surface undulates off the smooth quadric; a
    // pure quadric cap meets it with a slope crease. Harmonically interpolating
    // that residual inward makes the cap meet the rim tangentially, then decays
    // to the quadric in the interior. Only the SMALL residual is harmonic, so
    // no minimal-surface dent is reintroduced.
    let mut residual = vec![0.0f32; uv.len()];
    for index in 0..rim_len {
        let rim_height = (rim_positions[index] - surface.centroid).dot(surface.normal);
        residual[index] = rim_height - surface.height(uv[index]);
    }
    harmonic_interior(&mut residual, rim_len, &triangles);

    // Lift the final planar cap onto the fitted surface plus blended residual.
    let generated = attrs
        .into_iter()
        .zip(uv.iter().zip(&residual))
        .skip(rim_len)
        .map(|(mut vertex, (&ab, &extra))| {
            vertex.position = surface.lift_with(ab, extra).to_array();
            vertex
        })
        .collect();
    RefinedCap {
        generated,
        triangles,
    }
}

/// The growable cap state shared by the refinement passes: planar positions,
/// per-vertex target scales, and vertex attributes. The triangulation itself
/// lives in [`CapMesh`], which keeps its edge→owner map live across passes.
struct CapPatch {
    uv: Vec<Vec2>,
    scale: Vec<f32>,
    attrs: Vec<EditVertex>,
}

/// One bisection pass: every interior edge longer than `ALPHA` times its
/// local target scale is split at its midpoint with a conforming 2:4 rewrite
/// of both owner triangles (edge→owner map updated in place), then ONE
/// incremental Lawson repair seeded by the rewritten edges. Returns whether
/// anything split. Rim edges (one owner) are never touched. Deterministic:
/// the edge snapshot is processed in sorted order, and edges a split removed
/// disappear from the live map, so stale snapshot entries skip themselves.
fn bisect_pass(
    patch: &mut CapPatch,
    cap_mesh: &mut CapMesh,
    rim_len: usize,
    lift: impl Fn(Vec2) -> Vec3,
    policy: GeneratedVertexPolicy,
) -> bool {
    let CapPatch { uv, scale, attrs } = patch;
    let mut split_any = false;
    let mut suspects: BTreeSet<(usize, usize)> = BTreeSet::new();
    for key in cap_mesh.edges_sorted() {
        if uv.len() - rim_len >= rim_len * MAX_GENERATED_PER_RIM {
            break;
        }
        // Rim edges (one owner), non-manifold noise, and snapshot entries a
        // split already removed all fail the live owner-pair lookup.
        let Some(owners) = cap_mesh.owner_pair(key) else {
            continue;
        };
        let (u, v) = key;
        let target = (scale[u] + scale[v]) * 0.5;
        if uv[u].distance(uv[v]) <= ALPHA * target {
            continue;
        }
        let midpoint_index = uv.len();
        let midpoint = (uv[u] + uv[v]) * 0.5;
        let vertex = midpoint_vertex(lift(midpoint), attrs, [u, v], policy);
        attrs.push(vertex);
        uv.push(midpoint);
        scale.push(target);
        cap_mesh.bisect(key, owners, midpoint_index, &mut suspects);
        split_any = true;
    }
    cap_mesh.lawson(uv, suspects);
    split_any
}

/// Raise the target edge scale so the estimated interior vertex count stays
/// within [`CAP_INTERIOR_BUDGET`]. Refinement density is quadratic in the rim
/// length for round holes; without this, a 20 000-edge rim generated 640 000
/// interior vertices and the fill ran for minutes. Rims small enough to fit
/// the budget (~250 edges for a round hole) are left byte-for-byte unchanged.
fn rescale_for_budget(uv: &[Vec2], rim_len: usize, scale: &mut [f32]) {
    if rim_len < 3 {
        return;
    }
    // Shoelace area of the projected rim polygon, in f64 for stable summation.
    let mut doubled_area = 0.0_f64;
    for index in 0..rim_len {
        let a = uv[index];
        let b = uv[(index + 1) % rim_len];
        doubled_area += f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y);
    }
    let area = (doubled_area * 0.5).abs();
    let mut mean_scale = 0.0_f64;
    for &s in scale.iter().take(rim_len) {
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
    for s in scale.iter_mut() {
        *s *= factor;
    }
}

/// Harmonically interpolate a scalar field over the cap interior with the rim
/// values held fixed (Laplace with Dirichlet boundary): each interior value
/// converges to the mean of its neighbors. Used to blend the rim residual
/// inward so the cap meets the surrounding surface without a slope crease.
fn harmonic_interior(values: &mut [f32], rim_len: usize, triangles: &[[usize; 3]]) {
    if values.len() <= rim_len {
        return;
    }
    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); values.len()];
    for &[a, b, c] in triangles {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if !neighbors[u].contains(&v) {
                neighbors[u].push(v);
            }
            if !neighbors[v].contains(&u) {
                neighbors[v].push(u);
            }
        }
    }
    // Convergence tolerance relative to the boundary data magnitude: once a
    // sweep moves nothing beyond it, further sweeps are numeric noise.
    let mut max_abs = 0.0_f32;
    for &value in values.iter().take(rim_len) {
        max_abs = max_abs.max(value.abs());
    }
    let tolerance = max_abs * 1e-4;
    for _ in 0..HARMONIC_ITERATIONS {
        let mut max_delta = 0.0_f32;
        for index in rim_len..values.len() {
            let ring = &neighbors[index];
            if ring.is_empty() {
                continue;
            }
            let mut sum = 0.0;
            for &neighbor in ring {
                sum += values[neighbor];
            }
            let updated = sum / count_as_f32(ring.len());
            max_delta = max_delta.max((updated - values[index]).abs());
            values[index] = updated;
        }
        if max_delta <= tolerance {
            break;
        }
    }
}

/// Lossless-enough count-to-float for averaging small vertex fans. Cap sizes
/// never approach `u16::MAX`, so the saturation only guards a pathological rim.
fn count_as_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// Attributes for a bisection midpoint: the average of its edge endpoints
/// (or the deterministic neutral fallback).
fn midpoint_vertex(
    position: Vec3,
    attrs: &[EditVertex],
    endpoints: [usize; 2],
    policy: GeneratedVertexPolicy,
) -> EditVertex {
    match policy {
        GeneratedVertexPolicy::NeutralFallback => EditVertex::at(position.to_array()),
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

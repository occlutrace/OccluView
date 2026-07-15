//! Planar Delaunay machinery for hole caps: Lawson edge-flip sweeps (with a
//! cocircular shorter-diagonal tie-break) and the validity-guarded in-plane
//! relaxation. Split out of `cap_refine.rs` (file-size budget).

use glam::Vec2;

/// Lawson flip sweeps per convergence loop (a safety bound; it usually
/// settles in a handful of sweeps).
const MAX_FLIP_SWEEPS: usize = 64;
/// In-plane smoothing iterations that even out the interior sampling.
const RELAX_ITERATIONS: usize = 32;

/// Lawson edge-flip sweeps to convergence: flip any chord shared by two cap
/// triangles that violates the 2D Delaunay (empty-circumcircle) criterion.
/// Rim edges have a single cap triangle, so they are never candidates and the
/// rim polygon stays a constraint. Reaching Delaunay removes the fan hub.
pub(super) fn flip_to_delaunay(uv: &[Vec2], triangles: &mut [[usize; 3]]) {
    for _ in 0..MAX_FLIP_SWEEPS {
        if !flip_sweep(uv, triangles) {
            break;
        }
    }
}

/// One Lawson sweep. Returns whether any edge was flipped.
fn flip_sweep(uv: &[Vec2], triangles: &mut [[usize; 3]]) -> bool {
    use std::collections::HashMap;
    let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (triangle_index, &[a, b, c]) in triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            by_edge
                .entry((u.min(v), u.max(v)))
                .or_default()
                .push(triangle_index);
        }
    }
    let mut live_edges: std::collections::HashSet<(usize, usize)> =
        by_edge.keys().copied().collect();
    let mut changed = false;
    // Deterministic flip order: HashMap iteration is randomized per process,
    // and the flip ORDER changes the final triangulation — sweeping in hash
    // order made cap quality (and the exact output mesh) vary run to run.
    let mut edges: Vec<(usize, usize)> = by_edge.keys().copied().collect();
    edges.sort_unstable();
    for (u, v) in edges {
        let owners = &by_edge[&(u, v)];
        let [t1, t2] = match owners.as_slice() {
            [t1, t2] => [*t1, *t2],
            _ => continue,
        };
        // An earlier flip in this sweep may have rewritten either triangle;
        // the edge map is built once, so re-check both still carry the edge.
        let carries_edge = |triangle: [usize; 3]| triangle.contains(&u) && triangle.contains(&v);
        if !carries_edge(triangles[t1]) || !carries_edge(triangles[t2]) {
            continue;
        }
        let Some((apex1, apex2)) = (apex_of(triangles[t1], u, v)).zip(apex_of(triangles[t2], u, v))
        else {
            continue;
        };
        // The flipped diagonal must be a NEW edge, or the cap goes non-manifold.
        let diagonal = (apex1.min(apex2), apex1.max(apex2));
        if live_edges.contains(&diagonal) {
            continue;
        }
        // Convex-quad test: both candidate triangles must keep the original
        // winding sign (non-degenerate, non-inverted), else the flip folds the
        // quad or crosses a rim notch.
        let sign = signed_area(uv, triangles[t1]).signum();
        let candidate1 = replace_edge(triangles[t1], (u, v), apex2);
        let candidate2 = replace_edge(triangles[t2], (v, u), apex1);
        let a1 = signed_area(uv, candidate1);
        let a2 = signed_area(uv, candidate2);
        if a1 * sign <= f32::EPSILON || a2 * sign <= f32::EPSILON {
            continue;
        }
        // Delaunay: flip if apex2 lies inside the circumcircle of t1
        // (u,v,apex1). On a COCIRCULAR tie (all four points on one circle —
        // the shape of a round hole rim, where the determinant vanishes and
        // classic Lawson never fires) fall back to the shorter diagonal:
        // strictly length-decreasing, so it cannot cycle, and it dissolves
        // the ear-clip's long parallel chords that density refinement would
        // otherwise have to chop wholesale.
        let verdict = circumcircle_verdict(uv[u], uv[v], uv[apex1], uv[apex2]);
        let flip = match verdict {
            CircleVerdict::Inside => true,
            CircleVerdict::Tie => {
                let old_len = uv[u].distance_squared(uv[v]);
                let new_len = uv[apex1].distance_squared(uv[apex2]);
                new_len < old_len * 0.999
            }
            CircleVerdict::Outside => false,
        };
        if flip {
            triangles[t1] = candidate1;
            triangles[t2] = candidate2;
            live_edges.remove(&(u, v));
            live_edges.insert(diagonal);
            changed = true;
        }
    }
    changed
}

/// The vertex of `triangle` that is not on edge `(u, v)`.
fn apex_of(triangle: [usize; 3], u: usize, v: usize) -> Option<usize> {
    triangle
        .into_iter()
        .find(|&vertex| vertex != u && vertex != v)
}

/// Rebuild `triangle` with the endpoint `edge.1` swapped for the new apex.
/// The caller drops a DIFFERENT endpoint from each of the two triangles, which
/// preserves consistent winding and all four quad-boundary directed edges.
fn replace_edge(triangle: [usize; 3], edge: (usize, usize), new_apex: usize) -> [usize; 3] {
    let mut result = triangle;
    for slot in &mut result {
        if *slot == edge.1 {
            *slot = new_apex;
            return result;
        }
    }
    result
}

/// Signed area (2x) of a triangle in the plane; sign encodes winding.
fn signed_area(uv: &[Vec2], triangle: [usize; 3]) -> f32 {
    let [a, b, c] = triangle.map(|index| uv[index]);
    (b - a).perp_dot(c - a)
}

/// Circumcircle classification of `query` against triangle `(tri_a, tri_b,
/// tri_c)`: strictly inside, strictly outside, or numerically COCIRCULAR.
enum CircleVerdict {
    Inside,
    Outside,
    Tie,
}

/// Standard in-circle determinant, evaluated relative to `query`, with a
/// RELATIVE tie band: the determinant scales with the fourth power of the
/// quad size, so an absolute epsilon misclassifies every tie on real-world
/// coordinates. The sign is taken against the triangle's own winding so the
/// predicate is orientation-agnostic.
fn circumcircle_verdict(tri_a: Vec2, tri_b: Vec2, tri_c: Vec2, query: Vec2) -> CircleVerdict {
    let orient = (tri_b - tri_a).perp_dot(tri_c - tri_a);
    if orient.abs() <= f32::EPSILON {
        return CircleVerdict::Outside;
    }
    let da = tri_a - query;
    let db = tri_b - query;
    let dc = tri_c - query;
    let det = da.length_squared() * db.perp_dot(dc) - db.length_squared() * da.perp_dot(dc)
        + dc.length_squared() * da.perp_dot(db);
    // Relative tie threshold: mean squared reach of the quad, squared again
    // to match the determinant's quartic scaling.
    let reach = (da.length_squared() + db.length_squared() + dc.length_squared()) / 3.0;
    let tie_band = (reach * reach) * 1e-4;
    let signed = if orient > 0.0 { det } else { -det };
    if signed > tie_band {
        CircleVerdict::Inside
    } else if signed < -tie_band {
        CircleVerdict::Outside
    } else {
        CircleVerdict::Tie
    }
}

/// Even out the interior sampling: each generated vertex moves toward the mean
/// of its neighbors in the plane while the rim stays pinned. Uniform in-plane
/// distribution lifts to an evenly sampled cap.
///
/// The move is VALIDITY-GUARDED: it is applied only if it keeps every incident
/// triangle on the same (nonzero) side it started on. Near a concave rim notch
/// a raw Laplacian step could push an interior vertex across a rim edge and
/// invert a triangle (a self-intersecting cap); the guard forbids exactly that.
pub(super) fn relax_uv(uv: &mut [Vec2], rim_len: usize, triangles: &[[usize; 3]]) {
    if uv.len() <= rim_len {
        return;
    }
    let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); uv.len()];
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); uv.len()];
    for (triangle_index, &[a, b, c]) in triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if !neighbors[u].contains(&v) {
                neighbors[u].push(v);
            }
            if !neighbors[v].contains(&u) {
                neighbors[v].push(u);
            }
        }
        for &vertex in &[a, b, c] {
            incident[vertex].push(triangle_index);
        }
    }

    for _ in 0..RELAX_ITERATIONS {
        for index in rim_len..uv.len() {
            let ring = &neighbors[index];
            if ring.is_empty() {
                continue;
            }
            let mut sum = Vec2::ZERO;
            for &neighbor in ring {
                sum += uv[neighbor];
            }
            let degree = f32::from(u16::try_from(ring.len()).unwrap_or(u16::MAX));
            let target = sum / degree;
            // Damp toward the neighbor centroid; accept only if no incident
            // triangle collapses or flips sign.
            let proposed = uv[index].lerp(target, 0.5);
            if incident[index].iter().all(|&triangle_index| {
                let tri = triangles[triangle_index];
                let before = signed_area(uv, tri);
                let after = signed_area_with(uv, tri, index, proposed);
                before * after > f32::EPSILON
            }) {
                uv[index] = proposed;
            }
        }
    }
}

/// Signed area (2x) of `triangle` with vertex `moved` relocated to `position`.
fn signed_area_with(uv: &[Vec2], triangle: [usize; 3], moved: usize, position: Vec2) -> f32 {
    let p = triangle.map(|index| if index == moved { position } else { uv[index] });
    (p[1] - p[0]).perp_dot(p[2] - p[0])
}

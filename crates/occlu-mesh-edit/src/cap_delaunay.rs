//! Planar Delaunay predicates for hole caps (in-circle, winding, quad
//! rewrite) and the validity-guarded in-plane relaxation. The flip ENGINE —
//! worklist-driven incremental Lawson over a persistent edge map — lives in
//! `cap_lawson.rs`; this file keeps the shared geometry so both stay within
//! the file-size budget.

use glam::Vec2;

/// In-plane smoothing iterations that even out the interior sampling.
const RELAX_ITERATIONS: usize = 32;

/// The vertex of `triangle` that is not on edge `(u, v)`.
pub(super) fn apex_of(triangle: [usize; 3], u: usize, v: usize) -> Option<usize> {
    triangle
        .into_iter()
        .find(|&vertex| vertex != u && vertex != v)
}

/// Rebuild `triangle` with the endpoint `edge.1` swapped for the new apex.
/// The caller drops a DIFFERENT endpoint from each of the two triangles, which
/// preserves consistent winding and all four quad-boundary directed edges.
pub(super) fn replace_edge(
    triangle: [usize; 3],
    edge: (usize, usize),
    new_apex: usize,
) -> [usize; 3] {
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
pub(super) fn signed_area(uv: &[Vec2], triangle: [usize; 3]) -> f32 {
    let [a, b, c] = triangle.map(|index| uv[index]);
    (b - a).perp_dot(c - a)
}

/// Circumcircle classification of `query` against triangle `(tri_a, tri_b,
/// tri_c)`: strictly inside, strictly outside, or numerically COCIRCULAR.
///
/// On a cocircular tie (the shape of a round hole rim, where the determinant
/// vanishes and classic Lawson never fires) the engine falls back to the
/// shorter diagonal: strictly length-decreasing, so it cannot cycle, and it
/// dissolves the ear-clip's long parallel chords that density refinement
/// would otherwise have to chop wholesale.
pub(super) enum CircleVerdict {
    Inside,
    Outside,
    Tie,
}

/// Standard in-circle determinant, evaluated relative to `query`, with a
/// RELATIVE tie band: the determinant scales with the fourth power of the
/// quad size, so an absolute epsilon misclassifies every tie on real-world
/// coordinates. The sign is taken against the triangle's own winding so the
/// predicate is orientation-agnostic.
pub(super) fn circumcircle_verdict(
    tri_a: Vec2,
    tri_b: Vec2,
    tri_c: Vec2,
    query: Vec2,
) -> CircleVerdict {
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

//! Planar Delaunay predicates for hole caps.

use glam::Vec2;

/// The vertex of `triangle` that is not on edge `(u, v)`.
pub(super) fn apex_of(triangle: [usize; 3], u: usize, v: usize) -> Option<usize> {
    triangle
        .into_iter()
        .find(|&vertex| vertex != u && vertex != v)
}

/// Rebuild `triangle` with `edge.1` replaced by the new apex.
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
/// tri_c)`: inside, outside, or numerically cocircular.
///
/// Cocircular ties use the shorter diagonal, which strictly decreases length
/// and prevents cycling.
pub(super) enum CircleVerdict {
    Inside,
    Outside,
    Tie,
}

/// Standard in-circle determinant, evaluated relative to `query`, with a
/// relative tie band: the determinant scales with the fourth power of the
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

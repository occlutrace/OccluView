//! Exact triangle projection used by the surface index.

use glam::DVec3;

/// Closest point on a triangle to `point` — Ericson's region test, which
/// handles the face, the three edges, and the three corners without branching
/// on a projection that may fall outside.
pub(crate) fn closest_point_on_triangle(point: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
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
    let vc = d1 * d4 - d3 * d2;
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let denominator = d1 - d3;
        if denominator.abs() > f64::EPSILON {
            return a + ab * (d1 / denominator);
        }
        return a;
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
        if denominator.abs() > f64::EPSILON {
            return a + ac * (d2 / denominator);
        }
        return a;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let denominator = (d4 - d3) + (d5 - d6);
        if denominator.abs() > f64::EPSILON {
            return b + (c - b) * ((d4 - d3) / denominator);
        }
        return b;
    }
    let total = va + vb + vc;
    if total.abs() <= f64::EPSILON {
        return a;
    }
    a + ab * (vb / total) + ac * (vc / total)
}

//! Flatten (Minus) dab displacement, shared by every sculpt session.
//!
//! A flatten dab moves each selected vertex toward the selection plane along
//! the plane normal by its weighted share of the signed distance, so a bump
//! levels without sliding sideways. The plane — selection centroid plus area
//! normal — is built by the calling session, which owns selection, protection
//! and undo; this module answers one vertex at a time so the stroke cannot
//! drift between products sharing the brush.

use glam::DVec3;

/// What flattening needs to know about the surface under a weighted
/// selection. A trait rather than buffers because the sessions index their
/// surfaces differently, and neither may be rebuilt into the other's shape
/// just to be levelled.
pub trait FlattenSurface {
    /// Position of a vertex in the caller's own index space.
    fn flatten_position(&self, vertex: u32) -> DVec3;
}

/// One vertex's share of a flatten dab: the signed distance from the plane,
/// travelled along the plane normal, scaled by the clamped strength and the
/// selection weight. The normal is normalized here so a caller-side scale on
/// the area normal cannot leak into the stroke. The function proposes, never
/// writes: budgets, protection and undo stay in the owning session.
/// Callers feeding an already-weighted displacement into a second law (the
/// knot clamp) pass weight 1.0 there: this function owns the single weighting.
// the plane, weight, strength and clamp factors are one displacement law.
#[allow(clippy::too_many_arguments)]
pub fn flatten_displacement<S: FlattenSurface>(
    surface: &S,
    vertex: u32,
    weight: f64,
    strength: f64,
    plane_point: DVec3,
    plane_normal: DVec3,
) -> DVec3 {
    // A non-finite weight is refused rather than clamped: no caller produces
    // one, and a clamped jump from +inf is worse than no move. Held is the
    // same branch: weight zero must stay exactly zero, never a rounding.
    if weight <= 0.0 || !weight.is_finite() {
        return DVec3::ZERO;
    }
    // A corrupt strength must not reach the mesh: zero is always a safe dab.
    if !strength.is_finite() {
        return DVec3::ZERO;
    }
    let pos = surface.flatten_position(vertex);
    // A corrupt position or plane has no distance to travel: refusing keeps
    // one bad vertex from laundering NaN into its neighbours' undo.
    if !pos.is_finite() || !plane_point.is_finite() || !plane_normal.is_finite() {
        return DVec3::ZERO;
    }
    // The projection divides by the normal length, so a degenerate normal
    // names no plane to travel toward and must stop here rather than mint
    // an amplified or NaN displacement.
    let len = plane_normal.length();
    if !len.is_finite() || len <= 1e-12 {
        return DVec3::ZERO;
    }
    let unit = plane_normal * (1.0 / len);
    // Strength is the operator's share of the way to the plane: past one
    // would overshoot through it, below zero would raise the bump.
    let share = strength.clamp(0.0, 1.0) * weight;
    let out = unit * ((plane_point - pos).dot(unit) * share);
    // Overflow from huge coordinates must not reach the mesh as infinity.
    if out.is_finite() {
        out
    } else {
        DVec3::ZERO
    }
}

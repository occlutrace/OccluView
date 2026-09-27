//! Explicit Laplacian relaxation — the other half of the shared fairing family.
//!
//! There are two ways to smooth a selection, and they are not the same operator:
//!
//! - [`crate::fair_selection`] solves an implicit cotangent system. One call
//!   damps low frequencies across the whole footprint at any dose, so it
//!   reshapes large form quickly — which is what a form-level fairing pass
//!   wants, and what a brush does not want when the operator is trying to iron
//!   one-ring noise with a light touch.
//! - This module relaxes explicitly, one pass at a time, toward the one-ring
//!   centroid. Its reach is bounded by the pass count, so a light touch is a
//!   local effect and form survives until the count climbs.
//!
//! Both live here because both are product-neutral geometry, and because a
//! module that needs smoothing should find the whole family in one place
//! instead of writing a third variant. Which one a module runs, and at what
//! dose, stays that module's clinical decision — like every other policy in
//! `PLATFORM.md` §9, the operator is shared and the session is not.
//!
//! Nothing here writes: the target is returned and the caller applies its own
//! step budget, protection mask and journal.

use glam::DVec3;

/// What the operator needs from the surface under a selection.
///
/// A trait rather than buffers for the same reason [`crate::FairingSurface`] is
/// one: sessions index their surfaces differently (one by welded group, one by
/// mesh vertex) and neither should rebuild its topology into the other's shape
/// just to be relaxed.
pub trait RelaxSurface {
    /// Position of a vertex in the caller's own index space.
    fn relax_position(&self, vertex: u32) -> DVec3;
    /// One-ring neighbours of a vertex.
    fn relax_neighbors(&self, vertex: u32) -> &[u32];
}

/// Where `vertex` lands after one Laplacian pass toward its one-ring centroid,
/// approached by `factor * weight`. `None` when there is nothing to do — a ring
/// too small to have a centroid, or a step below arithmetic noise — so a caller
/// writes nothing and records nothing for that vertex.
///
/// `factor` is the pass gain and `weight` the vertex's share of the selection.
/// The product is clamped to ±1, so a strength above one cannot overshoot past
/// the centroid and a negative `factor` (the second half of a Taubin pass,
/// which pushes away instead of toward) stays bounded by the same rule.
pub fn laplacian_relax_target<S: RelaxSurface + ?Sized>(
    surface: &S,
    vertex: u32,
    factor: f64,
    weight: f64,
) -> Option<DVec3> {
    let ring = surface.relax_neighbors(vertex);
    // Structural, not a copy of any session's relaxability policy: a 1-ring has
    // no centroid to relax toward, only a pull at a single neighbour, and
    // averaging one point is not an average. Same reasoning as
    // `tangential_respace_target`.
    if ring.len() < 2 {
        return None;
    }
    let mut mean = DVec3::ZERO;
    for &neighbor in ring {
        mean += surface.relax_position(neighbor);
    }
    let centroid = mean * (1.0 / ring.len() as f64);
    let here = surface.relax_position(vertex);
    let t = (factor * weight).clamp(-1.0, 1.0);
    let target = here + ((centroid - here) * t);
    ((target - here).length() > 1e-15).then_some(target)
}

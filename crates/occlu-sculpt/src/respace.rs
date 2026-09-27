//! Tangential vertex-spacing relaxation used by the live remesh loop.
//!
//! Fairing changes surface shape. This pass moves vertices toward their
//! one-ring centroid after removing the normal component, so it evens spacing
//! while preserving the local surface. It proposes targets; the session owns
//! step limits, protection masks, surface checks, and undo records.

use glam::DVec3;

/// Per-pass share of the tangential centroid offset a full-weight vertex takes.
pub const RESPACE_GAIN: f64 = 0.5;

/// What the pass needs to know about the surface under a weighted selection.
///
/// A trait rather than buffers for the same reason [`crate::FairingSurface`] is
/// one: sessions index their surfaces differently, and neither may be rebuilt
/// into the other's shape just to even out spacing.
pub trait RespaceSurface {
    /// Position of a vertex in the caller's own index space.
    fn respace_position(&self, vertex: u32) -> DVec3;
    /// Unit surface normal at a vertex.
    fn respace_normal(&self, vertex: u32) -> DVec3;
    /// One-ring neighbours of a vertex.
    fn respace_neighbors(&self, vertex: u32) -> &[u32];
}

/// Where `vertex` should land to even out its spacing: its one-ring centroid
/// with the normal component removed, approached by `gain * weight`. `None`
/// when there is nothing to do — a ring too small to have a centroid, a
/// degenerate normal, or a step below arithmetic noise — so the caller writes
/// nothing and records nothing for that vertex.
///
/// Removing the normal component is what makes this redistribution and not a
/// second fairing pass: the vertex stays in the surface the operator shaped. On
/// a flat fan the centroid lies in the plane, so the whole step is tangential
/// and the plane is preserved bit for bit; on a ridge the pull is pure normal,
/// so nothing moves and the shape is not invented away.
pub fn tangential_respace_target<S: RespaceSurface + ?Sized>(
    surface: &S,
    vertex: u32,
    gain: f64,
    weight: f64,
) -> Option<DVec3> {
    // A non-finite weight is refused rather than clamped: no caller produces
    // one, and a silent budget-clamped jump is worse than no move.
    if weight <= 0.0 || !weight.is_finite() || gain <= 0.0 {
        return None;
    }
    let ring = surface.respace_neighbors(vertex);
    // Structural, not a copy of any session's policy: a one-ring has no centroid
    // to even toward, only a pull at a single neighbour, and averaging one point
    // is not an average.
    if ring.len() < 2 {
        return None;
    }
    let mut mean = DVec3::ZERO;
    for &neighbor in ring {
        mean += surface.respace_position(neighbor);
    }
    mean *= 1.0 / ring.len() as f64;
    let here = surface.respace_position(vertex);
    let normal = surface.respace_normal(vertex).normalize_or_zero();
    let delta = mean - here;
    let tangent = if normal.length() > 1e-12 {
        delta - (normal * delta.dot(normal))
    } else {
        // No usable normal leaves no tangential direction to trust; the
        // structural answer is to move nothing rather than pick an axis.
        return None;
    };
    let step = tangent * (gain * weight).clamp(0.0, 1.0);
    if !step.is_finite() || step.length() <= 1e-15 {
        return None;
    }
    Some(here + step)
}

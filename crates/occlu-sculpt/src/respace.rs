//! Tangential vertex-spacing relaxation — the spacing half of a Smooth dab.
//!
//! The fairing solve decides shape; this pass decides SPACING. It slides each
//! vertex toward its one-ring centroid with the surface-normal component
//! removed, so it opens a bunched fan and evens crowded rows while leaving the
//! form exactly where it was. This is the half a fairing operator can never
//! supply: a Laplacian pull moves the vertices a bad triangulation already has
//! and never redistributes them, so a seam of long slivers, a density step and
//! a bunched row survive any number of smoothing strokes. `fairing.rs` owns the
//! shape; this owns the tessellation the operator actually reads on screen.
//!
//! The function proposes, never writes: clamping to a step budget, the
//! protection mask and the undo record stay in the owning session, which is the
//! only place that knows them.
//!
//! This module owns the per-pass OPERATOR only. How many passes run, in what
//! order relative to the topological operators, and where the reprojection goes
//! is the isotropic loop's business and lives with the loop (`remesh.rs`),
//! because those three are what make the passes converge: split and collapse
//! change which edges exist, the passes move the vertices that make those edges
//! long or short, and only the reprojection keeps a repeated pass on the surface
//! the operator shaped. A pass is not the operator on its own — it is one
//! iteration of a fixed-point solve, and a single iteration is not a solve.

use glam::DVec3;

/// Per-pass share of the tangential centroid offset a full-weight vertex takes.
pub const RESPACE_GAIN: f64 = 0.5;

/// Sequential relaxation passes the isotropic loop runs per cycle. This is a
/// ceiling, not a dose: the loop stops earlier when the flow settles. The
/// reference remeshers relax several times between their topological passes —
/// the Polygon Mesh Processing library's `uniform_remeshing` calls its
/// `tangential_smoothing(5)` once per cycle — because one damped Jacobi step
/// moves only the highest-frequency part of the spacing error and leaves every
/// larger-scale unevenness exactly where it was.
pub const RESPACE_PASSES: usize = 4;

/// A vertex whose whole tangential pull is below this share of the cycle's
/// target edge length is already evenly spaced and is left alone. This is the
/// loop's flow-distance stopping scale: the reference remeshers terminate when
/// the mean per-pass movement falls to roughly one percent of the target edge,
/// and without such a scale a relaxation never reports that it has finished.
pub const RESPACE_SETTLED_SHARE: f64 = 0.01;

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

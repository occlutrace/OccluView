//! Dab-displacement clamping that cannot tear topology, shared by sculpt sessions.
//!
//! A dab proposes a raw displacement per vertex; near the selection rim that
//! proposal drags free vertices away from held ones and opens a rip along the
//! edge loop. The session cannot see the rip coming — it owns protection and
//! budgets, not the local frame — so the clamp lives here, in the one place
//! that projects the proposal onto the surface before it is written.
//!
//! INVARIANT: the output never moves a vertex tangentially further than half
//! its shortest ring edge, and a held vertex (weight 0) stays exactly 0. Half
//! an edge cannot cross the neighbour it is measured against, so vertex
//! ordering around the ring survives any single dab. Normal motion is passed
//! through at the selection weight: leaving the surface is shape, which the
//! session owns, while sliding along it is topology, which this clamp owns.
//!
//! The function proposes, never writes: step budgets, protection masks and
//! undo records stay in the owning session.

use glam::DVec3;

/// Tangential travel as a share of the shortest ring edge. Crossing a
/// neighbour needs the full edge, so one half keeps the ring ordered while
/// still answering the hand; a smaller share would fight large-radius soft
/// brushes that legitimately traverse most of a coarse triangle.
pub const KNOT_TANGENT_SHARE: f64 = 0.5;

/// What the clamp needs to know about the surface under a weighted selection.
/// A trait rather than buffers because the sessions index their surfaces
/// differently, and neither may be rebuilt into the other's shape just to be
/// clamped.
pub trait KnotSurface {
    /// Position of a vertex in the caller's own index space.
    fn knot_position(&self, vertex: u32) -> DVec3;
    /// Unit surface normal at a vertex.
    fn knot_normal(&self, vertex: u32) -> DVec3;
    /// One-ring neighbours of a vertex.
    fn knot_neighbors(&self, vertex: u32) -> &[u32];
}

/// Project one vertex's raw dab displacement onto its local surface frame and
/// clamp the tearing component. Held stays exactly zero, a NaN proposal is
/// refused rather than propagated, and whenever no clamp binds the answer is
/// the weight-scaled raw vector bit for bit, so direction is preserved
/// wherever preservation is safe. Callers feeding an already-weighted
/// displacement (flatten output) pass weight 1.0: double weighting would
/// square the falloff.
pub fn clamp_dab_displacement<S: KnotSurface + ?Sized>(
    surface: &S,
    vertex: u32,
    weight: f64,
    raw: DVec3,
) -> DVec3 {
    // A non-finite weight is refused rather than clamped: no caller produces
    // one, and a budget-clamped jump from +inf is worse than no move. Held is
    // the same branch: weight zero must stay exactly zero, never a rounding.
    if weight <= 0.0 || !weight.is_finite() {
        return DVec3::ZERO;
    }
    // A corrupt proposal must not reach the mesh: zero is always a safe dab.
    if !raw.is_finite() {
        return DVec3::ZERO;
    }
    // A session bug handing in a weight above one must not amplify the stroke.
    let w = weight.min(1.0);
    let here = surface.knot_position(vertex);
    let normal = surface.knot_normal(vertex);
    // A degenerate normal leaves no frame to project onto. There is nothing
    // tangential to tear with, so the weight gate alone decides.
    if !normal.is_finite() || normal.length() <= 1e-12 {
        return raw * w;
    }
    let unit = normal * (1.0 / normal.length());
    let along = raw.dot(unit);
    let tangent = raw - unit * along;
    let travel = tangent.length();
    if travel <= 0.0 || !travel.is_finite() {
        return raw * w;
    }
    // Shortest FINITE ring edge. Non-finite neighbour positions are skipped,
    // never trusted: a NaN edge would mint a NaN bound and launder the NaN
    // this function exists to stop.
    let mut shortest = f64::INFINITY;
    let mut edges = 0usize;
    for &neighbor in surface.knot_neighbors(vertex) {
        let edge = (surface.knot_position(neighbor) - here).length();
        if edge.is_finite() {
            edges += 1;
            if edge < shortest {
                shortest = edge;
            }
        }
    }
    // An isolated vertex has no neighbours to tear away from; with no finite
    // edge at all the mesh is corrupt beyond a topological verdict, and the
    // finite weight-scaled proposal is still the only finite answer.
    if edges == 0 || !shortest.is_finite() {
        return raw * w;
    }
    // Coincident neighbours separate under any tangential move, so a zero
    // shortest edge removes the tangential share while the normal pass keeps
    // the dab alive.
    let bound = KNOT_TANGENT_SHARE * shortest.max(0.0);
    if travel <= bound {
        return raw * w;
    }
    if bound <= 0.0 {
        return unit * (along * w);
    }
    // The tangential share shrinks to the bound; the normal share is untouched.
    // Direction changes only as far as the tear limit demands.
    (unit * (along * w)) + (tangent * ((bound / travel) * w))
}

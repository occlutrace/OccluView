//! The surface a remesh edit must stay on, read from the edit's own
//! neighbourhood.
//!
//! A live edit rewrites the faces around one edge or one vertex, and the
//! surface it has to preserve is exactly those faces before the edit. The
//! checks here therefore read a patch of at most two vertex stars. Building a
//! search tree over the whole footprint on every dab answered the same
//! question with thousands of tree queries, the largest single cost of a dab.

use super::*;
use occlu_geometry_math::closest_point_on_triangle;

/// Faces around one or two vertices, at their positions before an edit.
///
/// Fixed capacity on the stack: two stars of an ordinary surface hold far
/// fewer faces, and an edit around a vertex of extreme valence is refused
/// rather than allocated for.
pub(in crate::sculpt_session::kernel) struct LocalSurface {
    faces: [(u32, [DVec3; 3]); Self::CAPACITY],
    len: usize,
}

impl LocalSurface {
    const CAPACITY: usize = 32;

    pub(in crate::sculpt_session::kernel) fn new() -> Self {
        Self {
            faces: [(u32::MAX, [DVec3::ZERO; 3]); Self::CAPACITY],
            len: 0,
        }
    }

    /// Add one face. False when the patch is full.
    pub(in crate::sculpt_session::kernel) fn push(
        &mut self,
        face: u32,
        corners: [DVec3; 3],
    ) -> bool {
        if self.len == Self::CAPACITY {
            return false;
        }
        self.faces[self.len] = (face, corners);
        self.len += 1;
        true
    }

    fn faces(&self) -> &[(u32, [DVec3; 3])] {
        &self.faces[..self.len]
    }

    /// Closest point of the patch to `point`, with its distance.
    pub(in crate::sculpt_session::kernel) fn nearest(&self, point: DVec3) -> Option<(DVec3, f64)> {
        let mut best: Option<(DVec3, f64)> = None;
        for &(_, [a, b, c]) in self.faces() {
            let candidate = closest_point_on_triangle(point, a, b, c);
            let distance = (candidate - point).length();
            if distance.is_finite() && best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((candidate, distance));
            }
        }
        best
    }

    /// Whether `point` lies within `tolerance` of the patch. The face named by
    /// `hint` is tried first: a sample taken on an edited face almost always
    /// lies on that face's own old position, so most queries end after one
    /// test.
    pub(in crate::sculpt_session::kernel) fn within(
        &self,
        point: DVec3,
        tolerance: f64,
        hint: Option<u32>,
    ) -> bool {
        let close = |corners: [DVec3; 3]| {
            (closest_point_on_triangle(point, corners[0], corners[1], corners[2]) - point).length()
                <= tolerance
        };
        if let Some(hint) = hint {
            if let Some(&(_, corners)) = self.faces().iter().find(|(face, _)| *face == hint) {
                if close(corners) {
                    return true;
                }
            }
        }
        self.faces()
            .iter()
            .any(|&(face, corners)| Some(face) != hint && close(corners))
    }

    /// Whether a new face stays on the patch: its edge midpoints and its
    /// centroid all lie within `tolerance`. These are the points where a
    /// retriangulation of points on a surface departs from that surface.
    pub(in crate::sculpt_session::kernel) fn covers(
        &self,
        triangle: [DVec3; 3],
        tolerance: f64,
        hint: Option<u32>,
    ) -> bool {
        let [a, b, c] = triangle;
        [
            ((a + b) * 0.5),
            ((b + c) * 0.5),
            ((c + a) * 0.5),
            (((a + b) + c) * (1.0 / 3.0)),
        ]
        .into_iter()
        .all(|sample| self.within(sample, tolerance, hint))
    }
}

impl SculptSession {
    /// Largest distance, in millimetres, a live remesh edit may put between
    /// the surface before and after it. A quarter of the target edge length,
    /// kept between ten microns and a tenth of a millimetre.
    pub(in crate::sculpt_session::kernel) fn remesh_tolerance_mm(target: f64) -> f64 {
        (0.25 * target).clamp(0.010, 0.100)
    }

    /// The live faces around `groups`, each once, at their current positions.
    /// `None` when the patch would exceed its fixed capacity.
    pub(in crate::sculpt_session::kernel) fn local_surface(
        &self,
        groups: &[u32],
    ) -> Option<LocalSurface> {
        let mut patch = LocalSurface::new();
        for &group in groups {
            for &face in self.topology.incident_triangles(group) {
                if patch.faces().iter().any(|(seen, _)| *seen == face) {
                    continue;
                }
                let corners = self.topology.triangle(face)?;
                if !patch.push(face, corners.map(|corner| self.group_v(corner))) {
                    return None;
                }
            }
        }
        Some(patch)
    }
}

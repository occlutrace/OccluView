use crate::bbox::Aabb;

use glam::Vec3;

use super::{Camera, CameraProjection, MIN_ORTHOGRAPHIC_HEIGHT_MM};

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            // Looking slightly down from above — the occlusal bias.
            pitch: 1.0,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 100.0,
            fovy: 45.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        }
    }
}

impl Camera {
    /// Refit near/far planes around a visible scene box for the current view.
    ///
    /// Orthographic CAD inspection should not clip close geometry just because
    /// the target was panned or picked onto a surface, or because the eye
    /// orbited close to a wide multi-object scene. We bracket the box along the
    /// camera forward axis with a scene-proportional margin.
    ///
    /// Orthographic depth is **linear and uniform**, so the near plane may sit
    /// *behind* the eye (a negative near) and that is perfectly valid. The old
    /// implementation clamped near to a small positive value; with a wide scene
    /// viewed from a close orbit radius the nearest corners fall behind the eye
    /// (`min_depth < 0`), so that clamp planted an invisible clip plane in front
    /// of the camera and ate every object whose view-depth was smaller than it.
    pub fn fit_clip_planes_to_bbox(&mut self, bbox: Aabb) {
        if bbox.is_empty() {
            return;
        }

        let eye = self.eye();
        let forward = self.view_direction();
        if !eye.is_finite() || forward.length_squared() <= f32::EPSILON {
            return;
        }

        let mut min_depth = f32::INFINITY;
        let mut max_depth = f32::NEG_INFINITY;
        for corner in bbox_corners(bbox) {
            let depth = (corner - eye).dot(forward);
            min_depth = min_depth.min(depth);
            max_depth = max_depth.max(depth);
        }

        if !min_depth.is_finite() || !max_depth.is_finite() {
            return;
        }

        // Margin scales with the box (never an absolute crutch); the 1 mm floor
        // only guards a degenerate zero-size box so near stays strictly < far.
        let bbox_radius = (bbox.size().length() * 0.5).max(1.0);
        let padding = (bbox_radius * 0.35).max(1.0);

        match self.projection {
            // Bracket the box symmetrically. `near` may be negative — do NOT
            // clamp it: for the linear ortho depth that only clips the scene.
            // A future perspective arm MUST clamp near > 0 instead (the match
            // is exhaustive, so adding a variant forces that decision here).
            CameraProjection::Orthographic => {
                self.near = min_depth - padding;
                self.far = (max_depth + padding).max(self.near + padding.max(10.0));
            }
        }
    }

    /// Frame a bounding box with the **occlusal default** orientation.
    ///
    /// The occlusal view looks down the mesh's vertical (Y) axis onto the XZ
    /// plane, which corresponds to the chewing surface for a dental arch lying
    /// in XZ. For arbitrary meshes, this remains a conservative default.
    #[must_use]
    pub fn frame_occlusal(mut self, bbox: Aabb, fovy: f32) -> Self {
        if bbox.is_empty() {
            return self;
        }
        let center = bbox.center();
        let size = bbox.size();
        // Half-extent of the bbox in the occlusal (XZ) plane.
        let planar_half = 0.5_f32 * size.x.max(size.z);
        // Vertical half-extent — keep the arch depth in view too.
        let vertical_half = 0.5_f32 * size.y;
        let radius = planar_half.hypot(vertical_half).max(1.0);

        // Place the camera above, looking down at the occlusal plane.
        self.target = center;
        self.set_yaw_pitch(0.0, 0.6); // ~34° from horizontal: occlusal bias, not straight down
        self.projection = CameraProjection::Orthographic;
        self.fovy = fovy;
        self.orthographic_height = (radius * 2.0 / 0.7).max(MIN_ORTHOGRAPHIC_HEIGHT_MM);
        // Fit so the bbox radius fills ~70% of the half-FOV.
        let half_fov = 0.5 * fovy;
        self.distance = if half_fov > 1e-5 {
            radius / half_fov.tan() / 0.7
        } else {
            radius * 2.0
        };
        self.fit_clip_planes_to_bbox(bbox);
        self
    }
}

fn bbox_corners(bbox: Aabb) -> [Vec3; 8] {
    let min = bbox.min;
    let max = bbox.max;
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

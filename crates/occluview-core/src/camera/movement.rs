use glam::Vec2;

use super::{Camera, MAX_ORTHOGRAPHIC_HEIGHT_MM, MIN_ORTHOGRAPHIC_HEIGHT_MM};

impl Camera {
    /// Scale the camera distance and clip planes by a multiplicative factor.
    ///
    /// The height is clamped on BOTH sides: without the ceiling, a few hundred
    /// zoom-out wheel notches overflow `orthographic_height` to infinity, the
    /// GPU projection matrix turns NaN, and a subsequent pan NaN-poisons the
    /// target — an unrecoverable blank viewport. The clamp also heals a camera
    /// that already carries a non-finite height from legacy state.
    pub fn zoom_by(&mut self, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let next = self.orthographic_height * scale;
        self.orthographic_height = if next.is_finite() {
            next.clamp(MIN_ORTHOGRAPHIC_HEIGHT_MM, MAX_ORTHOGRAPHIC_HEIGHT_MM)
        } else {
            MAX_ORTHOGRAPHIC_HEIGHT_MM
        };
    }

    /// Zoom around a screen point instead of the camera target.
    ///
    /// Orthographic zoom changes the view-plane scale but does not move the
    /// eye, so the point under the cursor would otherwise slide toward the
    /// viewport centre. Move the target by the exact difference between the
    /// old and new view-plane offsets; the world point under the cursor then
    /// stays under it for both zoom-in and zoom-out.
    pub fn zoom_at_screen_point(
        &mut self,
        scale: f32,
        pointer_px: Vec2,
        viewport_px: Vec2,
    ) -> bool {
        if !scale.is_finite()
            || scale <= 0.0
            || !pointer_px.is_finite()
            || !viewport_px.is_finite()
            || viewport_px.x <= f32::EPSILON
            || viewport_px.y <= f32::EPSILON
        {
            return false;
        }

        let old_height = self.orthographic_height;
        let target_before = self.target;
        self.zoom_by(scale);
        let new_height = self.orthographic_height;
        if !old_height.is_finite()
            || !new_height.is_finite()
            || (new_height - old_height).abs() <= f32::EPSILON
        {
            return new_height != old_height;
        }

        let Some((right, up, _forward)) = self.view_basis() else {
            return new_height != old_height;
        };
        if !target_before.is_finite() {
            return new_height != old_height;
        }

        // Normalized screen coordinates use +Y up, matching the camera's
        // view-plane basis rather than egui's downward screen Y.
        let ndc = Vec2::new(
            pointer_px.x / viewport_px.x * 2.0 - 1.0,
            1.0 - pointer_px.y / viewport_px.y * 2.0,
        );
        let old_half_height = old_height * 0.5;
        let new_half_height = new_height * 0.5;
        let old_half_width = old_half_height * viewport_px.x / viewport_px.y;
        let new_half_width = new_half_height * viewport_px.x / viewport_px.y;
        let target_delta = right * (ndc.x * (old_half_width - new_half_width))
            + up * (ndc.y * (old_half_height - new_half_height));
        if target_delta.is_finite() {
            self.target += target_delta;
        }

        new_height != old_height
    }

    /// Pan the camera target in the current view plane using screen-space
    /// pixels. Positive X/Y deltas match pointer movement directions.
    pub fn pan_screen(&mut self, delta_px: Vec2, viewport_px: Vec2) {
        let viewport_height = viewport_px.y.max(1.0);
        if !delta_px.is_finite() || !viewport_px.is_finite() {
            return;
        }
        // A poisoned height must not spread: panning multiplies it into the
        // target, which would NaN-poison the camera permanently.
        if !self.orthographic_height.is_finite() {
            return;
        }

        let Some((right, up, _forward)) = self.view_basis() else {
            return;
        };

        let world_per_pixel = self.orthographic_height / viewport_height;
        self.target += (-delta_px.x * world_per_pixel) * right;
        self.target += (delta_px.y * world_per_pixel) * up;
    }
}

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

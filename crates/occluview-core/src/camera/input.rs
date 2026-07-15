use glam::Vec2;

/// Shared CAD orbit gain for raw pointer motion in the app and shell preview.
///
/// Owner-tuned for a crisp, unbraked feel: dragging half the viewport's
/// smaller dimension turns the model ~172° (the exocad "half screen, half
/// turn" rule). Retune only the magnitude — the drag direction is pinned by
/// `orbit_gain_scales_speed_without_flipping_direction`.
pub const CAD_ORBIT_DRAG_GAIN: f32 = 3.0;

/// Shared CAD wheel-zoom sensitivity (per scroll point) for the app viewport
/// and the shell preview. One Windows wheel notch (120–150 points) zooms
/// ~1.3–1.4×; scroll up always zooms in. Retune only the magnitude — the
/// direction is pinned by `zoom_direction_is_pinned_scroll_up_zooms_in`.
pub const CAD_ZOOM_SCROLL_SENSITIVITY: f32 = 0.0024;

/// Convert a wheel scroll (in points, + = scroll up) into a multiplicative
/// zoom factor for `Camera::zoom_by`: `< 1` shrinks the orthographic height
/// (zoom in), `> 1` grows it (zoom out).
pub fn zoom_factor_from_scroll(scroll_y: f32) -> f32 {
    if scroll_y.abs() <= f32::EPSILON {
        return 1.0;
    }
    (-scroll_y * CAD_ZOOM_SCROLL_SENSITIVITY).exp()
}

/// Convert raw pointer motion in viewport pixels into view-relative CAD orbit.
///
/// The input is a per-frame mouse delta, not an absolute cursor position. That
/// keeps rotation continuous even when the physical cursor reaches a window or
/// monitor edge.
pub fn orbit_delta_from_pointer_motion(delta_px: Vec2, viewport_size_px: Vec2) -> Option<Vec2> {
    if !delta_px.is_finite() || !viewport_size_px.is_finite() {
        return None;
    }
    let radius = viewport_size_px.x.min(viewport_size_px.y) * 0.5;
    if radius <= f32::EPSILON {
        return None;
    }

    let scaled = Vec2::new(delta_px.x / radius, delta_px.y / radius) * CAD_ORBIT_DRAG_GAIN;
    (scaled.length_squared() > f32::EPSILON).then_some(Vec2::new(-scaled.x, scaled.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner retunes gain for feel; the sign convention must never move.
    /// A rightward drag yields a negative yaw delta, a downward drag a
    /// positive pitch delta — flipping either inverts on-screen rotation.
    #[test]
    fn orbit_gain_scales_speed_without_flipping_direction() {
        let viewport = Vec2::new(800.0, 600.0);
        let right = orbit_delta_from_pointer_motion(Vec2::new(120.0, 0.0), viewport);
        let down = orbit_delta_from_pointer_motion(Vec2::new(0.0, 120.0), viewport);
        assert!(right.is_some() && down.is_some(), "drags must map");
        let right = right.unwrap_or(Vec2::ZERO);
        let down = down.unwrap_or(Vec2::ZERO);

        assert!(right.x < 0.0, "rightward drag flipped: {right}");
        assert!(down.y > 0.0, "downward drag flipped: {down}");

        // The gain must stay responsive: half the smaller viewport dimension
        // (300 px here) must turn at least a half circle's worth of yaw.
        let half_min =
            orbit_delta_from_pointer_motion(Vec2::new(300.0, 0.0), viewport).unwrap_or(Vec2::ZERO);
        assert!(
            half_min.x.abs() >= core::f32::consts::PI * 0.9,
            "orbit feels braked again: {} rad for a half-viewport drag",
            half_min.x.abs()
        );
    }

    /// Scroll up (positive points) must always zoom IN (factor < 1 shrinks
    /// the orthographic height); scroll down zooms out. Pinned so sensitivity
    /// retunes can never reverse the wheel.
    #[test]
    fn zoom_direction_is_pinned_scroll_up_zooms_in() {
        assert!((zoom_factor_from_scroll(0.0) - 1.0).abs() < 1e-6);
        assert!(
            zoom_factor_from_scroll(120.0) < 1.0,
            "scroll up must zoom in"
        );
        assert!(
            zoom_factor_from_scroll(-120.0) > 1.0,
            "scroll down must zoom out"
        );

        // One Windows wheel notch must feel decisive, not damped: at least
        // ~25% closer per 120-point notch, but never a jarring 2x jump.
        let per_notch = 1.0 / zoom_factor_from_scroll(120.0);
        assert!(
            (1.25..2.0).contains(&per_notch),
            "per-notch zoom drifted out of the crisp band: {per_notch}"
        );
    }
}

use eframe::egui;

pub(crate) const DEFAULT_RENDER_EXTENT_PX: [u16; 2] = [768, 512];

const MIN_RENDER_SIZE_PX: u16 = 256;
const MAX_RENDER_SIZE_PX: u16 = 2560;
const RENDER_SIZE_UPDATE_THRESHOLD_PX: u16 = 32;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn desired_render_extent_px(
    viewport_points: egui::Vec2,
    pixels_per_point: f32,
) -> Option<[u16; 2]> {
    if !viewport_points.is_finite() || !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }
    let width_px = viewport_points.x * pixels_per_point;
    let height_px = viewport_points.y * pixels_per_point;
    if width_px <= 0.0 || height_px <= 0.0 {
        return None;
    }
    Some([
        clamped_render_dimension_px(width_px),
        clamped_render_dimension_px(height_px),
    ])
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_render_dimension_px(px: f32) -> u16 {
    px.round()
        .clamp(f32::from(MIN_RENDER_SIZE_PX), f32::from(MAX_RENDER_SIZE_PX)) as u16
}

pub(crate) fn render_extent_change_requires_rerender(current: [u16; 2], desired: [u16; 2]) -> bool {
    current[0].abs_diff(desired[0]) >= RENDER_SIZE_UPDATE_THRESHOLD_PX
        || current[1].abs_diff(desired[1]) >= RENDER_SIZE_UPDATE_THRESHOLD_PX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desired_render_extent_clamps_to_reasonable_bounds() {
        assert_eq!(
            desired_render_extent_px(egui::vec2(120.0, 180.0), 1.0),
            Some([256, 256])
        );
        assert_eq!(
            desired_render_extent_px(egui::vec2(3200.0, 1800.0), 1.0),
            Some([2560, 1800])
        );
        assert_eq!(
            desired_render_extent_px(egui::vec2(400.0, 500.0), 2.0),
            Some([800, 1000])
        );
    }

    #[test]
    fn render_extent_change_uses_threshold() {
        assert!(!render_extent_change_requires_rerender(
            [512, 384],
            [530, 400]
        ));
        assert!(render_extent_change_requires_rerender(
            [512, 384],
            [544, 400]
        ));
        assert!(render_extent_change_requires_rerender(
            [512, 384],
            [530, 416]
        ));
    }
}

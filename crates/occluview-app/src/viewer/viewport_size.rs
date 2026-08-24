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
    // The factors are finite, but their product can still overflow to
    // infinity, and infinity would ride the shared scale below into a NaN
    // and out the far end as a zero-size render target.
    if !width_px.is_finite() || !height_px.is_finite() || width_px <= 0.0 || height_px <= 0.0 {
        return None;
    }
    Some(fitted_render_extent_px(width_px, height_px))
}

/// Scale both axes uniformly to preserve the viewport aspect ratio.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fitted_render_extent_px(width_px: f32, height_px: f32) -> [u16; 2] {
    let ceiling = f32::from(MAX_RENDER_SIZE_PX);
    let floor = f32::from(MIN_RENDER_SIZE_PX);
    let shrink = (ceiling / width_px).min(ceiling / height_px).min(1.0);
    let grow = (floor / width_px).max(floor / height_px).max(1.0);
    // Growing wins: an axis under the floor is the degenerate case, and when
    // growth pushes the long axis past the ceiling the final per-axis clamp
    // below lands it on the same bounds shrinking would have chosen.
    let scale = if grow > 1.0 { grow } else { shrink };
    // The exception, and it is the only one: a viewport longer than the
    // ceiling-to-floor ratio cannot satisfy both bounds at any single scale.
    // Squaring off the offending axis is the lesser evil there — the
    // alternative is a render target with a useless dimension.
    [
        clamped_render_dimension_px(width_px * scale),
        clamped_render_dimension_px(height_px * scale),
    ]
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
        // Both of the clamped cases keep the viewport's shape. They used to be
        // [256, 256] and [2560, 1800] — a 2:3 viewport rendered square and a
        // 16:9 one rendered at 1.42:1, which is the stretch this file exists to
        // prevent.
        assert_eq!(
            desired_render_extent_px(egui::vec2(120.0, 180.0), 1.0),
            Some([256, 384])
        );
        assert_eq!(
            desired_render_extent_px(egui::vec2(3200.0, 1800.0), 1.0),
            Some([2560, 1440])
        );
        assert_eq!(
            desired_render_extent_px(egui::vec2(400.0, 500.0), 2.0),
            Some([800, 1000])
        );
    }

    #[test]
    fn a_fullscreen_4k_viewport_keeps_its_shape() {
        // The reported bug: models distorted on a 4K display, seen in
        // fullscreen, because that is where the width first passes the
        // 2560 ceiling while the height does not. Clamped per axis that gave a
        // 2560x2160 target painted across a 3840x2160 rect — every model half
        // again too wide.
        //
        // Both ways a 4K viewport arrives: as raw pixels, and as points at the
        // 1.5x scaling Windows sets on a 4K panel by default.
        for (points, scale) in [
            (egui::vec2(3840.0, 2160.0), 1.0),
            (egui::vec2(2560.0, 1440.0), 1.5),
        ] {
            let [width, height] = desired_render_extent_px(points, scale).unwrap_or([0, 0]);
            assert!(
                width > 0 && height > 0,
                "a 4K viewport must have a render extent"
            );
            let asked = f64::from(points.x) / f64::from(points.y);
            let got = f64::from(width) / f64::from(height);
            assert!(
                (asked - got).abs() < 0.01,
                "a {points:?} viewport at {scale}x rendered at {width}x{height}: \
                 {got:.3} against the {asked:.3} it is painted into"
            );
            assert!(
                width <= 2560 && height <= 2560,
                "{width}x{height} is over the ceiling"
            );
        }
    }

    /// The widest viewport the bounds can express without changing its shape:
    /// the ceiling over the floor, so one axis can sit on each.
    const EXPRESSIBLE_ASPECT: f64 = MAX_RENDER_SIZE_PX as f64 / MIN_RENDER_SIZE_PX as f64;

    #[test]
    fn every_viewport_shape_survives_the_bounds() {
        // The property behind both tests above, swept over the range a desktop
        // window can actually take: whatever shape is asked for is the shape
        // that comes back, inside the bounds.
        for width in [300.0_f32, 800.0, 1920.0, 2560.0, 3840.0, 5120.0] {
            for height in [300.0_f32, 600.0, 1080.0, 1440.0, 2160.0] {
                let asked = f64::from(width) / f64::from(height);
                if !(1.0 / EXPRESSIBLE_ASPECT..=EXPRESSIBLE_ASPECT).contains(&asked) {
                    continue; // its own case, below
                }
                let points = egui::vec2(width, height);
                let [got_width, got_height] =
                    desired_render_extent_px(points, 1.0).unwrap_or([0, 0]);
                assert!(
                    got_width > 0 && got_height > 0,
                    "{points:?} must have a render extent"
                );
                let got = f64::from(got_width) / f64::from(got_height);
                assert!(
                    (asked - got).abs() / asked < 0.02,
                    "{width}x{height} rendered at {got_width}x{got_height}: \
                     {got:.3} against {asked:.3}"
                );
                assert!(
                    (256..=2560).contains(&got_width) && (256..=2560).contains(&got_height),
                    "{got_width}x{got_height} left the bounds"
                );
            }
        }
    }

    #[test]
    fn a_viewport_too_long_for_the_bounds_lands_on_them_rather_than_degenerating() {
        // A window dragged very short is longer than the ceiling and the floor
        // can express between them, so its shape cannot be kept. Say what
        // happens instead of leaving it to be discovered: both axes land on a
        // bound, which is a mild stretch, and not the zero-height render target
        // the alternative would be.
        let [width, height] =
            desired_render_extent_px(egui::vec2(3840.0, 300.0), 1.0).unwrap_or([0, 0]);

        assert_eq!([width, height], [MAX_RENDER_SIZE_PX, MIN_RENDER_SIZE_PX]);
        assert!(
            f64::from(3840.0_f32 / 300.0) > EXPRESSIBLE_ASPECT,
            "this fixture is only interesting while it is past what the bounds can express"
        );
    }

    #[test]
    fn an_overflowing_product_is_refused_rather_than_a_zero_size_target() {
        // Finite times finite can still be infinite, and infinity used to
        // ride the shared scale into a NaN and out the far end as a
        // zero-size texture.
        assert_eq!(
            desired_render_extent_px(egui::vec2(f32::MAX, 1080.0), 2.0),
            None
        );
        assert_eq!(
            desired_render_extent_px(egui::vec2(f32::MAX, f32::MAX), 2.0),
            None
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

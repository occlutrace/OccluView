use super::cursor::{sculpt_cursor_color, sculpt_cursor_linear_rgba};
use crate::sculpt::sculpt_kernel::BrushMode;
use eframe::egui;


#[test]
fn cursor_palette_is_published_to_the_linear_gpu_uniform() {
    // Every mode is washed 75 % toward white, so even the darkest ink
    // (Smooth) leaves a pale mark that only tints the lit surface.
    let color = sculpt_cursor_linear_rgba(sculpt_cursor_color(BrushMode::Smooth));
    assert!((color[0] - 0.761).abs() < 0.002, "got {}", color[0]);
    assert!((color[1] - 0.764).abs() < 0.002, "got {}", color[1]);
    assert!((color[2] - 0.766).abs() < 0.002, "got {}", color[2]);
    assert!((color[3] - 1.0).abs() < f32::EPSILON);
}

/// Channel indices ordered from smallest to largest.
fn channel_order(color: [f32; 3]) -> [usize; 3] {
    let mut indices = [0, 1, 2];
    indices.sort_by(|left, right| color[*left].total_cmp(&color[*right]));
    indices
}

#[test]
fn every_cursor_colour_is_washed_and_still_names_its_mode() {
    for mode in [BrushMode::Add, BrushMode::Remove, BrushMode::Smooth] {
        let token = egui::Rgba::from(sculpt_cursor_color(mode));
        let raw = [token.r(), token.g(), token.b()];
        let washed = sculpt_cursor_linear_rgba(sculpt_cursor_color(mode));
        for (index, channel) in raw.iter().enumerate() {
            let expected = channel + (1.0 - channel) * 0.75;
            assert!(
                (washed[index] - expected).abs() < 1e-6,
                "the wash is three quarters toward white: {} vs {expected}",
                washed[index]
            );
            assert!(
                washed[index] > 0.7,
                "a washed channel must stay pale, got {}",
                washed[index]
            );
        }
        // The hue survives: a washed colour keeps its token's channel order,
        // so green still reads Add, red Remove and grey Smooth.
        assert_eq!(
            channel_order([washed[0], washed[1], washed[2]]),
            channel_order(raw),
            "the wash keeps the mode's channel order"
        );
    }
}

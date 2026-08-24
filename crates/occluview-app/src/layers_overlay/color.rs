use eframe::egui;

/// The swatch colour for a layer tint.
///
/// A tint is multiplied into the shaded colour in the renderer's own space,
/// and nothing encodes on the way out of the shader, so the number the layer
/// carries is the number that reaches the screen. Measured on a white
/// triangle: the tint [0.03, 0.15, 0.79] renders as (12, 42, 200).
///
/// Run through a linear-to-sRGB transfer first, that tint draws as
/// (48, 108, 230): seventy levels between the colour beside the name and the
/// colour in the viewport, on the preset the palette leads with. The swatch is
/// the only place the colour is ever labelled, so it shows what the viewport
/// draws.
pub(super) fn color32_from_tint(color: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        unit_float_to_u8(color[0]),
        unit_float_to_u8(color[1]),
        unit_float_to_u8(color[2]),
        unit_float_to_u8(color[3]),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::color32_from_tint;

    #[test]
    fn the_swatch_shows_the_number_the_renderer_receives() {
        // The renderer's own test pins the other end: the same tint reaches
        // the screen as (12, 42, 200).
        let swatch = color32_from_tint([0.03, 0.15, 0.79, 1.0]);
        assert_eq!(
            [swatch.r(), swatch.g(), swatch.b()],
            [8, 38, 201],
            "the swatch must be the tint itself, rounded to bytes"
        );
    }

    #[test]
    fn full_and_empty_channels_survive_the_rounding() {
        let white = color32_from_tint([1.0, 1.0, 1.0, 1.0]);
        assert_eq!([white.r(), white.g(), white.b(), white.a()], [255; 4]);
        let black = color32_from_tint([0.0, 0.0, 0.0, 1.0]);
        assert_eq!([black.r(), black.g(), black.b()], [0, 0, 0]);
    }
}

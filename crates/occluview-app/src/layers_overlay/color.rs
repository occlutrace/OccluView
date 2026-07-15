use eframe::egui;

pub(super) fn color32_from_linear(color: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        linear_channel_to_srgb_u8(color[0]),
        linear_channel_to_srgb_u8(color[1]),
        linear_channel_to_srgb_u8(color[2]),
        unit_float_to_u8(color[3]),
    )
}

fn linear_channel_to_srgb_u8(channel: f32) -> u8 {
    let channel = channel.clamp(0.0, 1.0);
    let srgb = if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    unit_float_to_u8(srgb)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

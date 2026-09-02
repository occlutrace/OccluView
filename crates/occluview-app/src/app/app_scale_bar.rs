use super::{egui, Camera, ScaleBar};
use crate::app_settings::{UnitDisplay, ViewportBackground};

/// Draw the scale bar for what is on screen right now.
///
/// The scale comes from the camera, not from the scene's size: an orthographic
/// view puts `orthographic_height / viewport_height` millimetres in a pixel, and
/// that is the only number the bar can honestly be built from. It used to be
/// derived from the mesh's bounding box, so the bar was right for the first frame
/// after a file opened and wrong from the first scroll onwards.
pub(super) fn paint_scale_bar(
    ui: &egui::Ui,
    image_rect: egui::Rect,
    camera: &Camera,
    unit: UnitDisplay,
    background: ViewportBackground,
) {
    let mm_per_px =
        crate::align_drag::mm_per_pixel(camera.orthographic_height, image_rect.height());
    let Some(bar) = ScaleBar::for_mm_per_px(mm_per_px) else {
        return;
    };

    let margin = 16.0;
    let max_width = image_rect.width() - margin * 2.0;
    if max_width < 64.0 || bar.width_px > max_width {
        return;
    }

    let x0 = image_rect.left() + margin;
    let x1 = x0 + bar.width_px;
    let y = image_rect.bottom() - margin;
    let tick = 6.0;
    let painter = ui.painter();
    // Ink follows the render's background setting, not the chrome theme: a
    // dark theme over the default gray render still needs dark ink.
    let viewport_is_dark = background.is_dark();
    let shadow = egui::Stroke::new(
        4.0_f32,
        crate::ui_theme::viewport_ink_halo(viewport_is_dark),
    );
    let line = egui::Stroke::new(2.0_f32, crate::ui_theme::viewport_ink(viewport_is_dark));
    for stroke in [shadow, line] {
        painter.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], stroke);
        painter.line_segment([egui::pos2(x0, y - tick), egui::pos2(x0, y + tick)], stroke);
        painter.line_segment([egui::pos2(x1, y - tick), egui::pos2(x1, y + tick)], stroke);
    }
    painter.text(
        egui::pos2(x0, y - 22.0),
        egui::Align2::LEFT_TOP,
        bar.label(unit),
        egui::FontId::proportional(13.0),
        crate::ui_theme::viewport_ink(viewport_is_dark),
    );
}

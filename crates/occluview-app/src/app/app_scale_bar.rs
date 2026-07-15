use super::{egui, ScaleBar, SceneStats};

pub(super) fn paint_scale_bar(ui: &egui::Ui, image_rect: egui::Rect, stats: SceneStats) {
    let scene_width_mm = stats.bbox_mm[0].max(stats.bbox_mm[2]);
    let Some(bar) = ScaleBar::for_viewport(scene_width_mm, image_rect.width()) else {
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
    let shadow = egui::Stroke::new(
        4.0,
        egui::Color32::from_rgba_unmultiplied(248, 250, 252, 190),
    );
    let line = egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(15, 23, 42, 210));
    for stroke in [shadow, line] {
        painter.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], stroke);
        painter.line_segment([egui::pos2(x0, y - tick), egui::pos2(x0, y + tick)], stroke);
        painter.line_segment([egui::pos2(x1, y - tick), egui::pos2(x1, y + tick)], stroke);
    }
    painter.text(
        egui::pos2(x0, y - 22.0),
        egui::Align2::LEFT_TOP,
        bar.label(),
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgb(15, 23, 42),
    );
}

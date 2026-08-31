//! Shared icon-button control for mesh and alignment tools.

use eframe::egui::{self, FontId, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::icons::AppIcon;
use crate::ui_theme::ACCENT;

/// Shared corner radius for tool cells and commit buttons.
pub(crate) const CELL_ROUNDING: f32 = 4.0;

/// Draw an icon tile with a caption.
#[allow(clippy::too_many_arguments)]
pub(crate) fn icon_button(
    ui: &mut Ui,
    size: Vec2,
    icon: AppIcon,
    label: &str,
    tooltip: &str,
    enabled: bool,
    active: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    let fg = if !enabled {
        ui.visuals().weak_text_color()
    } else if active {
        ACCENT
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };

    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CELL_ROUNDING, ACCENT.gamma_multiply(0.20));
        painter.rect_stroke(
            rect,
            CELL_ROUNDING,
            egui::Stroke::new(1.2_f32, ACCENT.gamma_multiply(0.90)),
            egui::StrokeKind::Middle,
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, CELL_ROUNDING, ACCENT.gamma_multiply(0.12));
        painter.rect_stroke(
            rect,
            CELL_ROUNDING,
            egui::Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.30)),
            egui::StrokeKind::Middle,
        );
    }

    // Reserve the bottom strip for the caption; the glyph takes what is left.
    let caption_h = 22.0_f32;
    let icon_side = (rect.width().min(rect.height() - caption_h) - 8.0).clamp(14.0, 24.0);
    let icon_center = Pos2::new(rect.center().x, rect.top() + 4.0 + icon_side * 0.5);
    let icon_rect = Rect::from_center_size(icon_center, Vec2::splat(icon_side));
    crate::icons::paint(painter, icon_rect, icon, fg);

    let galley = painter.layout(
        label.to_owned(),
        FontId::proportional(10.0),
        fg,
        rect.width() - 2.0,
    );
    let caption_pos = Pos2::new(
        rect.center().x - galley.size().x * 0.5,
        rect.bottom() - caption_h + (caption_h - galley.size().y) * 0.5,
    );
    painter.galley(caption_pos, galley, fg);

    response.on_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The editor cells the two panels actually place.
    const CELL_ICONS: [AppIcon; 16] = [
        AppIcon::Lasso,
        AppIcon::Object,
        AppIcon::SelectAll,
        AppIcon::SelectNone,
        AppIcon::SelectInvert,
        AppIcon::Delete,
        AppIcon::Keep,
        AppIcon::CloseHoles,
        AppIcon::Smooth,
        AppIcon::SculptAdd,
        AppIcon::Cut,
        AppIcon::Separate,
        AppIcon::Undo,
        AppIcon::Redo,
        AppIcon::AlignFit,
        AppIcon::AlignRefine,
    ];

    #[test]
    fn icon_button_renders_in_every_state_without_panicking() {
        egui::__run_test_ui(|ui| {
            for icon in CELL_ICONS {
                for enabled in [false, true] {
                    for active in [false, true] {
                        icon_button(
                            ui,
                            Vec2::new(48.0, 50.0),
                            icon,
                            "Label",
                            "tooltip",
                            enabled,
                            active,
                        );
                    }
                }
            }
        });
    }
}

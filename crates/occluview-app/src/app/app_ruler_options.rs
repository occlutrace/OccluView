//! The Ruler's options strip: how a ruler that ends on another ruler's line
//! meets it.
//!
//! The choice is a durable setting (also in Settings, beside the unit), shown
//! here while the Ruler is armed so it can be changed where it is used. Shift
//! held selects the other choice for as long as it is held, and the strip
//! lights the choice that is live, so the operator sees what a click will do.

use super::{egui, layers_overlay, OccluViewApp};
use crate::app_settings::RulerLineAngle;
use crate::icons::AppIcon;
use crate::measure_overlay::ruler_line_angle_key;
use crate::measure_tool::MeasureMode;
use crate::ui_theme;

const STRIP_ID: &str = "ruler-options-strip";
/// Gap between the strip and the viewport's top edge, the Layers panel, or
/// the contact bar above it.
const STRIP_GAP_PX: f32 = 10.0;
/// Strip width before its first layout has measured it.
const STRIP_FIRST_WIDTH_PX: f32 = 420.0;
/// Space a chip keeps around its glyph and label.
const CHIP_PADDING_PX: f32 = 22.0;
/// Glyph size plus the gap to the label, as `align_panel::chip` draws it.
const CHIP_GLYPH_PX: f32 = 20.0;

impl OccluViewApp {
    /// The choice a ruler ending on a line uses right now: the setting, or the
    /// other one while Shift is held.
    pub(super) fn ruler_line_angle(&self, ctx: &egui::Context) -> RulerLineAngle {
        let chosen = self.persistence.settings.ruler_line_angle;
        if ctx.input(|input| input.modifiers.shift) {
            chosen.other()
        } else {
            chosen
        }
    }

    /// Paint the strip while the Ruler is armed and apply a click on it. It is
    /// its own foreground layer, so a click on it never reaches the viewport.
    pub(super) fn show_ruler_options(&mut self, ctx: &egui::Context, viewport_rect: egui::Rect) {
        if self.tools.measure.mode() != Some(MeasureMode::Ruler) {
            return;
        }
        let live = self.ruler_line_angle(ctx);
        let id = egui::Id::new(STRIP_ID);
        let width = ctx
            .memory(|memory| memory.area_rect(id))
            .map_or(STRIP_FIRST_WIDTH_PX, |rect| rect.width());
        let origin = self.ruler_options_origin(viewport_rect, width);
        let locale = &self.ui.locale;
        let title = locale.tr("measure-line-angle");
        let hint = locale.tr("measure-line-angle-hint");
        let shift_note = locale.tr("measure-line-angle-shift");
        let mut chosen = None;
        egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(origin)
            .constrain_to(viewport_rect)
            .show(ctx, |ui| {
                ui_theme::overlay_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(
                            egui::RichText::new(&title)
                                .size(12.0)
                                .color(ui_theme::text_muted()),
                        )
                        .on_hover_text(&hint);
                        for option in RulerLineAngle::OPTIONS {
                            let label = locale.tr(ruler_line_angle_key(option));
                            let icon = match option {
                                RulerLineAngle::Free => AppIcon::AngleFree,
                                RulerLineAngle::Perpendicular => AppIcon::AngleRight,
                            };
                            let text_width = ui
                                .painter()
                                .layout_no_wrap(
                                    label.clone(),
                                    egui::FontId::proportional(11.5),
                                    ui_theme::text(),
                                )
                                .rect
                                .width();
                            let response = crate::align_panel::chip(
                                ui,
                                text_width + CHIP_GLYPH_PX + CHIP_PADDING_PX,
                                Some(icon),
                                &label,
                                true,
                                live == option,
                            )
                            .on_hover_text(&hint);
                            if response.clicked() {
                                chosen = Some(option);
                            }
                        }
                        ui_theme::vertical_divider(ui, 18.0);
                        shift_keycap(ui, live != self.persistence.settings.ruler_line_angle);
                        ui.label(
                            egui::RichText::new(&shift_note)
                                .size(11.5)
                                .color(ui_theme::text_weak()),
                        );
                    });
                });
            });
        if let Some(option) = chosen {
            if self.persistence.settings.ruler_line_angle != option {
                self.persistence.settings.ruler_line_angle = option;
                self.persistence.settings_persistence.mark_dirty();
            }
            ctx.request_repaint();
        }
    }

    /// Top-left corner for a strip `width` wide: centred over the viewport,
    /// clear of the Layers panel on the left and below the contact bar when
    /// that is open.
    fn ruler_options_origin(&self, viewport_rect: egui::Rect, width: f32) -> egui::Pos2 {
        let layer_count = self
            .document
            .scene
            .as_ref()
            .map_or(0, |scene| scene.meshes().len());
        let layers_right = (layer_count > 0).then(|| {
            layers_overlay::layer_overlay_rect(viewport_rect, layer_count).right() + STRIP_GAP_PX
        });
        let top = if self.tools.contacts.is_open() {
            super::app_contact_bar::contact_bar_rect(viewport_rect, layer_count).bottom()
                + STRIP_GAP_PX
        } else {
            viewport_rect.top() + STRIP_GAP_PX
        };
        strip_origin(viewport_rect, width, layers_right, top)
    }
}

/// Centre a strip `width` wide over `viewport_rect` at `top`, moved right to
/// start at `clear_of` when that is further right, and back inside the
/// viewport when the window is too narrow for both.
fn strip_origin(
    viewport_rect: egui::Rect,
    width: f32,
    clear_of: Option<f32>,
    top: f32,
) -> egui::Pos2 {
    let centred = viewport_rect.center().x - width * 0.5;
    let left = clear_of.map_or(centred, |clear| centred.max(clear));
    let left = left
        .min(viewport_rect.right() - STRIP_GAP_PX - width)
        .max(viewport_rect.left() + STRIP_GAP_PX);
    egui::pos2(left, top)
}

/// A small key cap reading "Shift", lit while Shift has switched the choice.
fn shift_keycap(ui: &mut egui::Ui, held: bool) {
    let font = egui::FontId::proportional(10.5);
    let ink = if held {
        ui_theme::accent()
    } else {
        ui_theme::text_muted()
    };
    let galley = ui.painter().layout_no_wrap("Shift".to_owned(), font, ink);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    if held {
        painter.rect_filled(rect, 3.0, ui_theme::accent().gamma_multiply(0.16));
    }
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0_f32, ink.gamma_multiply(0.6)),
        egui::StrokeKind::Middle,
    );
    painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(1200.0, 800.0))
    }

    #[test]
    fn the_strip_centres_over_the_viewport_when_nothing_is_in_the_way() {
        let origin = strip_origin(viewport(), 400.0, None, 50.0);
        assert!((origin.x - 400.0).abs() < 1.0e-3);
        assert!((origin.y - 50.0).abs() < 1.0e-3);
    }

    #[test]
    fn the_strip_moves_clear_of_the_layers_panel_and_stays_in_the_viewport() {
        let origin = strip_origin(viewport(), 400.0, Some(520.0), 50.0);
        assert!((origin.x - 520.0).abs() < 1.0e-3);
        let narrow = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(500.0, 600.0));
        let origin = strip_origin(narrow, 400.0, Some(330.0), 50.0);
        assert!(
            origin.x + 400.0 <= narrow.right(),
            "never past the right edge"
        );
        assert!(origin.x >= narrow.left());
    }

    #[test]
    fn shift_selects_the_other_choice() {
        assert_eq!(RulerLineAngle::Free.other(), RulerLineAngle::Perpendicular);
        assert_eq!(RulerLineAngle::Perpendicular.other(), RulerLineAngle::Free);
        assert_eq!(RulerLineAngle::default(), RulerLineAngle::Free);
    }
}

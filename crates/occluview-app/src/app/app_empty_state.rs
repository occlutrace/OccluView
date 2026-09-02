//! Empty-viewport surface and drag-hover feedback: what the operator sees
//! before any scene exists, and how a file drag over the window answers.

use super::{egui, OccluViewApp};

impl OccluViewApp {
    /// While files hover anywhere over the window, frame the viewport as the
    /// drop target and switch to the copy cursor, so a drag is answered with
    /// "this will open" before the operator lets go.
    pub(super) fn show_drop_hover_frame_if_hovering(
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
        ui.painter().rect_stroke(
            viewport_rect.shrink(4.0),
            10.0,
            egui::Stroke::new(2.0, crate::ui_theme::accent()),
            egui::StrokeKind::Outside,
        );
    }

    /// The viewport's empty state: a centered call to action over the bare
    /// clear color. Clicking it opens the same native dialog as the toolbar
    /// Open button; the Ctrl+O hint is real — it is the one wired shortcut.
    pub(super) fn show_empty_state(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        if response.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // A failed startup load can leave the error dialog up over the empty
        // viewport; the guard owns the pointer until it is dismissed.
        if !self.modal_dialog_open() && response.clicked() {
            self.open_dialog_requested = true;
            ctx.request_repaint();
        }
        let card_rect =
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(340.0, 120.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(card_rect), |ui| {
            ui.vertical_centered(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                crate::icons::paint(
                    ui.painter(),
                    icon_rect,
                    crate::icons::AppIcon::Open,
                    crate::ui_theme::text_weak(),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Open a 3D file")
                        .color(crate::ui_theme::text())
                        .size(14.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("STL · PLY · OBJ · GLB · HPS — or drop files here")
                        .color(crate::ui_theme::text_weak())
                        .size(11.5),
                );
                ui.label(
                    egui::RichText::new("Ctrl+O")
                        .color(crate::ui_theme::text_muted())
                        .size(10.5),
                );
            });
        });
    }
}

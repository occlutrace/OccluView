//! Discoverable keyboard and pointer controls.
//!
//! The Help surface is intentionally separate from the product-information
//! dialogs: it is an operator reference, not a change to About, Settings, or
//! any editing surface.

use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use crate::interaction_hints::{contextual_line, HintContext, ALL_SECTIONS};
use crate::measure_overlay::{toolbar_toggle, ToolbarToggle};
use crate::modal_surface::show_information_modal;
use crate::ui_theme;
use eframe::egui;

const HELP_ROW_HEIGHT: f32 = 25.0;
const HELP_GESTURE_WIDTH: f32 = 196.0;

pub(super) fn show_help_toolbar_toggle(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    toolbar_toggle(
        ui,
        ToolbarToggle::new(
            crate::icons::AppIcon::Licenses,
            "Help",
            enabled,
            false,
            "Show keyboard and mouse controls",
        ),
    )
}

impl OccluViewApp {
    pub(super) fn show_help_dialog(&mut self, ctx: &egui::Context) {
        if self.information_dialog != InformationDialog::KeyboardMouse {
            return;
        }

        let mut close = false;
        let modal_response = show_information_modal(
            ctx,
            egui::Id::new("occluview-keyboard-mouse-dialog-v1"),
            egui::vec2(700.0, 570.0),
            |ui| {
                ui.set_width(668.0_f32.min(ui.available_width()));
                ui.label(
                    egui::RichText::new("Keyboard and mouse controls")
                        .size(18.0)
                        .strong()
                        .color(ui_theme::text()),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(
                        "The reference below matches the controls currently available in OccluView.",
                    )
                    .size(11.5)
                    .color(ui_theme::text_weak()),
                );
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_salt("occluview-keyboard-mouse-sections")
                    .max_height(438.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for section in ALL_SECTIONS {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(section.title)
                                    .size(12.5)
                                    .strong()
                                    .color(ui_theme::text()),
                            );
                            ui.separator();
                            for row in section.rows {
                                let row_width = ui.available_width();
                                ui.allocate_ui_with_layout(
                                    egui::vec2(row_width, HELP_ROW_HEIGHT),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let gesture_width =
                                            HELP_GESTURE_WIDTH.min(row_width.max(0.0));
                                        ui.add_sized(
                                            egui::vec2(gesture_width, HELP_ROW_HEIGHT),
                                            egui::Label::new(
                                                egui::RichText::new(row.gesture)
                                                    .strong()
                                                    .color(ui_theme::text()),
                                            )
                                            .truncate(),
                                        );
                                        ui.add_space(12.0);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(row.action)
                                                    .color(ui_theme::text_weak()),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            }
                        }
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    },
                );
            },
        );

        if close || modal_response.should_close() {
            self.information_dialog = InformationDialog::None;
        }
    }

    pub(super) fn interaction_hint_context(&self) -> HintContext {
        if self.measure.is_active() {
            HintContext::Measure
        } else if self.cut_view.is_active() || self.bridge_split_active() {
            HintContext::Cut
        } else if self.align_active() {
            HintContext::Align
        } else if self.edit_mode.has_active_session() {
            match self.editor_tab {
                crate::mesh_editor_overlay::EditorTab::EditMesh => HintContext::MeshEditing,
                crate::mesh_editor_overlay::EditorTab::Sculpt => HintContext::Sculpt,
            }
        } else {
            HintContext::Navigation
        }
    }
}

pub(super) fn render_contextual_hint(
    ui: &mut egui::Ui,
    _rect: egui::Rect,
    context: HintContext,
    ink: egui::Color32,
) {
    let line = contextual_line(context);
    let response =
        ui.add(egui::Label::new(egui::RichText::new(line).color(ink).size(11.5)).truncate());
    response.on_hover_text(line);
}

#[cfg(test)]
mod tests {
    use super::super::information_dialog::InformationDialog;

    #[test]
    fn help_route_is_a_single_information_surface() {
        assert!(InformationDialog::KeyboardMouse.is_open());
    }
}

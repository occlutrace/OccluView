//! Discoverable keyboard and pointer controls.
//!
//! The Help surface is intentionally separate from the product-information
//! dialogs: it is an operator reference, not a change to About, Settings, or
//! any editing surface.

use super::information_dialog::InformationDialog;
use super::OccluViewApp;
use crate::interaction_hints::{contextual_line, contextual_line_key, HintContext, ALL_SECTIONS};

use crate::modal_surface::show_information_modal;
use crate::ui_theme;
use eframe::egui;

use crate::i18n::LocaleManager;

const HELP_ROW_HEIGHT: f32 = 25.0;
const HELP_GESTURE_WIDTH: f32 = 196.0;

impl OccluViewApp {
    pub(super) fn show_help_dialog(&mut self, ctx: &egui::Context) {
        if self.ui.information_dialog != InformationDialog::KeyboardMouse {
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
                    egui::RichText::new(self.ui.locale.text("help-title"))
                        .size(18.0)
                        .strong()
                        .color(ui_theme::text()),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(self.ui.locale.text("help-subtitle"))
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
                                egui::RichText::new(self.ui.locale.text(section.key))
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
                                                egui::RichText::new(
                                                    crate::i18n::platform_shortcut_text(
                                                        row.gesture,
                                                    ),
                                                )
                                                .strong()
                                                .color(ui_theme::text()),
                                            )
                                            .truncate(),
                                        );
                                        ui.add_space(12.0);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(self.ui.locale.text(row.key))
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
                        if ui.button(self.ui.locale.text("help-close")).clicked() {
                            close = true;
                        }
                    },
                );
            },
        );

        if close || modal_response.should_close() {
            self.ui.information_dialog = InformationDialog::None;
        }
    }

    pub(super) fn interaction_hint_context(&self) -> HintContext {
        if self.tools.measure.is_active() {
            HintContext::Measure
        } else if self.tools.cut_view.is_active() || self.tools.bridge_split_active() {
            HintContext::Cut
        } else if self.align_active() {
            HintContext::Align
        } else if self.document.edit_mode.has_active_session() {
            match self.tools.editor_tab {
                crate::mesh_editor_overlay::EditorTab::EditMesh => HintContext::MeshEditing,
                crate::mesh_editor_overlay::EditorTab::Sculpt => HintContext::Sculpt,
            }
        } else if self.tools.contacts.is_open() {
            // A reading is a tool the operator is in the middle of using, so its
            // gestures replace the plain navigation reminder until it closes.
            HintContext::Contacts
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
    locale: &LocaleManager,
) {
    let line = crate::i18n::platform_shortcut_text(contextual_line(context));
    let localized = locale.text(contextual_line_key(context));
    let response =
        ui.add(egui::Label::new(egui::RichText::new(localized).color(ink).size(11.5)).truncate());
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

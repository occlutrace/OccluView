//! Discoverable keyboard and pointer controls.
//!
//! The Help surface is intentionally separate from the product-information
//! dialogs: it is an operator reference, not a change to About, Settings, or
//! any editing surface.

use super::information_dialog::InformationDialog;
use super::SceneContext;
use crate::ui::interaction_hints::{
    contextual_line, contextual_line_key, ContextualHint, HintContext, ALL_SECTIONS,
};

use crate::ui::modal_surface::show_information_modal;
use crate::ui::ui_theme;
use eframe::egui;

use crate::i18n::LocaleManager;

const HELP_ROW_HEIGHT: f32 = 22.0;
const HELP_GESTURE_WIDTH: f32 = 172.0;

impl SceneContext<'_> {
    pub(super) fn show_help_dialog(&mut self, ctx: &egui::Context) {
        if self.ui.information_dialog != InformationDialog::KeyboardMouse {
            return;
        }

        let mut close = false;
        let close_label = self.ui.locale.tr(crate::i18n::message_id!("help-close"));
        let modal_response = show_information_modal(
            ctx,
            egui::Id::new("occluview-keyboard-mouse-dialog-v1"),
            egui::vec2(620.0, 370.0),
            &close_label,
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(
                            self.ui.locale.tr(crate::i18n::message_id!("help-title")),
                        )
                        .strong()
                        .size(18.0)
                        .color(ui_theme::text()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(&close_label).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                draw_help_basics(
                    ui,
                    &self.ui.locale,
                    self.persistence.settings.scroll_behavior,
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
                super::mesh_editor_overlay::EditorTab::EditMesh => HintContext::MeshEditing,
                super::mesh_editor_overlay::EditorTab::Sculpt => HintContext::Sculpt,
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

/// The quick reference uses the same gesture catalogue as the tool hints.
fn draw_help_basics(
    ui: &mut egui::Ui,
    locale: &LocaleManager,
    scroll_behavior: crate::app_settings::ScrollBehavior,
) {
    let catalogue = |gesture: &str| {
        ALL_SECTIONS
            .iter()
            .flat_map(|section| section.rows)
            .find(|row| row.gesture == gesture)
            .map(|row| locale.text(row.action_key(scroll_behavior)))
    };
    let rows = [
        ("Ctrl+O", catalogue("Ctrl+O").unwrap_or_default()),
        ("RMB drag", catalogue("RMB drag").unwrap_or_default()),
        ("MMB drag", catalogue("MMB drag").unwrap_or_default()),
        ("Wheel / pinch", catalogue("Wheel").unwrap_or_default()),
        ("MMB click", catalogue("MMB click").unwrap_or_default()),
        (
            "E / A / C",
            locale.tr(crate::i18n::message_id!("help-basic-tools")),
        ),
        ("Ctrl+Z", catalogue("Ctrl+Z").unwrap_or_default()),
        (
            "Ctrl+Y / Ctrl+Shift+Z",
            catalogue("Ctrl+Y / Ctrl+Shift+Z").unwrap_or_default(),
        ),
        (
            "Delete",
            catalogue("Delete / Backspace").unwrap_or_default(),
        ),
        (
            "1 / 2",
            locale.tr(crate::i18n::message_id!("help-basic-sculpt")),
        ),
        (
            "F6 / Shift+F6",
            catalogue("F6 / Shift+F6").unwrap_or_default(),
        ),
        (
            "MMB: scene tab",
            locale.tr(crate::i18n::message_id!("help-basic-close-scene")),
        ),
    ];
    let row_height = ((ui.ctx().content_rect().height() - 110.0) / rows.len() as f32 - 3.0)
        .clamp(18.0, HELP_ROW_HEIGHT);
    for (gesture, action) in rows {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_sized(
                    egui::vec2(HELP_GESTURE_WIDTH, row_height),
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::platform_shortcut_text(gesture))
                            .strong()
                            .size(11.5)
                            .color(ui_theme::text()),
                    )
                    .truncate(),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(action)
                            .size(12.0)
                            .color(ui_theme::text_weak()),
                    )
                    .wrap(),
                );
            },
        );
    }
}

pub(super) fn render_contextual_hint(
    ui: &mut egui::Ui,
    _rect: egui::Rect,
    hint: ContextualHint,
    ink: egui::Color32,
    locale: &LocaleManager,
) {
    let line =
        crate::i18n::platform_shortcut_text(contextual_line(hint.context, hint.scroll_behavior));
    let localized = locale.text(contextual_line_key(hint.context, hint.scroll_behavior));
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

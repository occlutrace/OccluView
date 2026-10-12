//! Unsaved-work guard dialogs (close + replace-open).
//!
//! Content only: dispatch stays with the app methods that open each guard
//! (`app_dialogs` owns the toolbar/dialog surface).

use eframe::egui;

use crate::ui::icons::AppIcon;
use crate::ui::ui_theme;

pub(super) struct GuardDialogSpec<'a> {
    pub(super) id: &'static str,
    pub(super) title: &'a str,
    pub(super) headline: &'a str,
    pub(super) note: Option<&'a str>,
    pub(super) detail: &'a str,
    pub(super) destructive_label: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GuardDialogAction {
    Save,
    Destructive,
    Cancel,
}

pub(super) struct GuardDialogResponse {
    pub(super) action: Option<GuardDialogAction>,
}

// Canonical button labels "Save…", "Discard and open", "Cancel" resolve
// through `guard-save`, `guard-*-destructive`, `guard-cancel`.
pub(super) fn show_guard_dialog(
    ctx: &egui::Context,
    locale: &crate::i18n::LocaleManager,
    spec: GuardDialogSpec<'_>,
) -> GuardDialogResponse {
    let mut action = None;
    let cancel = locale.tr(crate::i18n::message_id!("guard-cancel"));
    let modal = crate::ui::modal_surface::show_information_modal(
        ctx,
        egui::Id::new(spec.id),
        egui::vec2(480.0, 240.0),
        &cancel,
        |ui| {
            ui_theme::dialog_header(ui, spec.title, AppIcon::Warn);
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                crate::ui::icons::paint(
                    ui.painter(),
                    icon_rect,
                    AppIcon::Warn,
                    ui_theme::warning(),
                );
                ui.label(egui::RichText::new(spec.headline).strong());
            });
            if let Some(note) = spec.note {
                ui.label(note);
            }
            ui.label(
                egui::RichText::new(spec.detail)
                    .color(ui_theme::text_weak())
                    .size(12.0),
            );
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                |ui| {
                    if ui
                        .add(ui_theme::primary_button(
                            &locale.tr(crate::i18n::message_id!("guard-save")),
                        ))
                        .clicked()
                    {
                        action = Some(GuardDialogAction::Save);
                    }
                    if ui.button(spec.destructive_label).clicked() {
                        action = Some(GuardDialogAction::Destructive);
                    }
                    if ui.button(&cancel).clicked() {
                        action = Some(GuardDialogAction::Cancel);
                    }
                },
            );
        },
    );
    if modal.should_close() && action.is_none() {
        action = Some(GuardDialogAction::Cancel);
    }
    GuardDialogResponse { action }
}

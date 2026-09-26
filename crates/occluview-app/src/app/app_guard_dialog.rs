//! Unsaved-work guard dialogs (close + replace-open).
//!
//! Content only: dispatch stays with the app methods that open each guard
//! (`app_dialogs` owns the toolbar/dialog surface).

use eframe::egui;

use crate::icons::AppIcon;
use crate::ui_theme;

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
    const CONTENT_WIDTH: f32 = 416.0;
    let mut open = true;
    let mut action = None;
    egui::Window::new(spec.title)
        .id(egui::Id::new(spec.id))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .constrain_to(ctx.content_rect().shrink(8.0))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(CONTENT_WIDTH);
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), icon_rect, AppIcon::Warn, ui_theme::warning());
                ui.label(egui::RichText::new(spec.headline).strong());
            });
            if let Some(note) = spec.note {
                ui.label(note);
            }
            ui.label(egui::RichText::new(spec.detail).weak().size(11.0));
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(dialog_primary_button(&locale.tr("guard-save")))
                        .clicked()
                    {
                        action = Some(GuardDialogAction::Save);
                    }
                    if ui.button(spec.destructive_label).clicked() {
                        action = Some(GuardDialogAction::Destructive);
                    }
                    if ui.button(locale.tr("guard-cancel")).clicked() {
                        action = Some(GuardDialogAction::Cancel);
                    }
                },
            );
        });
    if !open && action.is_none() {
        action = Some(GuardDialogAction::Cancel);
    }
    GuardDialogResponse { action }
}

/// Primary dialog action.
fn dialog_primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(label)
            .strong()
            .color(ui_theme::on_accent()),
    )
    .fill(ui_theme::accent())
    .corner_radius(ui_theme::RADIUS_CONTROL)
}

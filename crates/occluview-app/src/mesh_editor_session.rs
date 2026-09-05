//! Mesh editor status line and session commit bar.
//!
//! Split from the tool palette (`mesh_editor_groups`) so both files stay
//! within the per-file line budget. Presentation only — actions flow back
//! as [`MeshEditorAction`].

use eframe::egui;

use super::{MeshEditorAction, MeshEditorPanelState};
use crate::icons::AppIcon;
use crate::ui_theme;

use super::groups::{icon, tall_text_button};

/// One dim line of operator context: the pending-edits marker (only when it has
/// something to say) and the interaction hint for the active selection mode. The
/// raw selected-face count is intentionally NOT shown — it is noise that just ate
/// an info line.
pub(super) fn status(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) {
    ui.add_space(3.0);
    if state.busy {
        ui.spinner();
    } else if state.dirty {
        ui.horizontal(|ui| {
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::vec2(13.0, 13.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), icon_rect, AppIcon::Warn, ui_theme::warning());
            ui.label(
                egui::RichText::new(locale.tr("meshedit-status-unsaved"))
                    .color(ui_theme::warning())
                    .size(11.0),
            );
        })
        .response
        .on_hover_text(locale.tr("meshedit-status-unsaved-hint"));
    }
    let hint = if state.sculpt_armed.is_some() {
        locale.tr("meshedit-status-hint-sculpt")
    } else if state.object_mode {
        locale.tr("meshedit-status-hint-object")
    } else if state.lasso_armed {
        locale.tr("meshedit-status-hint-lasso")
    } else {
        locale.tr("meshedit-status-hint-default")
    };
    ui.label(egui::RichText::new(hint).weak().size(10.0));
}

/// History and session boundary, laid out as an OK/Cancel bar matching the
/// dental CAD convention: Undo/Redo as light history cells on the left, then
/// `Cancel` and the accented `Done` pinned bottom-right. Done confirms and
/// dismisses; Cancel reverts to baseline.
pub(super) fn session(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        // History cluster (left).
        let history_w = 42.0;
        if icon(
            ui,
            history_w,
            AppIcon::Undo,
            &locale.tr("meshedit-session-undo"),
            &locale.tr("meshedit-session-undo-hint"),
            state.can_undo && enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Undo);
        }
        if icon(
            ui,
            history_w,
            AppIcon::Redo,
            &locale.tr("meshedit-session-redo"),
            &locale.tr("meshedit-session-redo-hint"),
            state.can_redo && enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Redo);
        }
        // Commit cluster (right): Cancel + Done fill the remaining width, so
        // Done lands flush against the right edge as the primary action.
        let commit_w = ((ui.available_width() - spacing) / 2.0).max(48.0);
        if tall_text_button(
            ui,
            commit_w,
            &locale.tr("meshedit-session-cancel"),
            enabled,
            false,
        )
        .on_hover_text(locale.tr("meshedit-session-cancel-hint"))
        .clicked()
        {
            action = Some(MeshEditorAction::Cancel);
        }
        if tall_text_button(
            ui,
            commit_w,
            &locale.tr("meshedit-session-done"),
            enabled,
            true,
        )
        .on_hover_text(locale.tr("meshedit-session-done-hint"))
        .clicked()
        {
            action = Some(MeshEditorAction::Done);
        }
    });
    action
}

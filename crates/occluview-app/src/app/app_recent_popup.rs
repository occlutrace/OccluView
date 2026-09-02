//! The recent-scenes dropdown that hangs off the toolbar Open button.

use super::{recent_scene_hover, recent_scene_label, PathBuf};
use crate::recent_files::RecentFiles;
use eframe::egui;

const RECENT_FILES_POPUP_ID: &str = "recent-files-dropdown-v1";

pub(super) fn recent_files_popup_id() -> egui::Id {
    egui::Id::new(RECENT_FILES_POPUP_ID)
}

pub(super) enum RecentFilesAction {
    Open(Vec<PathBuf>),
    Clear,
}

pub(super) fn show_recent_files_popup(
    trigger: &egui::Response,
    recent_files: &RecentFiles,
) -> Option<RecentFilesAction> {
    let action = egui::Popup::from_toggle_button_response(trigger)
        .id(recent_files_popup_id())
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(220.0);
            for entry in recent_files.entries() {
                if ui
                    .button(recent_scene_label(entry))
                    .on_hover_text(recent_scene_hover(entry))
                    .clicked()
                {
                    return Some(RecentFilesAction::Open(entry.paths().to_vec()));
                }
            }
            ui.separator();
            ui.button("Clear recent")
                .clicked()
                .then_some(RecentFilesAction::Clear)
        })
        .and_then(|response| response.inner);
    if action.is_some() {
        egui::Popup::close_id(&trigger.ctx, recent_files_popup_id());
    }
    action
}

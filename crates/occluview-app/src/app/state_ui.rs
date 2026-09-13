//! UI-owned state: dialogs, transient presentation, notifications, and other
//! UI-only state.
//!
//! Owned invariants:
//!
//! - `status_message*` is write-only presentation: domain code reports
//!   outcomes, and only this owner (plus [`Self::expire_status_message`])
//!   decides what the operator still sees.
//! - Dialog flags (`close_guard_open`, `pending_replace_open`, `app_error`,
//!   `information_dialog`) gate input through [`Self::modal_dialog_open`];
//!   tools and hotkeys ask the predicate, they do not enumerate dialogs.
//! - `repaint_ctx` is the egui handle for scheduling repaints; native window
//!   handles live in [`PlatformState`](super::state_platform::PlatformState).
//!
//! Permitted mutation entry points: [`UiState::new`] for bootstrap, dialog
//! flows for their own flags, any domain for its status line. Cross-domain
//! outputs: the modal predicate and the status line consumed by input
//! routing and the panels.

use super::app_settings_panel::settings_popup_id;
use super::egui;
use super::information_dialog::InformationDialog;
use super::open_dialogs::OpenDialogs;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) enum AppErrorAction {
    /// No recovery is offered; the dialog only reports.
    #[default]
    None,
    /// The graphics fault latch may be cleared and drawing attempted again.
    /// The driver may recover on its own (a device reset or an eGPU that came
    /// back), and the alternative is asking the operator to restart the viewer
    /// and lose the scene.
    RetryGraphics,
}

#[derive(Clone)]
pub(super) struct AppErrorDialog {
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) details: String,
    pub(super) action: AppErrorAction,
}

fn information_route_is_blocked(
    close_guard_open: bool,
    pending_replace_open: bool,
    app_error_open: bool,
) -> bool {
    close_guard_open || pending_replace_open || app_error_open
}

#[allow(clippy::struct_excessive_bools)]
pub(super) struct UiState {
    pub(super) repaint_ctx: egui::Context,
    /// Runtime localizer: one catalog generation per frame. UI/presentation
    /// ownership; persisted preference lives in [`PersistenceState`](super::state_persistence::PersistenceState).
    pub(super) locale: crate::i18n::LocaleManager,
    /// Whether the native window title was synced to the catalog yet.
    /// Sent once on the first frame and on every manual switch afterwards.
    pub(super) native_title_sent: bool,
    pub(super) status_message: Option<String>,
    pub(super) status_message_since: Option<Instant>,
    pub(super) status_message_snapshot: Option<String>,
    pub(super) app_error: Option<AppErrorDialog>,
    pub(super) information_dialog: InformationDialog,
    /// Persistent post-repair report card, populated by the Repair executor and
    /// drawn in `ui()`; shows what a repair changed (or that nothing did).
    pub(super) repair_report: crate::repair_report::RepairReportDialog,
    pub(super) app_logo: Option<egui::TextureHandle>,
    pub(super) foreground_pulse_until: Option<Instant>,
    pub(super) viewport_orbit_cursor_grabbed: bool,
    /// Suppresses the stationary RMB context menu when the same press already
    /// moved the camera, including motion below egui's click/drag threshold.
    pub(super) viewport_secondary_gesture_moved_since_press: bool,
    /// The empty-viewport card was clicked: open the native Open dialog once,
    /// after the panel pass (the toolbar dispatches its dialog the same way).
    pub(super) open_dialog_requested: bool,
    /// The close-guard dialog is on screen.
    pub(super) close_guard_open: bool,
    /// The operator explicitly chose to close without saving.
    pub(super) close_confirmed: bool,
    /// A replace-scene request waiting for the unsaved-edit guard.
    pub(super) pending_replace_open: Option<PendingReplaceOpen>,
    /// Layer count the automatic window-growth hint last reacted to. The hint
    /// fires only when this count changes and only ever grows the window, so a
    /// manual user resize is never fought frame by frame.
    pub(super) layers_window_layer_count: Option<usize>,
}

/// A replace-scene open request parked behind the unsaved-edit guard dialog.
#[derive(Clone)]
pub(super) struct PendingReplaceOpen {
    pub(super) paths: Vec<std::path::PathBuf>,
    pub(super) source: &'static str,
}

impl UiState {
    pub(super) fn new(repaint_ctx: egui::Context, locale: crate::i18n::LocaleManager) -> Self {
        Self {
            repaint_ctx,
            locale,
            native_title_sent: false,
            status_message: None,
            status_message_since: None,
            status_message_snapshot: None,
            app_error: None,
            information_dialog: InformationDialog::default(),
            repair_report: crate::repair_report::RepairReportDialog::default(),
            app_logo: None,
            foreground_pulse_until: None,
            viewport_orbit_cursor_grabbed: false,
            viewport_secondary_gesture_moved_since_press: false,
            open_dialog_requested: false,
            close_guard_open: false,
            close_confirmed: false,
            pending_replace_open: None,
            layers_window_layer_count: None,
        }
    }

    /// Whether a modal dialog owns the keyboard.
    ///
    /// Escape belongs to the dialog in front of the operator, never to a tool
    /// behind it. Decided inline, that list drifts: the cut and align tools
    /// missed the replace-open guard and nobody counted the third-party
    /// licences window, so with either up Escape tore the tool down behind the
    /// dialog -- and for align also ran `cancel_align_session`, putting every
    /// scan back where it started. One predicate, so the next dialog gets
    /// remembered once.
    pub(super) fn modal_dialog_open(&self) -> bool {
        OpenDialogs {
            close_guard: self.close_guard_open,
            pending_replace: self.pending_replace_open.is_some(),
            error: self.app_error.is_some(),
            settings_popup: egui::Popup::is_id_open(&self.repaint_ctx, settings_popup_id()),
            information_dialog: self.information_dialog.is_open(),
        }
        .any()
    }

    /// A decision dialog takes precedence over informational content. Keep an
    /// open information route in state so it can return after the decision is
    /// resolved, but never let its modal consume Escape or clicks underneath.
    pub(super) fn foreground_dialog_open(&self) -> bool {
        information_route_is_blocked(
            self.close_guard_open,
            self.pending_replace_open.is_some(),
            self.app_error.is_some(),
        )
    }

    /// Expire transient status text after the shared display interval.
    pub(super) fn expire_status_message(&mut self, ctx: &egui::Context) {
        const STATUS_MESSAGE_TTL: std::time::Duration = std::time::Duration::from_secs(4);
        let now = Instant::now();
        if self.status_message != self.status_message_snapshot {
            self.status_message_snapshot = self.status_message.clone();
            self.status_message_since = self.status_message.as_ref().map(|_| now);
        }
        let Some(since) = self.status_message_since else {
            return;
        };
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= STATUS_MESSAGE_TTL {
            self.status_message = None;
            self.status_message_snapshot = None;
            self.status_message_since = None;
        } else {
            ctx.request_repaint_after(STATUS_MESSAGE_TTL.saturating_sub(elapsed));
        }
    }
    /// Push the catalog window title to the native window once per
    /// language generation. eframe applies `ViewportCommand::Title` live.
    pub(super) fn sync_native_title(&mut self, ctx: &egui::Context) {
        if !self.native_title_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.locale.window_title()));
            self.native_title_sent = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn information_route_yields_to_each_foreground_decision_dialog() {
        assert!(!information_route_is_blocked(false, false, false));
        assert!(information_route_is_blocked(true, false, false));
        assert!(information_route_is_blocked(false, true, false));
        assert!(information_route_is_blocked(false, false, true));
    }

    #[test]
    fn ui_state_starts_without_dialogs_or_status() {
        let ui = UiState::new(
            egui::Context::default(),
            crate::i18n::LocaleManager::for_tests(),
        );

        assert!(!ui.modal_dialog_open());
        assert!(!ui.foreground_dialog_open());
        assert!(ui.status_message.is_none());
    }
}

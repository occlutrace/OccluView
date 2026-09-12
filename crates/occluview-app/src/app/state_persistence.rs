//! Persistence-owned state: settings, paths, save/export coordination, and
//! update-compatible stored state.
//!
//! Owned invariants:
//!
//! - `settings` is loaded once at startup and saved on change; workers never
//!   touch it directly.
//! - `recent_files` / `last_export_dir` / `current_paths` are document
//!   locations, not content; the content lives in [`DocumentState`](super::state_document::DocumentState).
//! - `update_notice` polls the release channel without ever blocking the
//!   frame; `sculpt_settings_dirty_since` debounces brush-preference writes
//!   to one fsync per settled drag.
//!
//! Permitted mutation entry points: [`PersistenceState::new`] for bootstrap,
//! the settings/save/export flows for coordination. Cross-domain outputs:
//! resolved paths and preferences consumed by document, render, and UI.

use crate::app_files::{load_recent_files, save_recent_files};
use crate::app_settings::{Settings, SettingsPersistence};
use crate::recent_files::RecentFiles;
use crate::update_notice::UpdateNotice;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long the sculpt sliders must stay still before the debounced preference
/// persist marks settings dirty (one fsync per settled drag, not per frame).
const SCULPT_SETTINGS_PERSIST_DELAY: Duration = Duration::from_secs(1);

pub(super) struct PersistenceState {
    /// Operator preferences, loaded once at startup and saved on change.
    pub(super) settings: Settings,
    pub(super) settings_persistence: SettingsPersistence,
    /// Retry state for the language sidecar, mirroring settings persistence.
    /// The sidecar lives in its own file so an old binary rewriting
    /// settings.json can never erase the language choice.
    pub(super) language_persistence: SettingsPersistence,
    pub(super) recent_files: RecentFiles,
    pub(super) last_export_dir: Option<PathBuf>,
    pub(super) current_paths: Vec<PathBuf>,
    pub(super) update_notice: UpdateNotice,
    /// When the sculpt sliders last changed during the current drag: the
    /// debounced persist in [`Self::sync_sculpt_preferences`] waits for them
    /// to settle before marking settings dirty.
    pub(super) sculpt_settings_dirty_since: Option<Instant>,
}

impl PersistenceState {
    pub(super) fn new() -> Self {
        let settings = Settings::load();
        let last_export_dir = if settings.remember_export_dir {
            settings.last_export_dir.as_ref().map(PathBuf::from)
        } else {
            None
        };
        let update_check_on_start = settings.update_check_on_start;
        Self {
            recent_files: load_recent_files(crate::app_settings::RECENT_FILES_LIMIT),
            settings,
            settings_persistence: SettingsPersistence::default(),
            language_persistence: SettingsPersistence::default(),
            last_export_dir,
            current_paths: Vec::new(),
            update_notice: UpdateNotice::begin_check(update_check_on_start),
            sculpt_settings_dirty_since: None,
        }
    }

    pub(super) fn push_recent_scene(&mut self, paths: &[PathBuf]) {
        self.recent_files.push_paths(paths);
    }

    pub(super) fn save_recent_files(&self) {
        save_recent_files(&self.recent_files);
    }

    pub(super) fn persist_settings_if_due(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.settings_persistence.should_attempt(now) {
            match self.settings.save() {
                Ok(()) => self.settings_persistence.record_success(),
                Err(error) => {
                    tracing::warn!(%error, "could not persist viewer preferences");
                    self.settings_persistence
                        .record_failure(now, error.to_string());
                }
            }
        }
        if let Some(delay) = self.settings_persistence.retry_after(now) {
            ctx.request_repaint_after(delay);
        }
    }

    /// Persist the language sidecar on the same rhythm as settings, through
    /// its own file and retry state.
    pub(super) fn persist_language_if_due(
        &mut self,
        ctx: &egui::Context,
        preference: &crate::i18n::preference::UiLanguagePreference,
    ) {
        let now = Instant::now();
        if self.language_persistence.should_attempt(now) {
            match crate::app_paths::app_state_dir() {
                Some(dir) => match crate::i18n::preference::save(&dir, preference) {
                    Ok(()) => self.language_persistence.record_success(),
                    Err(error) => {
                        tracing::warn!(%error, "could not persist language preference");
                        self.language_persistence
                            .record_failure(now, error.to_string());
                    }
                },
                None => self
                    .language_persistence
                    .record_failure(now, "application state directory is unavailable".to_owned()),
            }
        }
        if let Some(delay) = self
            .settings_persistence
            .retry_after(now)
            .into_iter()
            .chain(self.language_persistence.retry_after(now))
            .min()
        {
            ctx.request_repaint_after(delay);
        }
    }

    /// Mirror the sculpt sliders into settings while "remember brush settings"
    /// is on. The sliders themselves live in egui memory while the editor is
    /// open; this one-way sync makes the next launch restore exactly what the
    /// operator last used. The persist is debounced: a drag changes the sliders
    /// about sixty times a second, and every dirty frame would cost an fsync.
    pub(super) fn sync_sculpt_preferences(&mut self, ctx: &egui::Context) {
        if !self.settings.remember_sculpt_brush {
            self.sculpt_settings_dirty_since = None;
            return;
        }
        let size = crate::mesh_editor_overlay::sculpt_size(ctx);
        let intensity = crate::mesh_editor_overlay::sculpt_intensity(ctx);
        if (self.settings.sculpt_size - size).abs() > f32::EPSILON
            || (self.settings.sculpt_intensity - intensity).abs() > f32::EPSILON
        {
            self.settings.sculpt_size = size;
            self.settings.sculpt_intensity = intensity;
            self.sculpt_settings_dirty_since
                .get_or_insert(Instant::now());
        } else if let Some(since) = self.sculpt_settings_dirty_since {
            let settled = SCULPT_SETTINGS_PERSIST_DELAY.saturating_sub(since.elapsed());
            if settled.is_zero() {
                self.sculpt_settings_dirty_since = None;
                self.settings_persistence.mark_dirty();
            } else {
                ctx.request_repaint_after(settled);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_persistence() -> PersistenceState {
        PersistenceState {
            settings: Settings::default(),
            settings_persistence: SettingsPersistence::default(),
            language_persistence: SettingsPersistence::default(),
            recent_files: RecentFiles::new(10),
            last_export_dir: None,
            current_paths: Vec::new(),
            update_notice: UpdateNotice::begin_check(false),
            sculpt_settings_dirty_since: None,
        }
    }

    #[test]
    fn recent_scenes_accumulate_without_touching_disk() {
        let mut persistence = empty_persistence();

        assert!(persistence.recent_files.is_empty());
        persistence.push_recent_scene(&[PathBuf::from("/tmp/a.stl")]);
        assert!(!persistence.recent_files.is_empty());
    }
}

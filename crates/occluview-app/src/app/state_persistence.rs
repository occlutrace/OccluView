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
use crate::app_settings::{Settings, SettingsPersistence, RECENT_FILES_LIMIT};
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
    /// The sidecar lives in its own file so an older binary rewriting
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
            recent_files: load_recent_files(RECENT_FILES_LIMIT),
            settings,
            settings_persistence: SettingsPersistence::default(),
            language_persistence: SettingsPersistence::default(),
            last_export_dir,
            current_paths: Vec::new(),
            update_notice: UpdateNotice::begin_check(update_check_on_start),
            sculpt_settings_dirty_since: None,
        }
    }

    /// Construct persistence state without reading user preferences for tests.
    #[cfg(test)]
    pub(super) fn for_tests() -> Self {
        Self {
            settings: Settings::default(),
            settings_persistence: SettingsPersistence::default(),
            language_persistence: SettingsPersistence::default(),
            recent_files: RecentFiles::new(1),
            last_export_dir: None,
            current_paths: Vec::new(),
            update_notice: UpdateNotice::begin_check(false),
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

    /// Mirror sculpt settings into preferences while "remember brush settings"
    /// is on. The active normalized radius share is saved directly so tip
    /// changes cannot quantize it through another tip's millimetre steps. The
    /// persist is debounced because a drag changes values every frame.
    pub(super) fn sync_sculpt_preferences(&mut self, ctx: &egui::Context) {
        if !self.settings.remember_sculpt_brush {
            self.sculpt_settings_dirty_since = None;
            return;
        }
        // A close request races the debounce: the settings are already written
        // into `settings`, but the dirty mark is only set once the values have
        // been still for a second, and closing does not persist anything
        // (`intercept_unsaved_close_request` only fires for unsaved mesh edits,
        // and there is no `on_exit`). Closing within that second would discard
        // the new brush settings, so flush them now rather than on a
        // timer the closing window will not run.
        if self.sculpt_settings_dirty_since.is_some()
            && ctx.input(|i| i.viewport().close_requested())
        {
            self.sculpt_settings_dirty_since = None;
            self.settings_persistence.mark_dirty();
        }
        let radii = crate::sculpt_tool::SculptTip::ALL
            .map(|tip| crate::mesh_editor_overlay::sculpt_radius_mm(ctx, tip));
        let strengths = [
            crate::sculpt_tool::SculptToolKind::AddRemove,
            crate::sculpt_tool::SculptToolKind::Smooth,
        ]
        .map(|kind| crate::mesh_editor_overlay::sculpt_strength(ctx, kind));
        let tip = crate::mesh_editor_overlay::sculpt_tip(ctx);
        let radius_share = crate::mesh_editor_overlay::sculpt_radius_share(ctx);
        let tip_changed = self.settings.last_sculpt_tip != tip;
        let share_changed = self
            .settings
            .sculpt_radius_share
            .is_none_or(|stored| stored.to_bits() != radius_share.to_bits());
        let changed = self
            .settings
            .sculpt_radii_mm
            .iter()
            .zip(radii)
            .any(|(before, after)| (before - after).abs() > f32::EPSILON)
            || self
                .settings
                .sculpt_strengths
                .iter()
                .zip(strengths)
                .any(|(before, after)| (before - after).abs() > f32::EPSILON)
            || tip_changed
            || share_changed;
        if changed {
            self.settings.sculpt_radii_mm = radii;
            self.settings.sculpt_radius_share = Some(radius_share);
            self.settings.sculpt_strengths = strengths;
            self.settings.last_sculpt_tip = tip;
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

    #[test]
    fn sculpt_sync_persists_the_exact_shared_radius_and_active_tip() {
        let ctx = egui::Context::default();
        let share = 0.123_456_7_f32;
        crate::mesh_editor_overlay::set_sculpt_tip(&ctx, crate::sculpt_tool::SculptTip::Cylinder);
        crate::mesh_editor_overlay::set_sculpt_radius_share(&ctx, share);
        let mut persistence = empty_persistence();

        persistence.sync_sculpt_preferences(&ctx);

        assert_eq!(
            persistence.settings.last_sculpt_tip,
            crate::sculpt_tool::SculptTip::Cylinder
        );
        assert_eq!(
            persistence.settings.sculpt_radius_share.map(f32::to_bits),
            Some(share.to_bits()),
            "saved share must not pass through another tip's millimetre step grid"
        );
        let expected_radii = [
            crate::mesh_editor_overlay::sculpt_radius_mm(&ctx, crate::sculpt_tool::SculptTip::Ball),
            crate::mesh_editor_overlay::sculpt_radius_mm(
                &ctx,
                crate::sculpt_tool::SculptTip::Knife,
            ),
            crate::mesh_editor_overlay::sculpt_radius_mm(
                &ctx,
                crate::sculpt_tool::SculptTip::Cylinder,
            ),
        ];
        assert_eq!(
            persistence.settings.sculpt_radii_mm.map(f32::to_bits),
            expected_radii.map(f32::to_bits)
        );
    }
}

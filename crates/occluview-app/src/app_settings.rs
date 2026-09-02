//! Persistent viewer preferences and their retry state.

use anyhow::{Context as _, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SETTINGS_FILE: &str = "settings.json";
pub(crate) const SETTINGS_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Format used only when the source format cannot be written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FallbackExportFormat {
    #[default]
    Ply,
    Stl,
    Obj,
}

impl FallbackExportFormat {
    pub(crate) const OPTIONS: [Self; 3] = [Self::Ply, Self::Stl, Self::Obj];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Ply => "PLY",
            Self::Stl => "STL",
            Self::Obj => "OBJ",
        }
    }
}

/// Preset for the 3D viewport clear color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ViewportBackground {
    /// The established neutral studio gray.
    #[default]
    Gray,
    White,
    Dark,
}

impl ViewportBackground {
    pub(crate) const OPTIONS: [Self; 3] = [Self::Gray, Self::White, Self::Dark];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Gray => "Gray",
            Self::White => "White",
            Self::Dark => "Dark",
        }
    }

    /// Whether the clear color reads as dark. Overlays painted directly on the
    /// render (scale bar) pick their ink by this — not by the chrome theme,
    /// which is an independent setting.
    pub(crate) const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    /// The sRGB-encoded clear color, for UI surfaces painted around the render.
    /// This is the source of truth for the preset's appearance.
    pub(crate) const fn srgb(self) -> egui::Color32 {
        match self {
            Self::Gray => egui::Color32::from_rgb(226, 230, 234),
            Self::White => egui::Color32::from_rgb(247, 247, 247),
            Self::Dark => egui::Color32::from_rgb(32, 35, 40),
        }
    }

    /// The clear color in the renderer's linear space, converted from the
    /// sRGB intent so the two representations can never drift apart (they once
    /// did: the dark preset's linear values encoded back to a medium gray).
    pub(crate) fn linear(self) -> [f64; 4] {
        let color = self.srgb();
        [
            srgb_to_linear_channel(color.r()),
            srgb_to_linear_channel(color.g()),
            srgb_to_linear_channel(color.b()),
            1.0,
        ]
    }
}

/// The inverse sRGB piecewise curve for one 0..255 channel.
fn srgb_to_linear_channel(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Length unit for every measurement readout (ruler, thickness, scale bar).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum UnitDisplay {
    #[default]
    Millimeters,
    Inches,
}

impl UnitDisplay {
    pub(crate) const OPTIONS: [Self; 2] = [Self::Millimeters, Self::Inches];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Millimeters => "mm",
            Self::Inches => "in",
        }
    }
}

/// UI chrome theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ThemePreference {
    #[default]
    Light,
    Dark,
}

impl ThemePreference {
    pub(crate) const OPTIONS: [Self; 2] = [Self::Light, Self::Dark];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

fn deserialize_export_format<'de, D>(deserializer: D) -> Result<FallbackExportFormat, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(match value.as_str() {
        "Stl" => FallbackExportFormat::Stl,
        "Obj" => FallbackExportFormat::Obj,
        _ => FallbackExportFormat::Ply,
    })
}

/// The durable choices exposed by the preferences panel. Many independent
/// toggles is the shape of a preferences document; collapsing them into enums
/// would be the over-engineering here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    #[serde(
        alias = "default_export_format",
        deserialize_with = "deserialize_export_format"
    )]
    pub(crate) fallback_export_format: FallbackExportFormat,
    pub(crate) remember_export_dir: bool,
    pub(crate) last_export_dir: Option<String>,
    pub(crate) update_check_on_start: bool,
    /// Frame (fit) a scene to the home view when it opens, instead of keeping
    /// the current camera pose.
    pub(crate) frame_scene_on_open: bool,
    /// Double primary click on the viewport resets the camera to the home view.
    pub(crate) double_click_resets_camera: bool,
    /// Multiplier on the fixed orbit drag gain, clamped at use to 0.25..=4.
    pub(crate) orbit_sensitivity: f32,
    /// Exponent on the scroll zoom factor, clamped at use to 0.25..=4.
    pub(crate) zoom_sensitivity: f32,
    /// How many recent scenes the Open chevron keeps, clamped at use to 4..=20.
    pub(crate) recent_files_limit: usize,
    pub(crate) viewport_background: ViewportBackground,
    /// Draw the cut-away side as a translucent ghost during a cut view.
    pub(crate) show_cut_ghost: bool,
    pub(crate) unit_display: UnitDisplay,
    /// UI scale multiplier on the system pixel density, clamped at use to
    /// 0.85..=1.5 (1.0 keeps the platform default).
    pub(crate) ui_scale: f32,
    pub(crate) theme: ThemePreference,
    /// Keep the sculpt-brush size/intensity sliders across sessions instead of
    /// resetting them to the built-in defaults.
    pub(crate) remember_sculpt_brush: bool,
    /// Last used sculpt size, honored only while `remember_sculpt_brush`.
    pub(crate) sculpt_size: f32,
    /// Last used sculpt intensity, honored only while `remember_sculpt_brush`.
    pub(crate) sculpt_intensity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fallback_export_format: FallbackExportFormat::Ply,
            remember_export_dir: false,
            last_export_dir: None,
            update_check_on_start: true,
            frame_scene_on_open: true,
            double_click_resets_camera: true,
            orbit_sensitivity: 1.0,
            zoom_sensitivity: 1.0,
            recent_files_limit: 8,
            viewport_background: ViewportBackground::default(),
            show_cut_ghost: true,
            unit_display: UnitDisplay::default(),
            ui_scale: 1.0,
            theme: ThemePreference::default(),
            remember_sculpt_brush: true,
            sculpt_size: 40.0,
            sculpt_intensity: 50.0,
        }
    }
}

impl Settings {
    pub(crate) fn orbit_sensitivity(&self) -> f32 {
        self.orbit_sensitivity.clamp(0.25, 4.0)
    }

    pub(crate) fn zoom_sensitivity(&self) -> f32 {
        self.zoom_sensitivity.clamp(0.25, 4.0)
    }

    pub(crate) fn recent_files_limit(&self) -> usize {
        self.recent_files_limit.clamp(4, 20)
    }

    pub(crate) fn ui_scale(&self) -> f32 {
        self.ui_scale.clamp(0.85, 1.5)
    }
    fn path() -> Option<PathBuf> {
        crate::app_paths::app_state_dir().map(|dir| dir.join(SETTINGS_FILE))
    }

    pub(crate) fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(settings) => settings,
                Err(error) => {
                    tracing::warn!(%error, "settings.json is invalid; using defaults");
                    // Preserve the broken file for diagnosis instead of letting
                    // the next dirty write silently overwrite it.
                    let backup = path.with_extension("json.bak");
                    if let Err(backup_error) = std::fs::rename(&path, &backup) {
                        tracing::warn!(
                            %backup_error,
                            "could not preserve the invalid settings file"
                        );
                    }
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(%error, "settings.json could not be read; using defaults");
                Self::default()
            }
        }
    }

    pub(crate) fn save(&self) -> Result<()> {
        let path = Self::path().context("application state directory is unavailable")?;
        self.save_to(&path)
    }

    pub(crate) fn set_remember_export_dir(&mut self, remember: bool, current: Option<&Path>) {
        self.remember_export_dir = remember;
        self.last_export_dir = if remember {
            current.and_then(Path::to_str).map(str::to_owned)
        } else {
            None
        };
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("settings path has no parent directory")?;
        std::fs::create_dir_all(parent).context("could not create settings directory")?;

        let bytes = serde_json::to_vec_pretty(self).context("could not serialize settings")?;
        let temporary = path.with_extension("json.tmp");
        if let Err(error) = write_temporary_settings(&temporary, &bytes) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }

        if let Err(error) = std::fs::rename(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error).context("could not replace settings file");
        }
        Ok(())
    }
}

fn write_temporary_settings(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path).context("could not create settings file")?;
    file.write_all(bytes)
        .context("could not write settings file")?;
    file.sync_all().context("could not flush settings file")?;
    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct SettingsPersistence {
    dirty: bool,
    retry_at: Option<Instant>,
    error: Option<String>,
}

impl SettingsPersistence {
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
        self.retry_at = None;
    }

    pub(crate) fn should_attempt(&self, now: Instant) -> bool {
        self.dirty && self.retry_at.is_none_or(|deadline| now >= deadline)
    }

    pub(crate) fn record_success(&mut self) {
        self.dirty = false;
        self.retry_at = None;
        self.error = None;
    }

    pub(crate) fn record_failure(&mut self, now: Instant, error: String) {
        self.dirty = true;
        self.retry_at = Some(now + SETTINGS_RETRY_DELAY);
        self.error = Some(error);
    }

    pub(crate) fn retry_after(&self, now: Instant) -> Option<Duration> {
        self.dirty
            .then_some(self.retry_at)
            .flatten()
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    #[cfg(test)]
    fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_persistence_stays_pending_until_the_retry_deadline() {
        let now = Instant::now();
        let mut persistence = SettingsPersistence::default();
        persistence.mark_dirty();

        assert!(persistence.should_attempt(now));
        persistence.record_failure(now, "read-only filesystem".to_string());
        assert!(persistence.is_dirty());
        assert!(!persistence.should_attempt(now + Duration::from_secs(1)));
        assert!(persistence.should_attempt(now + SETTINGS_RETRY_DELAY));

        persistence.record_success();
        assert!(!persistence.is_dirty());
        assert!(persistence.error().is_none());
    }

    #[test]
    fn saving_replaces_the_complete_document_without_a_torn_temp_file() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "occluview-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&root)?;
        let path = root.join("settings.json");
        std::fs::write(&path, b"old")?;

        let settings = Settings {
            remember_export_dir: true,
            last_export_dir: Some("/case/exports".to_string()),
            ..Settings::default()
        };
        settings.save_to(&path)?;

        let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        assert_eq!(stored["remember_export_dir"], true);
        assert_eq!(stored["last_export_dir"], "/case/exports");
        assert!(!path.with_extension("json.tmp").exists());

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn obsolete_preferences_are_removed_when_the_document_is_rewritten() -> Result<()> {
        let legacy = br#"{
            "schema_version": 1,
            "reset_camera_on_open": false,
            "default_export_format": "Stl",
            "remember_export_dir": false,
            "last_export_dir": null,
            "update_check_on_start": true
        }"#;
        let settings: Settings = serde_json::from_slice(legacy)?;
        let rewritten = serde_json::to_value(settings)?;

        assert_eq!(rewritten["fallback_export_format"], "Stl");
        assert!(rewritten.get("default_export_format").is_none());
        assert!(rewritten.get("schema_version").is_none());
        assert!(rewritten.get("reset_camera_on_open").is_none());
        // `recent_files_limit` used to be obsolete and was stripped on rewrite;
        // it is a live preference again, so a rewritten document keeps it.
        assert_eq!(
            rewritten["recent_files_limit"],
            Settings::default().recent_files_limit
        );
        Ok(())
    }

    #[test]
    fn legacy_auto_fallback_becomes_ply() -> Result<()> {
        let settings: Settings = serde_json::from_str(r#"{"default_export_format":"Auto"}"#)?;

        assert_eq!(settings.fallback_export_format, FallbackExportFormat::Ply);
        Ok(())
    }

    #[test]
    fn enabling_folder_memory_captures_the_current_session_folder() {
        let mut settings = Settings::default();
        settings.set_remember_export_dir(true, Some(Path::new("/case/exports")));

        assert!(settings.remember_export_dir);
        assert_eq!(settings.last_export_dir.as_deref(), Some("/case/exports"));

        settings.set_remember_export_dir(false, Some(Path::new("/case/exports")));
        assert!(!settings.remember_export_dir);
        assert!(settings.last_export_dir.is_none());
    }
}

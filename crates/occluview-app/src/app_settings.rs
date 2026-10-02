//! Persistent viewer preferences and their retry state.

use anyhow::{Context as _, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SETTINGS_FILE: &str = "settings.json";
pub(crate) const SETTINGS_RETRY_DELAY: Duration = Duration::from_secs(5);

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
    /// sRGB intent so the two representations can never drift apart.
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

/// How a ruler ending on another ruler's line meets it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RulerLineAngle {
    /// The end goes where the pointer is along the line, and the ruler reads
    /// out the angle it makes with the line.
    #[default]
    Free,
    /// The end is the foot of the perpendicular and stays at 90 degrees.
    Perpendicular,
}

impl RulerLineAngle {
    pub(crate) const OPTIONS: [Self; 2] = [Self::Free, Self::Perpendicular];

    /// The other choice, which Shift selects while it is held.
    pub(crate) const fn other(self) -> Self {
        match self {
            Self::Free => Self::Perpendicular,
            Self::Perpendicular => Self::Free,
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
}

/// Action for pixel-unit scroll input in the 3D viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ScrollBehavior {
    /// Move the view with smooth scroll input.
    #[default]
    Pan,
    /// Change the view scale with smooth scroll input.
    Zoom,
}

impl ScrollBehavior {
    #[cfg(target_os = "macos")]
    pub(crate) const OPTIONS: [Self; 2] = [Self::Pan, Self::Zoom];
}

/// Number of recent scenes the Open menu keeps.
///
/// Fixed rather than a preference: menu length has no clinical outcome, and the
/// preferences panel holds choices that change what the operator sees on a
/// scan.
pub(crate) const RECENT_FILES_LIMIT: usize = 8;

/// The durable choices exposed by the preferences panel. Many independent
/// toggles is the shape of a preferences document; collapsing them into enums
/// would be the over-engineering here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
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
    /// How macOS pixel-unit scroll input moves the viewport.
    pub(crate) scroll_behavior: ScrollBehavior,
    pub(crate) viewport_background: ViewportBackground,
    /// Draw the cut-away side as a translucent ghost during a cut view.
    pub(crate) show_cut_ghost: bool,
    pub(crate) unit_display: UnitDisplay,
    /// How a ruler ending on another ruler's line meets it.
    pub(crate) ruler_line_angle: RulerLineAngle,
    /// UI scale multiplier on the system pixel density, clamped at use to
    /// 0.85..=1.5 (1.0 keeps the platform default).
    pub(crate) ui_scale: f32,
    pub(crate) theme: ThemePreference,
    /// Keep the sculpt brush, tip, size, and strengths across sessions instead
    /// of resetting them to the tool-catalog defaults.
    pub(crate) remember_sculpt_brush: bool,
    /// Last selected Sculpt brush and tip. They are restored on entering the
    /// Sculpt tab; app startup remains in the non-editing state.
    #[serde(deserialize_with = "deserialize_last_sculpt_tool")]
    pub(crate) last_sculpt_tool: crate::sculpt::sculpt_tool::SculptToolKind,
    #[serde(deserialize_with = "deserialize_last_sculpt_tip")]
    pub(crate) last_sculpt_tip: crate::sculpt::sculpt_tool::SculptTip,
    /// Compatibility snapshot of one normalized size choice, mapped into the
    /// Ball, Knife and Cylinder physical ranges for the existing settings file.
    #[serde(default = "default_sculpt_radii_mm")]
    pub(crate) sculpt_radii_mm: [f32; 3],
    /// Exact normalized brush size. The millimetre array above remains the
    /// migration fallback and a compatibility snapshot for older builds.
    #[serde(deserialize_with = "deserialize_sculpt_radius_share")]
    pub(crate) sculpt_radius_share: Option<f32>,
    /// Add/Remove and Smooth strengths in kernel units.
    #[serde(default = "default_sculpt_strengths")]
    pub(crate) sculpt_strengths: [f32; 2],
    /// Previous versions saved generic 1..100 sliders. Read them once for
    /// migration, but never write the obsolete fields back out.
    #[serde(default, rename = "sculpt_size", skip_serializing)]
    legacy_sculpt_size: Option<f32>,
    #[serde(default, rename = "sculpt_intensity", skip_serializing)]
    legacy_sculpt_intensity: Option<f32>,
}

const fn default_sculpt_radii_mm() -> [f32; 3] {
    [0.75, 0.5, 0.5]
}

const fn default_sculpt_strengths() -> [f32; 2] {
    [0.35, 0.15]
}

fn deserialize_last_sculpt_tool<'de, D>(
    deserializer: D,
) -> std::result::Result<crate::sculpt::sculpt_tool::SculptToolKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value.as_str() {
        Some("smooth") => crate::sculpt::sculpt_tool::SculptToolKind::Smooth,
        _ => crate::sculpt::sculpt_tool::SculptToolKind::AddRemove,
    })
}

fn deserialize_last_sculpt_tip<'de, D>(
    deserializer: D,
) -> std::result::Result<crate::sculpt::sculpt_tool::SculptTip, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value.as_str() {
        Some("knife") => crate::sculpt::sculpt_tool::SculptTip::Knife,
        Some("cylinder") => crate::sculpt::sculpt_tool::SculptTip::Cylinder,
        _ => crate::sculpt::sculpt_tool::SculptTip::Ball,
    })
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "Share is stored as f32; out-of-range conversions are filtered and then clamped at use."
)]
fn deserialize_sculpt_radius_share<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_f64()
        .map(|share| share as f32)
        .filter(|share| share.is_finite()))
}

const LEGACY_SCULPT_SIZE_DEFAULT: f32 = 40.0;
const LEGACY_SCULPT_INTENSITY_DEFAULT: f32 = 50.0;

impl Default for Settings {
    fn default() -> Self {
        Self {
            remember_export_dir: false,
            last_export_dir: None,
            update_check_on_start: true,
            frame_scene_on_open: true,
            double_click_resets_camera: true,
            orbit_sensitivity: 1.0,
            zoom_sensitivity: 1.0,
            scroll_behavior: ScrollBehavior::default(),
            viewport_background: ViewportBackground::default(),
            show_cut_ghost: true,
            unit_display: UnitDisplay::default(),
            ruler_line_angle: RulerLineAngle::default(),
            ui_scale: 1.0,
            theme: ThemePreference::default(),
            remember_sculpt_brush: true,
            last_sculpt_tool: crate::sculpt::sculpt_tool::SculptToolKind::default(),
            last_sculpt_tip: crate::sculpt::sculpt_tool::SculptTip::default(),
            sculpt_radii_mm: default_sculpt_radii_mm(),
            sculpt_radius_share: None,
            sculpt_strengths: default_sculpt_strengths(),
            legacy_sculpt_size: None,
            legacy_sculpt_intensity: None,
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

    pub(crate) fn ui_scale(&self) -> f32 {
        self.ui_scale.clamp(0.85, 1.5)
    }

    // An old slider default is not an authored preference. Keep exact equality
    // here: tolerances could overwrite a user's saved value.
    #[allow(clippy::float_cmp)]
    fn migrate_legacy_sculpt_preferences(
        &mut self,
        has_explicit_radii: bool,
        has_explicit_strengths: bool,
    ) {
        if let Some(size) = self.legacy_sculpt_size.take() {
            if !has_explicit_radii {
                if size == LEGACY_SCULPT_SIZE_DEFAULT {
                    self.sculpt_radii_mm = default_sculpt_radii_mm();
                } else {
                    // Preserve the former linear mapping: slider 1..100 represented
                    // 0.4..12 mm. Clamp that remembered physical radius to each
                    // donor tip's actual catalog range.
                    let fraction = ((size - 1.0) / 99.0).clamp(0.0, 1.0);
                    let radius = 0.4 + fraction * (12.0 - 0.4);
                    self.sculpt_radii_mm = [
                        radius.clamp(0.25, 4.0),
                        radius.clamp(0.25, 2.5),
                        radius.clamp(0.25, 2.0),
                    ];
                }
            }
        }
        if let Some(intensity) = self.legacy_sculpt_intensity.take() {
            if !has_explicit_strengths {
                if intensity == LEGACY_SCULPT_INTENSITY_DEFAULT {
                    self.sculpt_strengths = default_sculpt_strengths();
                } else {
                    let strength = (intensity / 100.0).clamp(0.0, 1.0);
                    self.sculpt_strengths = [strength.clamp(0.05, 1.0), strength.clamp(0.01, 1.0)];
                }
            }
        }
    }

    fn from_json_slice(bytes: &[u8]) -> std::result::Result<Self, serde_json::Error> {
        let serialized: serde_json::Value = serde_json::from_slice(bytes)?;
        let has_explicit_radii = serialized.get("sculpt_radii_mm").is_some();
        let has_explicit_strengths = serialized.get("sculpt_strengths").is_some();
        let mut settings: Self = serde_json::from_value(serialized)?;
        settings.migrate_legacy_sculpt_preferences(has_explicit_radii, has_explicit_strengths);
        Ok(settings)
    }
    fn path() -> Option<PathBuf> {
        crate::desktop::app_paths::app_state_dir().map(|dir| dir.join(SETTINGS_FILE))
    }

    pub(crate) fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => match Self::from_json_slice(&bytes) {
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
            "fallback_export_format": "Stl",
            "keep_source_export_format": false,
            "remember_export_dir": false,
            "last_export_dir": null,
            "update_check_on_start": true,
            "recent_files_limit": 20
        }"#;
        let settings: Settings = serde_json::from_slice(legacy)?;
        let rewritten = serde_json::to_value(settings)?;

        // The save format is not a stored preference: a document that carries
        // the format fields loads without complaint and is rewritten without
        // them.
        assert!(rewritten.get("default_export_format").is_none());
        assert!(rewritten.get("fallback_export_format").is_none());
        assert!(rewritten.get("keep_source_export_format").is_none());
        assert!(rewritten.get("schema_version").is_none());
        assert!(rewritten.get("reset_camera_on_open").is_none());
        assert!(rewritten.get("recent_files_limit").is_none());
        Ok(())
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "These persisted catalog values are exactly representable."
    )]
    fn legacy_sculpt_defaults_migrate_to_catalog_defaults() -> Result<()> {
        let settings = Settings::from_json_slice(
            br#"{"remember_sculpt_brush":true,"sculpt_size":40.0,"sculpt_intensity":50.0}"#,
        )?;

        assert_eq!(settings.sculpt_radii_mm, default_sculpt_radii_mm());
        assert_eq!(settings.sculpt_strengths, default_sculpt_strengths());

        let stored = serde_json::to_value(settings)?;
        assert!(stored.get("sculpt_size").is_none());
        assert!(stored.get("sculpt_intensity").is_none());
        Ok(())
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "These migrated settings are exactly representable catalog values."
    )]
    fn authored_legacy_sculpt_settings_migrate_and_explicit_new_preferences_win() -> Result<()> {
        let migrated =
            Settings::from_json_slice(br#"{"sculpt_size":80.0,"sculpt_intensity":70.0}"#)?;
        assert_eq!(migrated.sculpt_radii_mm, [4.0, 2.5, 2.0]);
        assert_eq!(migrated.sculpt_strengths, [0.7, 0.7]);

        let explicit = Settings::from_json_slice(
            br#"{
                "sculpt_size":80.0,
                "sculpt_intensity":70.0,
                "sculpt_radii_mm":[0.75,0.5,0.5],
                "sculpt_strengths":[0.35,0.15]
            }"#,
        )?;
        assert_eq!(explicit.sculpt_radii_mm, default_sculpt_radii_mm());
        assert_eq!(explicit.sculpt_strengths, default_sculpt_strengths());
        Ok(())
    }

    /// A settings document that carries obsolete export-format fields loads, and
    /// those fields are dropped.
    #[test]
    fn documents_with_obsolete_export_format_fields_load() -> Result<()> {
        for legacy in [
            r#"{"default_export_format":"Auto"}"#,
            r#"{"default_export_format":"Stl"}"#,
            r#"{"fallback_export_format":"Obj"}"#,
            r#"{"keep_source_export_format":false}"#,
        ] {
            let settings: Settings = serde_json::from_str(legacy)?;
            assert_eq!(
                settings.remember_export_dir,
                Settings::default().remember_export_dir,
                "the rest of the document must read as its default: {legacy}"
            );
        }
        Ok(())
    }

    #[test]
    fn pixel_scroll_behavior_defaults_for_older_settings_and_round_trips() -> Result<()> {
        let mut settings: Settings = serde_json::from_str(r#"{"zoom_sensitivity":1.0}"#)?;
        assert_eq!(settings.scroll_behavior, ScrollBehavior::Pan);

        settings.scroll_behavior = ScrollBehavior::Zoom;
        let saved = serde_json::to_vec(&settings)?;
        let loaded: Settings = serde_json::from_slice(&saved)?;
        assert_eq!(loaded.scroll_behavior, ScrollBehavior::Zoom);
        Ok(())
    }

    #[test]
    fn sculpt_selection_round_trips_and_invalid_values_fall_back_safely() -> Result<()> {
        let settings = Settings {
            last_sculpt_tool: crate::sculpt::sculpt_tool::SculptToolKind::Smooth,
            last_sculpt_tip: crate::sculpt::sculpt_tool::SculptTip::Cylinder,
            sculpt_radius_share: Some(0.123_456_7),
            ..Settings::default()
        };
        let encoded = serde_json::to_vec(&settings)?;
        let loaded: Settings = serde_json::from_slice(&encoded)?;
        assert_eq!(
            loaded.last_sculpt_tool,
            crate::sculpt::sculpt_tool::SculptToolKind::Smooth
        );
        assert_eq!(
            loaded.last_sculpt_tip,
            crate::sculpt::sculpt_tool::SculptTip::Cylinder
        );
        assert_eq!(
            loaded.sculpt_radius_share.map(f32::to_bits),
            Some(0.123_456_7_f32.to_bits())
        );

        let invalid = Settings::from_json_slice(
            br#"{"last_sculpt_tool":"pull","last_sculpt_tip":17,"sculpt_radius_share":"large"}"#,
        )?;
        assert_eq!(
            invalid.last_sculpt_tool,
            crate::sculpt::sculpt_tool::SculptToolKind::AddRemove
        );
        assert_eq!(
            invalid.last_sculpt_tip,
            crate::sculpt::sculpt_tool::SculptTip::Ball
        );
        assert_eq!(invalid.sculpt_radius_share, None);
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

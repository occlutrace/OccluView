//! Persistent viewer preferences and their retry state.

use anyhow::{Context as _, Result};
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

/// The three durable choices exposed by the compact preferences panel.
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fallback_export_format: FallbackExportFormat::Ply,
            remember_export_dir: false,
            last_export_dir: None,
            update_check_on_start: true,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        crate::app_paths::app_state_dir().map(|dir| dir.join(SETTINGS_FILE))
    }

    pub(crate) fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "settings.json is invalid; using defaults");
                Self::default()
            }),
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
            "recent_files_limit": 20,
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
        assert!(rewritten.get("recent_files_limit").is_none());
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

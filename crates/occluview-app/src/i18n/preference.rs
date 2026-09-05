//! Versioned atomic sidecar for the manual UI language preference.
//!
//! The preference lives OUTSIDE `settings.json`: an old binary silently
//! drops unknown JSON fields on rewrite, which would erase the language
//! choice on downgrade. The sidecar survives installer updates and old
//! rewrites, is ignored by old binaries, and degrades to `Auto`/English
//! on any corruption — without touching the rest of `Settings`.
//!
//! Mirrors the `settings.json` persistence pattern (temp file + `sync_all`
//! + rename, `.bak` preservation, retry-friendly `Result`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::Path;

use super::tags;

/// Sidecar file name, stored beside existing app state (never in the
/// installation directory).
pub(crate) const SIDECAR_FILE: &str = "ui-language-preference.json";
/// Schema version of the sidecar document.
pub(crate) const SIDECAR_SCHEMA_VERSION: u32 = 1;

/// UI language preference: system-driven or an explicit canonical tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UiLanguagePreference {
    /// Resolve from the OS preferred-language list at defined points
    /// (cold launch, first launch after update, explicit apply action).
    Auto,
    /// Manual choice. Always beats `Auto`. The tag is canonical BCP-47;
    /// an unavailable catalog still renders English while the tag is kept.
    Explicit(&'static str),
}

impl UiLanguagePreference {
    /// The effective catalog tag for this preference given an `Auto`
    /// resolution. `Explicit` wins unconditionally.
    pub(crate) fn effective_tag(&self, auto_resolved: &'static str) -> &'static str {
        match self {
            Self::Auto => auto_resolved,
            Self::Explicit(tag) => tag,
        }
    }
}

/// Machine-readable diagnostic code for sidecar problems. These codes are
/// locale-invariant (safe for logs); the user-facing explanation shown in
/// the UI is localized separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidecarDiagnostic {
    /// File exists but is not valid JSON or has the wrong schema.
    Malformed,
    /// File parses but the stored preference value is not usable.
    InvalidValue,
    /// File could not be read (permissions, directory missing, ...).
    Unreadable,
    /// File parses but carries a newer unknown schema version. Left in
    /// place (never renamed away) so a newer binary — or a future release
    /// that restores the schema — still finds the operator's choice.
    UnsupportedVersion,
}

#[derive(Debug, Serialize, Deserialize)]
struct SidecarDocument {
    version: u32,
    preference: String,
}

/// Load the preference from `state_dir`. Never panics; any problem falls
/// back to `Auto` with a diagnostic, leaving other `Settings` intact.
pub(crate) fn load(state_dir: &Path) -> (UiLanguagePreference, Option<SidecarDiagnostic>) {
    let path = state_dir.join(SIDECAR_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (UiLanguagePreference::Auto, None);
        }
        Err(_) => {
            return (
                UiLanguagePreference::Auto,
                Some(SidecarDiagnostic::Unreadable),
            )
        }
    };
    let Ok(document) = serde_json::from_slice::<SidecarDocument>(&bytes) else {
        preserve_corrupt(&path);
        return (
            UiLanguagePreference::Auto,
            Some(SidecarDiagnostic::Malformed),
        );
    };
    if document.version != SIDECAR_SCHEMA_VERSION {
        // A newer unknown schema is NOT renamed away: downgrade-then-upgrade
        // must still find the operator's choice afterwards.
        return (
            UiLanguagePreference::Auto,
            Some(SidecarDiagnostic::UnsupportedVersion),
        );
    }
    let value = document.preference.trim();
    if value.eq_ignore_ascii_case("auto") {
        (UiLanguagePreference::Auto, None)
    } else if let Some(tag) = tags::validate_explicit(value) {
        (UiLanguagePreference::Explicit(tag), None)
    } else if tags::canonicalize(value).is_some() {
        // Well-formed BCP-47 but an unknown catalog: almost certainly a
        // newer binary's catalog seen after a downgrade. Like
        // `UnsupportedVersion` the file stays in place so a later upgrade
        // restores the operator's choice; this launch runs `Auto`.
        (
            UiLanguagePreference::Auto,
            Some(SidecarDiagnostic::InvalidValue),
        )
    } else {
        preserve_corrupt(&path);
        (
            UiLanguagePreference::Auto,
            Some(SidecarDiagnostic::InvalidValue),
        )
    }
}

/// Persist the preference atomically. Refuses to write an invalid explicit
/// tag so a good file is never overwritten with a corrupt value.
pub(crate) fn save(state_dir: &Path, preference: &UiLanguagePreference) -> Result<()> {
    let value: &str = match preference {
        UiLanguagePreference::Auto => "auto",
        UiLanguagePreference::Explicit(tag) => {
            if tags::validate_explicit(tag).is_none() {
                anyhow::bail!("refusing to persist invalid language tag");
            }
            tag
        }
    };
    let document = SidecarDocument {
        version: SIDECAR_SCHEMA_VERSION,
        preference: value.to_owned(),
    };
    let path = state_dir.join(SIDECAR_FILE);
    let parent = path
        .parent()
        .context("language preference path has no parent directory")?;
    std::fs::create_dir_all(parent).context("could not create application state directory")?;

    let bytes =
        serde_json::to_vec_pretty(&document).context("could not serialize language preference")?;
    let temporary = path.with_extension("json.tmp");
    if let Err(error) = write_temporary(&temporary, &bytes) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).context("could not replace language preference file");
    }
    Ok(())
}

/// Keep the corrupt file for diagnosis instead of letting the next save
/// silently overwrite it. Failures here are non-fatal by design.
fn preserve_corrupt(path: &Path) {
    let backup = path.with_extension("json.bak");
    // `rename` fails on Windows when the target exists; drop a stale
    // backup first so quarantine never silently no-ops.
    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::rename(path, &backup);
}

fn write_temporary(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        std::fs::File::create(path).context("could not create language preference file")?;
    file.write_all(bytes)
        .context("could not write language preference file")?;
    file.sync_all()
        .context("could not flush language preference file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn unique_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "occluview-i18n-test-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn absent_sidecar_means_auto_without_diagnostic() {
        let dir = unique_dir("absent");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_auto_and_explicit() {
        let dir = unique_dir("roundtrip");
        save(&dir, &UiLanguagePreference::Auto).expect("save auto");
        assert_eq!(load(&dir).0, UiLanguagePreference::Auto);
        save(&dir, &UiLanguagePreference::Explicit("ru")).expect("save ru");
        assert_eq!(load(&dir).0, UiLanguagePreference::Explicit("ru"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_and_invalid_files_fall_back_to_auto_with_diagnostic() {
        let dir = unique_dir("malformed");
        std::fs::write(dir.join(SIDECAR_FILE), b"{not json").expect("write");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, Some(SidecarDiagnostic::Malformed));

        std::fs::write(
            dir.join(SIDECAR_FILE),
            r#"{"version":1,"preference":"pt-PT"}"#,
        )
        .expect("write");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, Some(SidecarDiagnostic::InvalidValue));

        std::fs::write(
            dir.join(SIDECAR_FILE),
            r#"{"version":99,"preference":"auto"}"#,
        )
        .expect("write");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, Some(SidecarDiagnostic::UnsupportedVersion));
        // A newer schema is left in place for a future upgrade to find.
        assert!(
            dir.join(SIDECAR_FILE).is_file(),
            "unsupported version must not be renamed away"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wellformed_unknown_tag_stays_for_future_upgrade() {
        // Canonical BCP-47 but no catalog (newer binary's choice seen
        // after a downgrade): run `Auto` now, leave the file in place so
        // a later upgrade restores the choice.
        let dir = unique_dir("future-tag");
        let body = r#"{"version":1,"preference":"pt-PT"}"#;
        std::fs::write(dir.join(SIDECAR_FILE), body).expect("write");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, Some(SidecarDiagnostic::InvalidValue));
        assert_eq!(
            std::fs::read(dir.join(SIDECAR_FILE)).expect("read"),
            body.as_bytes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_is_quarantined_even_over_stale_backup() {
        // Truly malformed content is renamed to `.bak`; a stale backup
        // never blocks quarantine (Windows `rename` semantics).
        let dir = unique_dir("quarantine");
        std::fs::write(dir.join("ui-language-preference.json.bak"), b"stale").expect("write");
        std::fs::write(dir.join(SIDECAR_FILE), b"{not json").expect("write");
        let (preference, diagnostic) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Auto);
        assert_eq!(diagnostic, Some(SidecarDiagnostic::Malformed));
        assert!(!dir.join(SIDECAR_FILE).exists());
        assert_eq!(
            std::fs::read(dir.join("ui-language-preference.json.bak")).expect("read"),
            b"{not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refuses_invalid_tag_and_keeps_good_file() {
        let dir = unique_dir("refuse");
        save(&dir, &UiLanguagePreference::Explicit("de")).expect("save de");
        let before = std::fs::read(dir.join(SIDECAR_FILE)).expect("read");
        assert!(save(&dir, &UiLanguagePreference::Explicit("xx")).is_err());
        let after = std::fs::read(dir.join(SIDECAR_FILE)).expect("read");
        assert_eq!(before, after);
        assert_eq!(load(&dir).0, UiLanguagePreference::Explicit("de"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_unavailable_catalog_is_kept_but_renders_english() {
        // `ja` is known but not embedded: the tag survives, the manager
        // (tested in catalog.rs) renders English for it.
        let dir = unique_dir("unavailable");
        save(&dir, &UiLanguagePreference::Explicit("ja")).expect("save ja");
        let (preference, _) = load(&dir);
        assert_eq!(preference, UiLanguagePreference::Explicit("ja"));
        assert_eq!(preference.effective_tag("de"), "ja");
        assert_eq!(UiLanguagePreference::Auto.effective_tag("de"), "de");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

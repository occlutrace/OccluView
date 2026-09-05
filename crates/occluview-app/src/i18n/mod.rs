//! UI language foundation: preference, resolution, catalogs.
//!
//! One [`LocaleManager`] per app frame generation: it owns exactly one
//! complete catalog set. Resolution is atomic per message — a missing
//! message falls back to English for that message alone, so a frame may
//! mix languages on a partial catalog, but never blanks, never panics,
//! never a raw key: the diagnostic marker names the id instead.
//!
//! Startup flow (exactly once, no double `Settings::load`):
//! sidecar preference → OS preferred list → effective tag → embedded
//! catalog or English (tag retained when unavailable).

pub(crate) mod catalog;
#[cfg(test)]
mod fonts;
pub(crate) mod os;
pub(crate) mod preference;
#[cfg(test)]
pub(crate) mod shots;
pub(crate) mod tags;

use std::collections::HashMap;
use std::path::Path;

use catalog::{Catalog, EMBEDDED_TAGS};
use fluent_bundle::FluentArgs;
use preference::{SidecarDiagnostic, UiLanguagePreference};
use tags::FALLBACK_TAG;

/// Marker for the impossible case in a shipped binary (English itself
/// missing a key — an `en` gap fails the build per contract, see
/// `build.rs`). Never blank; carries the id so screenshots and logs stay
/// diagnosable instead of failing silently.
fn missing_marker(id: &str) -> String {
    format!("⟦{id}⟧")
}

/// Native window title. Today the product proper name in every catalog;
/// per-locale descriptors land without code changes once terminology
/// approves them. Resolved live on every switch via `ViewportCommand`.
pub(crate) const NATIVE_TITLE_KEY: &str = "app-window-title";

/// Endonym display names for the language selector. Proper names are never
/// translated; only the `Auto` row label comes from the catalog.
pub(crate) fn endonym(tag: &'static str) -> &'static str {
    match tag {
        "en" => "English",
        "ru" => "Русский",
        "de" => "Deutsch",
        "es" => "Español",
        "fr" => "Français",
        "it" => "Italiano",
        "pt-BR" => "Português (Brasil)",
        "zh-Hans" => "简体中文",
        "ja" => "日本語",
        "ko" => "한국어",
        _ => tag,
    }
}

/// Point-in-time startup result: loaded exactly once and shared, never
/// re-resolved mid-operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupSnapshot {
    /// Stored preference (`Auto` when absent or corrupt).
    pub preference: UiLanguagePreference,
    /// Tag the OS list resolved to (informational under manual choice).
    pub auto_resolved: &'static str,
    /// Tag in effect: explicit choice wins, else `auto_resolved`.
    /// Retained even when no catalog is embedded for it.
    pub active_tag: &'static str,
    /// Embedded tag actually rendering (`en` when `active_tag` is
    /// unavailable).
    pub render_tag: &'static str,
    /// Sidecar diagnostic, if any (locale-invariant code for logs).
    pub diagnostic: Option<SidecarDiagnostic>,
}

/// Single-generation locale state for one app frame.
pub(crate) struct LocaleManager {
    catalogs: HashMap<&'static str, Catalog>,
    fallback: Catalog,
    snapshot: StartupSnapshot,
}

impl LocaleManager {
    /// Build all embedded catalogs. A broken non-English catalog is skipped
    /// (it renders English); a broken English baseline becomes an empty
    /// catalog that renders markers. Both cases are build/CI failures via
    /// `validate_embedded` — this graceful path only protects shipped
    /// binaries from panicking, never from the test gate.
    fn build_all() -> (HashMap<&'static str, Catalog>, Catalog) {
        let mut catalogs = HashMap::new();
        for tag in EMBEDDED_TAGS {
            match Catalog::build(tag) {
                Ok(catalog) => {
                    catalogs.insert(*tag, catalog);
                }
                Err(error) => {
                    tracing::warn!(tag, %error, "embedded catalog is broken; using English");
                }
            }
        }
        let fallback = Catalog::build(FALLBACK_TAG).unwrap_or_else(|error| {
            tracing::error!(%error, "embedded English catalog is broken");
            Catalog::empty_fallback()
        });
        (catalogs, fallback)
    }

    /// Cold-launch entry: preference from `state_dir` (`None` when the
    /// state directory is unavailable), OS list from `source`. Infallible
    /// by design — see `build_all`.
    pub(crate) fn startup(
        state_dir: Option<&Path>,
        source: &dyn os::OsLocaleSource,
    ) -> (Self, StartupSnapshot) {
        let (catalogs, fallback) = Self::build_all();
        let (preference, diagnostic) = match state_dir {
            Some(dir) => preference::load(dir),
            None => (UiLanguagePreference::Auto, None),
        };
        let auto_resolved = os::resolve_with(source);
        let active_tag = preference.effective_tag(auto_resolved);
        let render_tag = if catalogs.contains_key(active_tag) {
            active_tag
        } else {
            FALLBACK_TAG
        };
        let snapshot = StartupSnapshot {
            preference,
            auto_resolved,
            active_tag,
            render_tag,
            diagnostic,
        };
        (
            Self {
                catalogs,
                fallback,
                snapshot: snapshot.clone(),
            },
            snapshot,
        )
    }

    /// Test helper: English manager with no state directory.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        struct Empty;
        impl os::OsLocaleSource for Empty {
            fn preferred_languages(&self) -> Vec<String> {
                Vec::new()
            }
        }
        Self::startup(None, &Empty).0
    }

    /// The snapshot this manager was built from.
    pub(crate) fn snapshot(&self) -> &StartupSnapshot {
        &self.snapshot
    }

    /// Native window title in the active catalog generation.
    pub(crate) fn window_title(&self) -> String {
        self.text(NATIVE_TITLE_KEY)
    }

    /// Manual switch: re-renders all in-app UI immediately from the next
    /// frame. The caller persists via [`preference::save`] on the same path.
    pub(crate) fn set_preference(&mut self, preference: UiLanguagePreference) {
        let active_tag = preference.effective_tag(self.snapshot.auto_resolved);
        self.snapshot.preference = preference;
        self.snapshot.active_tag = active_tag;
        self.snapshot.render_tag = self.render_tag_for(active_tag);
    }

    /// "Apply system language now": re-resolve the live OS list. Only takes
    /// effect while the preference is `Auto`; a manual choice is untouched.
    pub(crate) fn reapply_auto(&mut self, source: &dyn os::OsLocaleSource) {
        self.snapshot.auto_resolved = os::resolve_with(source);
        if self.snapshot.preference == UiLanguagePreference::Auto {
            self.snapshot.active_tag = self.snapshot.auto_resolved;
            self.snapshot.render_tag = self.render_tag_for(self.snapshot.auto_resolved);
        }
    }

    fn render_tag_for(&self, active_tag: &'static str) -> &'static str {
        if self.catalogs.contains_key(active_tag) {
            active_tag
        } else {
            FALLBACK_TAG
        }
    }

    fn active_catalog(&self) -> &Catalog {
        self.catalogs
            .get(self.snapshot.render_tag)
            .unwrap_or(&self.fallback)
    }

    /// Localized text for a message id. Falls back to English per message;
    /// never blank, never a raw key, never panics.
    pub(crate) fn text(&self, id: &str) -> String {
        self.text_with(id, None)
    }

    /// Short alias for `text` at dense call sites (toolbar rows, panels).
    pub(crate) fn tr(&self, id: &str) -> String {
        self.text(id)
    }

    /// `text_with` with inline pairs — no `args()` import at call sites.
    pub(crate) fn tr_with(&self, id: &str, pairs: &[(&str, &str)]) -> String {
        self.text_with(id, Some(&catalog::args(pairs)))
    }

    /// Plural selects with data: string pairs plus `usize` counts.
    pub(crate) fn tr_plural(
        &self,
        id: &str,
        strings: &[(&str, &str)],
        numbers: &[(&str, usize)],
    ) -> String {
        self.text_with(id, Some(&catalog::plural_args(strings, numbers)))
    }

    /// Localized text with Fluent arguments.
    pub(crate) fn text_with(&self, id: &str, args: Option<&FluentArgs<'_>>) -> String {
        if let Some(rendered) = self.active_catalog().format(id, args) {
            return rendered;
        }
        if let Some(rendered) = self.fallback.format(id, args) {
            return rendered;
        }
        missing_marker(id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::catalog::args;
    use super::os::OsLocaleSource;
    use super::*;

    struct Fixed(Vec<&'static str>);

    impl OsLocaleSource for Fixed {
        fn preferred_languages(&self) -> Vec<String> {
            self.0.iter().map(|item| (*item).to_owned()).collect()
        }
    }

    #[test]
    fn auto_cold_launch_resolves_os_language() {
        let source = Fixed(vec!["de-DE"]);
        let (manager, snapshot) = LocaleManager::startup(None, &source);
        assert_eq!(snapshot.preference, UiLanguagePreference::Auto);
        assert_eq!(snapshot.auto_resolved, "de");
        assert_eq!(snapshot.active_tag, "de");
        assert_eq!(snapshot.render_tag, "de");
        assert_eq!(manager.text("settings-language-label"), "Sprache");
    }

    #[test]
    fn unavailable_catalog_renders_english_keeping_tag() {
        // `ja` is known but not embedded (CJK waits for the font spike).
        let source = Fixed(vec!["ja-JP"]);
        let (manager, snapshot) = LocaleManager::startup(None, &source);
        assert_eq!(snapshot.active_tag, "ja");
        assert_eq!(snapshot.render_tag, "en");
        assert_eq!(manager.text("settings-language-label"), "Language");
    }

    #[test]
    fn manual_switch_rerenders_and_outlives_os_change() {
        let source = Fixed(vec!["de-DE"]);
        let (mut manager, _) = LocaleManager::startup(None, &source);
        manager.set_preference(UiLanguagePreference::Explicit("ru"));
        assert_eq!(manager.snapshot().active_tag, "ru");
        assert_eq!(manager.text("settings-language-label"), "Язык");
        // OS list is snapshotted: later OS changes do not leak in.
        assert_eq!(manager.snapshot().auto_resolved, "de");
    }

    #[test]
    fn reapply_auto_refreshes_only_under_auto() {
        let first = Fixed(vec!["de-DE"]);
        let (mut manager, _) = LocaleManager::startup(None, &first);
        manager.reapply_auto(&Fixed(vec!["ru-RU"]));
        assert_eq!(manager.snapshot().active_tag, "ru");
        assert_eq!(manager.text("settings-language-label"), "Язык");

        manager.set_preference(UiLanguagePreference::Explicit("de"));
        manager.reapply_auto(&Fixed(vec!["ru-RU"]));
        assert_eq!(manager.snapshot().auto_resolved, "ru");
        assert_eq!(manager.snapshot().active_tag, "de");
        assert_eq!(manager.text("settings-language-label"), "Sprache");
    }

    #[test]
    fn preference_survives_restart_through_sidecar() {
        let dir = std::env::temp_dir().join(format!(
            "occluview-i18n-restart-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        preference::save(&dir, &UiLanguagePreference::Explicit("ru")).expect("save");
        let source = Fixed(vec!["de-DE"]);
        let (manager, snapshot) = LocaleManager::startup(Some(&dir), &source);
        // Manual beats Auto even though the OS now says German.
        assert_eq!(snapshot.active_tag, "ru");
        assert_eq!(manager.text("settings-language-label"), "Язык");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_message_never_blanks_or_leaks_key() {
        let source = Fixed(vec!["ru"]);
        let (manager, _) = LocaleManager::startup(None, &source);
        let rendered = manager.text("no-such-key");
        assert_eq!(rendered, "⟦no-such-key⟧");
    }

    #[test]
    fn variables_flow_to_the_active_catalog() {
        let source = Fixed(vec!["ru"]);
        let (manager, _) = LocaleManager::startup(None, &source);
        let rendered = manager.text_with("about-version", Some(&args(&[("version", "2.0")])));
        // Bidi isolation marks around the variable are Fluent default.
        assert_eq!(rendered, "Версия \u{2068}2.0\u{2069}");
    }

    #[test]
    fn window_title_resolves_per_generation() {
        let (manager, _) = LocaleManager::startup(None, &Fixed(vec!["fr-FR"]));
        assert_eq!(manager.window_title(), "OccluView 3D Viewer");
        assert_eq!(manager.snapshot().render_tag, "fr");
    }

    #[test]
    fn every_embedded_catalog_renders_its_own_label() {
        for (tag, expected) in [
            ("en", "Language"),
            ("ru", "Язык"),
            ("de", "Sprache"),
            ("es", "Idioma"),
            ("fr", "Langue"),
            ("it", "Lingua"),
            ("pt-BR", "Idioma"),
        ] {
            let (manager, snapshot) = LocaleManager::startup(None, &Fixed(vec![tag]));
            assert_eq!(snapshot.render_tag, tag);
            assert_eq!(manager.text("settings-language-label"), expected);
        }
    }

    #[test]
    fn language_options_cover_every_embedded_catalog() {
        for tag in EMBEDDED_TAGS {
            assert!(!endonym(tag).is_empty(), "missing endonym for {tag}");
        }
        assert_eq!(endonym("en"), "English");
        assert_eq!(endonym("pt-BR"), "Português (Brasil)");
        assert_eq!(endonym("xx"), "xx");
    }

    #[test]
    fn pseudo_spot_checks_bracket_single_line_keys() {
        // Superseded in coverage by `pseudo_locale_covers_every_embedded_key`
        // in catalog.rs; kept as a readable spot check with exact rendering.
        let pseudo = Catalog::pseudo().expect("pseudo builds");
        for key in ["app-title", "help-toggle", "about-tagline"] {
            let rendered = pseudo.text(key).unwrap_or_else(|| key.to_owned());
            assert!(
                rendered.starts_with('⟦'),
                "pseudo must bracket '{key}', got '{rendered}'"
            );
        }
        let rendered = pseudo
            .format("about-version", Some(&args(&[("version", "1")])))
            .expect("formats");
        assert!(rendered.contains('⟦') && rendered.contains('1'));
    }
}

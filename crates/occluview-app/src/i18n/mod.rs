//! UI locale resolution and Fluent catalogs.
//!
//! Startup reads the sidecar preference and OS locale once. Each missing
//! message falls back to English; a missing English key renders `⟦id⟧`.

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

/// Diagnostic marker for a missing English key.
fn missing_marker(id: &str) -> String {
    format!("⟦{id}⟧")
}

/// Render the command modifier with the platform's actual key name.
///
/// The catalogs keep Ctrl/Strg terminology for Windows and Linux; macOS binds
/// these shortcuts to Command, so every localized tooltip and status message
/// uses the native ⌘ glyph instead.
pub(crate) fn platform_shortcut_text(text: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        let mut rendered = text.to_owned();
        for (source, native) in [
            ("Ctrl/Command", "⌘"),
            ("Ctrl/Cmd", "⌘"),
            ("Strg/Command", "⌘"),
            ("Strg/Cmd", "⌘"),
            ("Ctrl", "⌘"),
            ("Strg", "⌘"),
        ] {
            rendered = rendered.replace(source, native);
        }
        rendered
    }
    #[cfg(not(target_os = "macos"))]
    {
        text.to_owned()
    }
}

/// Catalog key for the native window title.
pub(crate) const NATIVE_TITLE_KEY: &str = "app-window-title";

/// Native language names for the selector.
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

/// Resolved locale state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupSnapshot {
    /// Stored preference.
    pub preference: UiLanguagePreference,
    /// Tag resolved from the OS list.
    pub auto_resolved: &'static str,
    /// Effective tag, including unsupported tags.
    pub active_tag: &'static str,
    /// Embedded catalog used to render the UI.
    pub render_tag: &'static str,
    /// Sidecar diagnostic for logs.
    pub diagnostic: Option<SidecarDiagnostic>,
}

/// Locale state for the app.
pub(crate) struct LocaleManager {
    catalogs: HashMap<&'static str, Catalog>,
    fallback: Catalog,
    snapshot: StartupSnapshot,
}

impl LocaleManager {
    /// Builds embedded catalogs; invalid catalogs fall back safely at runtime.
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

    /// Starts from the sidecar preference and an OS locale source.
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

    /// English manager without a state directory.
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

    /// Current locale state.
    pub(crate) fn snapshot(&self) -> &StartupSnapshot {
        &self.snapshot
    }

    /// Catalog that System would render.
    pub(crate) fn system_render_tag(&self) -> &'static str {
        self.render_tag_for(self.snapshot.auto_resolved)
    }

    /// Native window title in the active catalog.
    pub(crate) fn window_title(&self) -> String {
        self.text(NATIVE_TITLE_KEY)
    }

    /// Switches to a manual language.
    pub(crate) fn set_preference(&mut self, preference: UiLanguagePreference) {
        let active_tag = preference.effective_tag(self.snapshot.auto_resolved);
        self.snapshot.preference = preference;
        self.snapshot.active_tag = active_tag;
        self.snapshot.render_tag = self.render_tag_for(active_tag);
    }

    /// Re-reads the OS and selects System.
    pub(crate) fn use_system_language(&mut self, source: &dyn os::OsLocaleSource) {
        let auto_resolved = os::resolve_with(source);
        self.snapshot.preference = UiLanguagePreference::Auto;
        self.snapshot.auto_resolved = auto_resolved;
        self.snapshot.active_tag = auto_resolved;
        self.snapshot.render_tag = self.render_tag_for(auto_resolved);
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

    /// Resolves a message with per-message English fallback.
    pub(crate) fn text(&self, id: &str) -> String {
        self.text_with(id, None)
    }

    /// Short alias for [`Self::text`].
    pub(crate) fn tr(&self, id: &str) -> String {
        self.text(id)
    }

    /// Resolves text with inline string arguments.
    pub(crate) fn tr_with(&self, id: &str, pairs: &[(&str, &str)]) -> String {
        self.text_with(id, Some(&catalog::args(pairs)))
    }

    /// Resolves a plural message with arguments.
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
            return platform_shortcut_text(&rendered);
        }
        if let Some(rendered) = self.fallback.format(id, args) {
            return platform_shortcut_text(&rendered);
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
    fn localized_shortcuts_use_the_host_command_modifier() {
        let (mut manager, _) = LocaleManager::startup(None, &Fixed(vec!["en"]));
        for tag in ["en", "de", "es", "fr", "it", "pt-BR", "ru"] {
            manager.set_preference(UiLanguagePreference::Explicit(tag));
            let hint = manager.text("help-hintline-align");
            if cfg!(target_os = "macos") {
                assert!(
                    hint.contains('⌘'),
                    "{tag} hint did not show Command: {hint}"
                );
                assert!(!hint.contains("Ctrl"), "{tag} hint retained Ctrl: {hint}");
                assert!(!hint.contains("Strg"), "{tag} hint retained Strg: {hint}");
            } else {
                assert!(
                    hint.contains("Ctrl") || hint.contains("Strg"),
                    "{tag} hint lost its PC modifier: {hint}"
                );
                assert!(!hint.contains('⌘'), "{tag} hint showed a Mac glyph: {hint}");
            }
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
    fn using_system_language_refreshes_auto_and_active_catalog() {
        let (mut manager, _) = LocaleManager::startup(None, &Fixed(vec!["ru-RU"]));
        manager.set_preference(UiLanguagePreference::Explicit("de"));

        assert_eq!(manager.system_render_tag(), "ru");

        manager.use_system_language(&Fixed(vec!["ru-RU"]));

        assert_eq!(manager.snapshot().preference, UiLanguagePreference::Auto);
        assert_eq!(manager.snapshot().auto_resolved, "ru");
        assert_eq!(manager.snapshot().active_tag, "ru");
        assert_eq!(manager.snapshot().render_tag, "ru");
    }

    #[test]
    fn using_system_language_keeps_unavailable_tag_but_renders_english() {
        let (mut manager, _) = LocaleManager::startup(None, &Fixed(vec!["de-DE"]));

        manager.use_system_language(&Fixed(vec!["ja-JP"]));

        assert_eq!(manager.snapshot().preference, UiLanguagePreference::Auto);
        assert_eq!(manager.snapshot().auto_resolved, "ja");
        assert_eq!(manager.snapshot().active_tag, "ja");
        assert_eq!(manager.snapshot().render_tag, "en");
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
        // `app-title` was a key nothing resolved; the live title key is
        // `app-window-title` (`NATIVE_TITLE_KEY`).
        for key in ["app-window-title", "settings-shortcuts", "about-tagline"] {
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

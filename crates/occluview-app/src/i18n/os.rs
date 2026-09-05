//! OS locale source abstraction.
//!
//! Tests inject fixed lists through [`OsLocaleSource`], so resolution is
//! hermetic (locale resolution must never depend on ambient CI locale).
//! Production on Linux reads the gettext locale environment with documented
//! precedence; other platforms use `sys-locale`.

/// A provider of the ordered OS preferred UI language list.
pub(crate) trait OsLocaleSource {
    /// Ordered preferred languages, most-preferred first.
    /// Items may use any common shape (`ru-RU`, `ru_RU`, `ru`,
    /// `LANGUAGE`-style `de:fr` lists — split before matching).
    fn preferred_languages(&self) -> Vec<String>;
}

/// Production source.
///
/// On Linux the gettext locale environment is authoritative, in order:
/// `LANGUAGE` (colon-separated priority list) > `LC_ALL` > `LC_MESSAGES` >
/// `LANG`. Elsewhere `sys-locale` provides the preferred list.
pub(crate) struct SystemLocaleSource;

impl OsLocaleSource for SystemLocaleSource {
    fn preferred_languages(&self) -> Vec<String> {
        #[cfg(target_os = "linux")]
        {
            preferred_from_env_or_system(
                linux_preferred_from_env(
                    std::env::var("LANGUAGE").ok(),
                    std::env::var("LC_ALL").ok(),
                    std::env::var("LC_MESSAGES").ok(),
                    std::env::var("LANG").ok(),
                ),
                sys_locale::get_locales().collect(),
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            sys_locale::get_locales().collect()
        }
    }
}

/// Pure Linux environment folding, tested without touching ambient env.
#[cfg(target_os = "linux")]
fn linux_preferred_from_env(
    language: Option<String>,
    lc_all: Option<String>,
    lc_messages: Option<String>,
    lang: Option<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(list) = language {
        out.extend(split_env_list(&list));
    }
    for single in [lc_all, lc_messages, lang].into_iter().flatten() {
        let trimmed = single.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_owned());
        }
    }
    out
}

/// Split a colon-separated `LANGUAGE`-style value, dropping empties.
#[cfg(target_os = "linux")]
fn split_env_list(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Prefer the gettext environment; fall back to `sys-locale` when it
/// yields nothing (bare container, unset locale variables).
#[cfg(target_os = "linux")]
fn preferred_from_env_or_system(env: Vec<String>, system: Vec<String>) -> Vec<String> {
    if env.is_empty() {
        system
    } else {
        env
    }
}

/// Resolve with any source: exact catalog → base mapping → next item → `en`.
pub(crate) fn resolve_with(source: &dyn OsLocaleSource) -> &'static str {
    let preferred = source.preferred_languages();
    super::tags::resolve(&preferred)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    struct Fixed(Vec<&'static str>);

    impl OsLocaleSource for Fixed {
        fn preferred_languages(&self) -> Vec<String> {
            self.0.iter().map(|item| (*item).to_owned()).collect()
        }
    }

    #[test]
    fn system_source_resolves_without_ambient_dependence() {
        // Must return a usable tag on any machine; the exact value depends
        // on ambient OS locale, so only shape is asserted here.
        let tag = resolve_with(&SystemLocaleSource);
        assert!(!tag.is_empty());
    }

    #[test]
    fn injected_source_drives_resolution() {
        let source = Fixed(vec!["pt-PT", "de-DE"]);
        assert_eq!(resolve_with(&source), "de");
        let empty = Fixed(vec![]);
        assert_eq!(resolve_with(&empty), "en");
    }

    #[test]
    fn linux_env_shapes_are_accepted() {
        // `LC_MESSAGES=ru_RU.UTF-8`-style values and Unix `@euro`
        // variants; suffixes are stripped before canonicalization.
        let source = Fixed(vec!["ru_RU.UTF-8", "de"]);
        assert_eq!(resolve_with(&source), "ru");
        let variant = Fixed(vec!["de_DE@euro"]);
        assert_eq!(resolve_with(&variant), "de");
    }

    #[test]
    fn colon_separated_language_lists_fall_through_in_order() {
        // `LANGUAGE=pt_PT:de_DE` must resolve German, not English.
        let source = Fixed(vec!["pt_PT:de_DE"]);
        assert_eq!(resolve_with(&source), "de");
        let source = Fixed(vec!["xx:ru-RU"]);
        assert_eq!(resolve_with(&source), "ru");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_precedence_is_language_then_lc_all_messages_lang() {
        assert_eq!(
            linux_preferred_from_env(
                Some("de:fr".to_owned()),
                Some("ru_RU.UTF-8".to_owned()),
                Some("es_ES.UTF-8".to_owned()),
                Some("it_IT.UTF-8".to_owned()),
            ),
            vec!["de", "fr", "ru_RU.UTF-8", "es_ES.UTF-8", "it_IT.UTF-8"]
        );
        assert_eq!(
            linux_preferred_from_env(None, None, Some("ja_JP.UTF-8".to_owned()), None),
            vec!["ja_JP.UTF-8"]
        );
        assert!(linux_preferred_from_env(None, None, None, None).is_empty());
        assert!(
            linux_preferred_from_env(Some(String::new()), Some("  ".to_owned()), None, None)
                .is_empty()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn empty_env_falls_back_to_system_locales() {
        assert_eq!(
            preferred_from_env_or_system(vec![], vec!["de-DE".to_owned()]),
            vec!["de-DE".to_owned()]
        );
        assert_eq!(
            preferred_from_env_or_system(vec!["fr".to_owned()], vec!["de-DE".to_owned()]),
            vec!["fr".to_owned()]
        );
    }
}

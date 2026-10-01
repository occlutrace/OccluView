#![forbid(unsafe_code)]

//! Shared Fluent catalogs and locale resolution for the app and shell.

pub mod tags;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// Catalogs shipped with the app. The app and Explorer shell resolve these same files.
pub const EMBEDDED_TAGS: &[&str] = &["en", "ru", "de", "es", "fr", "it", "pt-BR"];

/// Fluent source shared by the app and shell, in [`EMBEDDED_TAGS`] order.
pub const EMBEDDED_SOURCES: &[(&str, &str)] = &[
    ("en", include_str!("../i18n/en.ftl")),
    ("ru", include_str!("../i18n/ru.ftl")),
    ("de", include_str!("../i18n/de.ftl")),
    ("es", include_str!("../i18n/es.ftl")),
    ("fr", include_str!("../i18n/fr.ftl")),
    ("it", include_str!("../i18n/it.ftl")),
    ("pt-BR", include_str!("../i18n/pt-BR.ftl")),
];

/// Return the embedded Fluent source for a canonical catalog tag.
#[must_use]
pub fn embedded_source(tag: &str) -> Option<&'static str> {
    EMBEDDED_SOURCES
        .iter()
        .find_map(|(known, source)| (*known == tag).then_some(*source))
}

/// One compiled Fluent catalog.
pub struct Catalog {
    tag: &'static str,
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    /// Build a catalog for an embedded tag.
    ///
    /// # Errors
    /// Returns an error if the tag is not embedded or its catalog cannot be parsed.
    pub fn build(tag: &'static str) -> Result<Self, String> {
        let source =
            embedded_source(tag).ok_or_else(|| format!("no embedded catalog for '{tag}'"))?;
        Self::from_source(tag, source)
    }

    /// Build a catalog from a Fluent source string.
    ///
    /// # Errors
    /// Returns an error if the tag is invalid or the source cannot be parsed.
    pub fn from_source(tag: &'static str, source: &str) -> Result<Self, String> {
        let langid: LanguageIdentifier = tag.parse().map_err(|_| format!("bad tag '{tag}'"))?;
        let resource = FluentResource::try_new(source.to_owned())
            .map_err(|(_, errors)| join_errors(&errors))?;
        let mut bundle = FluentBundle::new(vec![langid]);
        bundle
            .add_resource(resource)
            .map_err(|errors| join_errors(&errors))?;
        Ok(Self { tag, bundle })
    }

    /// Create an empty English bundle for the broken-catalog fallback path.
    #[must_use]
    pub fn empty_fallback() -> Self {
        let langid: LanguageIdentifier = tags::FALLBACK_TAG.parse().unwrap_or_default();
        Self {
            tag: tags::FALLBACK_TAG,
            bundle: FluentBundle::new(vec![langid]),
        }
    }

    /// The embedded tag used to build this catalog.
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        self.tag
    }

    /// Format a message id (`key` or `key.attribute`).
    #[must_use]
    pub fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let (head, attribute) = match id.split_once('.') {
            Some((head, attribute)) => (head, Some(attribute)),
            None => (id, None),
        };
        let message = self.bundle.get_message(head)?;
        let pattern = match attribute {
            Some(name) => message.get_attribute(name)?.value(),
            None => message.value()?,
        };
        let mut errors = Vec::new();
        let value = self.bundle.format_pattern(pattern, args, &mut errors);
        errors.is_empty().then(|| value.into_owned())
    }

    /// Format a message id without arguments.
    #[must_use]
    pub fn text(&self, id: &str) -> Option<String> {
        self.format(id, None)
    }
}

/// A system-selected catalog with per-message English fallback.
pub struct LocalizedCatalog {
    active: Catalog,
    english: Catalog,
}

impl LocalizedCatalog {
    /// Build the catalog selected by the operating system's UI language list.
    #[must_use]
    pub fn for_system() -> Self {
        let preferred = sys_locale::get_locales().collect::<Vec<_>>();
        Self::for_tag(tags::resolve(&preferred))
    }

    /// Build a catalog for a requested language tag, falling back per message to English.
    #[must_use]
    pub fn for_tag(tag: &str) -> Self {
        let resolved = tags::resolve(&[tag]);
        let active_tag = if EMBEDDED_TAGS.contains(&resolved) {
            resolved
        } else {
            tags::FALLBACK_TAG
        };
        let active = build_or_fallback(active_tag);
        let english = build_or_fallback(tags::FALLBACK_TAG);
        Self { active, english }
    }

    /// Format a localized message, using English when the active catalog lacks it.
    #[must_use]
    pub fn text(&self, id: &str) -> Option<String> {
        self.active.text(id).or_else(|| self.english.text(id))
    }
}

fn build_or_fallback(tag: &'static str) -> Catalog {
    match Catalog::build(tag) {
        Ok(catalog) => catalog,
        Err(_) => Catalog::empty_fallback(),
    }
}

/// Build Fluent string arguments from key/value pairs.
#[must_use]
pub fn args<'a>(pairs: &[(&'a str, &'a str)]) -> FluentArgs<'a> {
    let mut result = FluentArgs::new();
    for (key, value) in pairs {
        result.set(*key, FluentValue::from(*value));
    }
    result
}

/// Convert a count to a Fluent numeric value without wrapping.
#[must_use]
pub fn count(value: usize) -> FluentValue<'static> {
    FluentValue::from(i64::try_from(value).unwrap_or(i64::MAX))
}

/// Build Fluent arguments with string and plural-count values.
#[must_use]
pub fn plural_args<'a>(
    strings: &[(&'a str, &'a str)],
    numbers: &[(&'a str, usize)],
) -> FluentArgs<'a> {
    let mut result = args(strings);
    for (key, value) in numbers {
        result.set(*key, count(*value));
    }
    result
}

fn join_errors<T: std::fmt::Display>(errors: &[T]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

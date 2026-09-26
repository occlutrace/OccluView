//! BCP-47 canonicalization and `Auto` language resolution.
//!
//! Pure logic over caller-provided preferred-language lists, so every rule
//! is unit-testable without touching the OS. The production OS adapter
//! lives in `super::os`.

/// Fallback tag: plain English, always available.
pub(crate) const FALLBACK_TAG: &str = "en";

/// Canonical tags the resolver may return.
/// Only a subset is embedded — see `super::catalog::EMBEDDED_TAGS`.
/// Unavailable tags resolve here but render English (tag retained).
pub(crate) const KNOWN_TAGS: &[&str] = &[
    "en", "ru", "de", "es", "fr", "it", "pt-BR", "zh-Hans", "ja", "ko",
];

/// Tags explicitly refused even though they look structurally valid.
const DENYLIST: &[&str] = &["c", "posix"];

/// Canonicalize a raw locale identifier to BCP-47 shape.
///
/// Accepts `-`/`_` separators and any letter case (`ru_Cyrl_RU` →
/// `ru-Cyrl-RU`). Returns `None` for malformed input, `C`/`POSIX`, and
/// private-use tags. Never panics.
pub(crate) fn canonicalize(raw: &str) -> Option<String> {
    // Strip Unix locale suffixes (`ru_RU.UTF-8`, `de_DE@euro`) first:
    // they are platform conventions, not part of BCP-47.
    let without_suffix = raw.split(['.', '@']).next().unwrap_or_default();
    let mut parts: Vec<&str> = without_suffix
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .collect();
    // Drop extension/private-use tails at the first single-character
    // subtag (`de-DE-u-co-phonebk` → `de-DE`): singletons open BCP-47
    // extension sequences, never the language itself (min two letters).
    // A leading singleton (`x-private`) truncates to nothing → `None`.
    if let Some(cut) = parts.iter().position(|part| part.len() == 1) {
        parts.truncate(cut);
    }
    let normalized: String = parts.join("-");
    if normalized.is_empty() || normalized.len() > 35 {
        return None;
    }
    let mut parts = normalized.split('-');
    let language = parts.next()?;
    if !(2..=8).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = String::with_capacity(normalized.len());
    out.push_str(&language.to_ascii_lowercase());
    for part in parts {
        if part.len() < 2 || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        out.push('-');
        if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars.next()?;
            out.push(first.to_ascii_uppercase());
            out.push_str(&chars.as_str().to_ascii_lowercase());
        } else if part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            out.push_str(&part.to_ascii_uppercase());
        } else {
            out.push_str(&part.to_ascii_lowercase());
        }
    }
    if out.starts_with("x-") || DENYLIST.contains(&out.as_str()) {
        return None;
    }
    Some(out)
}

/// Split a canonical tag into (language, script, region).
fn split_tag(tag: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default();
    let mut script = None;
    let mut region = None;
    for part in parts {
        if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            script = Some(part);
        } else if (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))
        {
            region = Some(part);
        }
    }
    (language, script, region)
}

/// Map one canonical tag to a known catalog tag.
/// Returns `None` when this item must not select anything
/// (caller continues with the next preferred language).
fn map_canonical(tag: &str) -> Option<&'static str> {
    if let Some(known) = KNOWN_TAGS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(tag))
    {
        return Some(known);
    }
    let (language, script, region) = split_tag(tag);
    match language.to_ascii_lowercase().as_str() {
        "en" => Some("en"),
        "ru" => Some("ru"),
        "de" => Some("de"),
        "es" => Some("es"),
        "fr" => Some("fr"),
        "it" => Some("it"),
        "pt" => {
            if region.is_some_and(|region| region.eq_ignore_ascii_case("BR")) {
                Some("pt-BR")
            } else {
                // Never substitute Brazilian Portuguese for Portugal/Angola.
                None
            }
        }
        "zh" => {
            if script.is_some_and(|script| script.eq_ignore_ascii_case("hans")) {
                return Some("zh-Hans");
            }
            if script.is_some_and(|script| script.eq_ignore_ascii_case("hant")) {
                // Traditional never falls back to Simplified.
                return None;
            }
            // No script: only explicit Hans regions resolve. Hant regions,
            // bare `zh`, and unknown regions never guess — try next item.
            match region.map(str::to_ascii_uppercase).as_deref() {
                Some("CN" | "SG") => Some("zh-Hans"),
                Some(_) | None => None,
            }
        }
        "ja" => Some("ja"),
        "ko" => Some("ko"),
        _ => None,
    }
}

/// Resolve an ordered OS preferred-language list to a canonical tag.
///
/// Tries each item as exact catalog → approved base mapping, then the next
/// item. Falls back to `en` only after the list is exhausted. Never uses
/// IP/geolocation; never guesses regional equivalence.
pub(crate) fn resolve(preferred: &[impl AsRef<str>]) -> &'static str {
    for raw in preferred {
        // Colon-separated `LANGUAGE`-style lists survive into any source;
        // try each member in order. `:` is never valid BCP-47.
        for member in raw.as_ref().split(':') {
            if let Some(canonical) = canonicalize(member) {
                if let Some(mapped) = map_canonical(&canonical) {
                    return mapped;
                }
            }
        }
    }
    FALLBACK_TAG
}

/// Validate a persisted explicit tag: canonical shape + known catalog.
pub(crate) fn validate_explicit(raw: &str) -> Option<&'static str> {
    let canonical = canonicalize(raw.trim())?;
    KNOWN_TAGS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(&canonical))
        .copied()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn required_mapping_table() {
        let cases: &[(&[&str], &str)] = &[
            (&["ru-RU"], "ru"),
            (&["ru_Cyrl_RU"], "ru"),
            (&["ru_RU.UTF-8"], "ru"),
            (&["de_DE@euro"], "de"),
            (&["ru"], "ru"),
            (&["de-DE"], "de"),
            (&["de-AT"], "de"),
            (&["de-CH"], "de"),
            (&["es-ES"], "es"),
            (&["es-MX"], "es"),
            (&["es-419"], "es"),
            (&["fr-FR"], "fr"),
            (&["fr-CA"], "fr"),
            (&["it-IT"], "it"),
            (&["it-CH"], "it"),
            (&["pt-BR"], "pt-BR"),
            (&["pt"], "en"),
            (&["pt-PT"], "en"),
            (&["pt-AO"], "en"),
            (&["zh-Hans"], "zh-Hans"),
            (&["zh-CN"], "zh-Hans"),
            (&["zh-SG"], "zh-Hans"),
            (&["zh-Hans-CN"], "zh-Hans"),
            (&["zh-Hant"], "en"),
            (&["zh-TW"], "en"),
            (&["zh-HK"], "en"),
            (&["zh-MO"], "en"),
            (&["zh"], "en"),
            (&["ja-JP"], "ja"),
            (&["ko-KR"], "ko"),
            (&["en-US"], "en"),
            (&["en-GB"], "en"),
            (&["de-DE-u-co-phonebk"], "de"),
            (&["en-US-u-va-posix"], "en"),
            (&["xx-YY"], "en"),
            (&[""], "en"),
            (&["C"], "en"),
            (&["POSIX"], "en"),
            (&["posix"], "en"),
            (&["x-private"], "en"),
            (&["not a locale!!"], "en"),
        ];
        for (preferred, expected) in cases {
            assert_eq!(resolve(preferred), *expected, "input: {preferred:?}");
        }
    }

    #[test]
    fn preferred_list_falls_through_to_next_item() {
        assert_eq!(resolve(&["pt-PT", "de-DE"]), "de");
        assert_eq!(resolve(&["en-US", "de-DE"]), "en");
        assert_eq!(resolve(&["zh-TW", "ja-JP"]), "ja");
        assert_eq!(resolve(&["xx", "fr-CA"]), "fr");
        assert_eq!(resolve(&["pt-PT", "zh-Hant"]), "en");
    }

    #[test]
    fn manual_override_validation() {
        assert_eq!(validate_explicit("auto"), None);
        assert_eq!(validate_explicit("ru"), Some("ru"));
        assert_eq!(validate_explicit("pt-BR"), Some("pt-BR"));
        assert_eq!(validate_explicit("ZH-hans"), Some("zh-Hans"));
        assert_eq!(validate_explicit("pt-PT"), None);
        assert_eq!(validate_explicit("klingon"), None);
        assert_eq!(validate_explicit(""), None);
    }
}

//! Default-font coverage for the embedded catalog scripts.
//!
//! Test-only: asserts what the eframe `default_fonts` stack (Ubuntu-Light
//! proportional, Hack monospace, Noto Emoji) can and cannot render. CJK
//! locales need a redistributable CJK font, which the stack does not carry.
//! Uses dev-dependencies only; ships nothing.

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use epaint_default_fonts::{HACK_REGULAR, NOTO_EMOJI_REGULAR, UBUNTU_LIGHT};
    use skrifa::charmap::Charmap;

    fn has_glyph(font: &[u8], ch: char) -> bool {
        let Ok(face) = skrifa::FontRef::new(font) else {
            return false;
        };
        Charmap::new(&face).map(ch).is_some()
    }

    fn missing(font: &[u8], text: &str) -> String {
        text.chars().filter(|ch| !has_glyph(font, *ch)).collect()
    }

    /// The embedded catalogs' scripts render in the proportional default font.
    #[test]
    fn default_proportional_font_covers_catalog_scripts() {
        // Cyrillic block + ё/Ё.
        let cyrillic: String = ('\u{410}'..='\u{44f}').collect();
        assert_eq!(missing(UBUNTU_LIGHT, &cyrillic), String::new());
        // German/French accented Latin.
        assert_eq!(
            missing(UBUNTU_LIGHT, "äöüÄÖÜßàâçéèêëîïôûùÿñæœ",),
            String::new()
        );
        // Punctuation used by the embedded catalogs.
        assert_eq!(missing(UBUNTU_LIGHT, "—–…·«»„“"), String::new());
    }

    /// Monospace UI text (numbers, IDs) renders in the monospace default font.
    #[test]
    fn monospace_font_covers_latin_and_digits() {
        assert_eq!(missing(HACK_REGULAR, "AZaz09.,:%×→—…"), String::new());
    }

    /// The default fonts have no CJK glyphs, so zh-Hans, ja and ko need a
    /// bundled CJK font (redistribution licence, Han unification, package
    /// size). Bundling one changes this test to assert the new coverage.
    #[test]
    fn cjk_is_missing_from_default_fonts() {
        for probe in ['\u{4e2d}', '\u{65e5}', '\u{ac00}'] {
            let sample = probe.to_string();
            assert!(
                missing(UBUNTU_LIGHT, &sample) == sample,
                "unexpected CJK coverage for {probe:?}"
            );
        }
        // Emoji fallback exists for filenames, not for CJK.
        assert!(has_glyph(NOTO_EMOJI_REGULAR, '\u{1f600}'));
    }
}

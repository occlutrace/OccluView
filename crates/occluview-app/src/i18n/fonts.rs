//! Default-font coverage proof for the pilot scripts.
//!
//! Test-only: asserts what the eframe `default_fonts` stack (Ubuntu-Light
//! proportional, Hack monospace, Noto Emoji) can and cannot render, so the
//! CJK wave stays explicitly blocked until redistributable fonts land.
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

    /// Pilot scripts render in the proportional default font.
    #[test]
    fn default_proportional_font_covers_pilot_scripts() {
        // Cyrillic block + ё/Ё.
        let cyrillic: String = ('\u{410}'..='\u{44f}').collect();
        assert_eq!(missing(UBUNTU_LIGHT, &cyrillic), String::new());
        // German/French accented Latin.
        assert_eq!(
            missing(UBUNTU_LIGHT, "äöüÄÖÜßàâçéèêëîïôûùÿñæœ",),
            String::new()
        );
        // The pilot catalogs' own punctuation.
        assert_eq!(missing(UBUNTU_LIGHT, "—–…·«»„“"), String::new());
    }

    /// Monospace UI (numbers, IDs) survives the pilot too.
    #[test]
    fn monospace_font_covers_latin_and_digits() {
        assert_eq!(missing(HACK_REGULAR, "AZaz09.,:%×→—…"), String::new());
    }

    /// CJK is NOT covered: proof that zh-Hans/ja/ko stay blocked behind
    /// the font spike (redistributable fonts, licensing, Han unification,
    /// package size). When that spike lands, this test is updated — not
    /// deleted — to assert the new coverage.
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

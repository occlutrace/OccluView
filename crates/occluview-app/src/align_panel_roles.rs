//! Which scan moves onto which, said out loud in the panel.
//!
//! The tool decides this from the operator's first click, and with two scans in
//! view it guesses beforehand so a plain comparison needs no clicks at all. A
//! guess that nothing on screen states is a guess the operator finds out about
//! from the result: an arch that jumped when they expected the other one to.
//!
//! So the line below always names both scans in the direction the fit will run,
//! and offers one button to turn it around.

use eframe::egui;

use crate::mesh_editor_icons::EditorIcon;
use crate::ui_theme;

/// Longest a scan's name is shown at. Both names and the button have to fit one
/// line of a 272-pixel window, and a file name can be arbitrarily long.
const NAME_BUDGET: usize = 13;

/// The two scans of the pair, as the operator's own file names.
pub(crate) struct AlignRoles {
    /// Name of the scan that will move.
    pub(crate) moving: String,
    /// Name of the scan that stays put.
    pub(crate) fixed: String,
    /// Whether this is still the arm-time guess rather than the operator's own
    /// choice. Only the wording changes; the swap works either way.
    pub(crate) implied: bool,
}

impl AlignRoles {
    /// The sentence for the panel.
    pub(crate) fn sentence(&self) -> String {
        let moving = shorten(&self.moving);
        let fixed = shorten(&self.fixed);
        if self.implied {
            format!("{moving} → {fixed} (a guess)")
        } else {
            format!("{moving} → {fixed}")
        }
    }

    /// What the row explains on hover.
    pub(crate) fn hint(&self) -> String {
        let head = if self.implied {
            "Nothing clicked yet, so the tool guessed from the order the files were opened. Your first click decides it: "
        } else {
            ""
        };
        format!(
            "{head}{} moves, {} stays put",
            self.moving.trim(),
            self.fixed.trim()
        )
    }
}

/// Cut a name down to the budget, keeping the end.
///
/// The end is what tells two scans apart — `patient_2026_07_29_lower.stl` and
/// `patient_2026_07_29_upper.stl` differ in their last six characters, so a
/// front-anchored truncation would print the same string twice.
fn shorten(name: &str) -> String {
    let name = name.trim();
    let characters: Vec<char> = name.chars().collect();
    if characters.len() <= NAME_BUDGET {
        return name.to_owned();
    }
    let tail: String = characters[characters.len() - (NAME_BUDGET - 1)..]
        .iter()
        .collect();
    format!("…{tail}")
}

/// Draw the row. Returns whether the operator asked to turn the pair around.
pub(crate) fn show(ui: &mut egui::Ui, roles: Option<&AlignRoles>, enabled: bool) -> bool {
    let Some(roles) = roles else {
        return false;
    };
    let mut swap = false;
    ui.horizontal(|ui| {
        let button_width = 62.0;
        let text_width =
            (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui(
            egui::vec2(text_width, crate::align_panel::CHIP_HEIGHT),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(roles.sentence()).size(11.5).color(
                        if roles.implied {
                            ui_theme::TEXT_MUTED
                        } else {
                            ui_theme::TEXT
                        },
                    ))
                    .on_hover_text(roles.hint());
                });
            },
        );
        swap = crate::align_panel::chip(
            ui,
            button_width,
            Some(EditorIcon::Redo),
            "Swap",
            enabled,
            false,
        )
        .on_hover_text("Fit the other way round — the arrows move with it")
        .clicked();
    });
    ui.add_space(2.0);
    swap
}

#[cfg(test)]
mod tests {
    use super::{shorten, AlignRoles, NAME_BUDGET};

    fn roles(implied: bool) -> AlignRoles {
        AlignRoles {
            moving: "lower.stl".to_owned(),
            fixed: "upper.stl".to_owned(),
            implied,
        }
    }

    /// The direction is stated in the direction the fit runs, both ways round.
    #[test]
    fn the_row_names_the_scan_that_moves_first() {
        assert_eq!(roles(false).sentence(), "lower.stl → upper.stl");
        assert!(roles(false).hint().starts_with("lower.stl moves"));
    }

    /// A guess says it is a guess. It used to say nothing at all, and then the
    /// operator learnt about it from an arch that jumped the wrong way.
    #[test]
    fn a_guess_admits_to_being_one() {
        let guessed = roles(true).sentence();
        assert!(
            guessed.contains("guess"),
            "the operator has to be told this is not their choice yet, got {guessed}"
        );
        assert!(roles(true).hint().contains("first click decides"));
    }

    /// Two scans from one case differ at the END of the name, so that is the
    /// end that has to survive. Front-anchored truncation printed the same
    /// string for both.
    #[test]
    fn a_long_name_keeps_the_part_that_tells_two_scans_apart() {
        let lower = shorten("patient_2026_07_29_lower.stl");
        let upper = shorten("patient_2026_07_29_upper.stl");
        assert_ne!(lower, upper, "both scans printed as the same name");
        assert!(lower.ends_with("lower.stl"), "got {lower}");
        assert!(upper.ends_with("upper.stl"), "got {upper}");
    }

    /// Whatever comes in, the row stays inside its line.
    #[test]
    fn every_name_fits_the_budget() {
        for name in [
            "",
            "a",
            "lower.stl",
            "exactly13char",
            "a_rather_long_scan_file_name.stl",
            "мандибула_нижняя_челюсть_скан.stl",
        ] {
            let shown = shorten(name);
            assert!(
                shown.chars().count() <= NAME_BUDGET,
                "{name} came out as {shown}, {} characters",
                shown.chars().count()
            );
        }
    }

    /// A name that already fits is printed as it is, with no ellipsis added.
    #[test]
    fn a_short_name_is_left_alone() {
        assert_eq!(shorten("upper.stl"), "upper.stl");
        assert_eq!(shorten("  upper.stl  "), "upper.stl");
    }
}

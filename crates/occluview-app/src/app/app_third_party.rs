//! The Third-party licenses window: the generated THIRD-PARTY-NOTICES.md,
//! readable in place from the About dialog.

use super::OccluViewApp;
use crate::ui_theme;
use std::sync::OnceLock;

/// The attribution file the artifacts ship, embedded verbatim. `include_str!`
/// ties the app's rebuild to regeneration, so this window and the installed
/// file cannot disagree — and a missing file is a compile error, not a
/// shipping surprise.
const THIRD_PARTY_NOTICES: &str = include_str!("../../../../THIRD-PARTY-NOTICES.md");

/// The notices split once into lines. A quarter-megabyte in one label would
/// re-lay-out every frame; `show_rows` over pre-split lines paints only the
/// visible slice.
fn notice_lines() -> &'static [&'static str] {
    static LINES: OnceLock<Vec<&'static str>> = OnceLock::new();
    LINES.get_or_init(|| THIRD_PARTY_NOTICES.lines().collect())
}

impl OccluViewApp {
    pub(super) fn show_third_party_window(&mut self, ctx: &egui::Context) {
        if !self.third_party_window_open {
            return;
        }
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let viewport = ctx.content_rect();
        egui::Window::new("Third-party licenses")
            .default_pos(viewport.center() - egui::vec2(280.0, 210.0))
            .constrain_to(viewport)
            .resizable(true)
            .collapsible(false)
            .title_bar(false)
            .default_size([560.0, 420.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Third-party licenses")
                            .strong()
                            .color(ui_theme::TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                let lines = notice_lines();
                // Measured from the same 11 pt font the rows paint with; the
                // Monospace style's default height is taller, and show_rows
                // anchors scrolling by the declared height, so a mismatch
                // slides the text against the scrollbar and leaves a blank
                // band at the bottom of a six-thousand-line document.
                let row_height =
                    ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::monospace(11.0)));
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, lines.len(), |ui, rows| {
                        // License texts are preformatted; wrapping them would
                        // mangle the alignment the horizontal scrollbar exists
                        // for.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        for line in &lines[rows] {
                            ui.label(
                                egui::RichText::new(*line)
                                    .monospace()
                                    .size(11.0)
                                    .color(ui_theme::TEXT_WEAK),
                            );
                        }
                    });
            });

        if close {
            self.third_party_window_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_notices_are_embedded_and_carry_the_font_licenses() {
        assert!(
            THIRD_PARTY_NOTICES.len() > 10_000,
            "the embedded notices are implausibly small"
        );
        assert!(THIRD_PARTY_NOTICES.contains("SIL OPEN FONT LICENSE"));
        assert!(THIRD_PARTY_NOTICES.contains("UBUNTU FONT LICENCE"));
    }

    #[test]
    fn no_first_party_crate_attributes_itself() {
        assert!(
            !notice_lines()
                .iter()
                .any(|line| line.starts_with("- occluview")),
            "a first-party crate leaked into the third-party notices"
        );
    }
}

//! The Third-party licenses modal: the generated THIRD-PARTY-NOTICES.md,
//! readable in place from the About dialog.

use super::app_settings_window::show_information_modal;
use super::information_dialog::InformationDialog;
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
        if self.information_dialog != InformationDialog::ThirdPartyNotices {
            return;
        }
        let mut close = false;
        let modal_response = show_information_modal(
            ctx,
            egui::Id::new("occluview-third-party-notices-v2"),
            egui::vec2(560.0, 420.0),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Third-party licenses")
                            .strong()
                            .color(ui_theme::text()),
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
                                    .color(ui_theme::text_weak()),
                            );
                        }
                    });
            },
        );

        if close || modal_response.should_close() {
            self.information_dialog = InformationDialog::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 384.0))
    }

    fn modal_escape_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (bool, bool) {
        let input = egui::RawInput {
            screen_rect: Some(test_screen()),
            events,
            ..Default::default()
        };
        let mut lower_should_close = false;
        let mut upper_should_close = false;
        ctx.run_ui(input, |ui| {
            lower_should_close = egui::Modal::new(egui::Id::new("lower-modal-escape-contract"))
                .show(ui.ctx(), |ui| {
                    ui.set_min_size(egui::vec2(120.0, 72.0));
                })
                .should_close();
            upper_should_close = egui::Modal::new(egui::Id::new("upper-modal-escape-contract"))
                .show(ui.ctx(), |ui| {
                    ui.set_min_size(egui::vec2(160.0, 96.0));
                })
                .should_close();
        })
        .drop_without_applying_deltas();
        (lower_should_close, upper_should_close)
    }

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

    #[test]
    fn modal_response_assigns_escape_to_the_topmost_dialog() {
        let ctx = egui::Context::default();
        assert_eq!(modal_escape_frame(&ctx, Vec::new()), (false, false));

        let (lower_should_close, upper_should_close) = modal_escape_frame(
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            !lower_should_close,
            "Escape must not reach a dialog behind the topmost modal"
        );
        assert!(
            upper_should_close,
            "Escape must close the topmost modal through ModalResponse::should_close"
        );
    }
}

//! Post-repair report card.
//!
//! One-click Repair already drops a one-line status toast (the executor in
//! `app/app_layer_edits/repair.rs`). This module adds the calm, persistent
//! *card* the operator reads to SEE what the pass actually did: one human line
//! per non-zero pass with a small painted icon, the informational open-rim
//! tail, and a positive "nothing to repair" confirmation when the mesh was
//! already clean (matching the dental CAD convention — a clean scan still
//! gets an answer).
//!
//! The kernel [`RepairReport`] is consumed read-only; every number shown here
//! is one of its existing fields. Presentation only — no mesh logic lives here.

use eframe::egui;
use occluview_core::RepairReport;

use crate::icons::AppIcon;
use crate::modal_surface::show_information_modal;
use crate::ui_theme;

/// Headline shown when Repair ran but found nothing to fix.
pub(crate) const CLEAN_HEADLINE: &str = "Nothing to repair — mesh is clean";

/// The small glyph painted in a report line's gutter. Removals borrow the
/// editor's trash glyph, closed pinholes borrow the close-holes glyph, and
/// pure topology fixes get a neutral accent check ("done").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineIcon {
    /// Something was deleted (slivers, duplicate/debris faces, unused verts).
    Removed,
    /// A boundary rim was capped.
    Closed,
    /// A topology repair with no natural editor glyph (weld / non-manifold /
    /// bowtie / reorientation / flip).
    Fixed,
}

/// One rendered report line: a painted icon plus a finished human sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReportLine {
    /// Gutter glyph.
    pub(crate) icon: LineIcon,
    /// "Verb N noun" with grouped digits and correct singular/plural.
    pub(crate) text: String,
}

/// One human line per non-zero pass, in pipeline order. Zero passes are
/// suppressed, so an all-clean report yields an empty vector (the caller shows
/// [`CLEAN_HEADLINE`] instead).
#[must_use]
pub(crate) fn report_lines(report: &RepairReport) -> Vec<ReportLine> {
    // (icon, count, verb, singular, plural) in pipeline order. Zero counts are
    // filtered out; digits over 999 are grouped for readability ("1 240").
    let passes: [(LineIcon, usize, &str, &str, &str); 10] = [
        (
            LineIcon::Fixed,
            report.welded_vertices,
            "Welded",
            "duplicate vertex",
            "duplicate vertices",
        ),
        (
            LineIcon::Removed,
            report.removed_degenerate_triangles,
            "Removed",
            "sliver face",
            "sliver faces",
        ),
        (
            LineIcon::Removed,
            report.removed_duplicate_triangles,
            "Removed",
            "duplicate face",
            "duplicate faces",
        ),
        (
            LineIcon::Fixed,
            report.split_nonmanifold_edges,
            "Fixed",
            "non-manifold edge",
            "non-manifold edges",
        ),
        (
            LineIcon::Fixed,
            report.split_bowtie_vertices,
            "Split",
            "bowtie vertex",
            "bowtie vertices",
        ),
        (
            LineIcon::Fixed,
            report.reoriented_triangles,
            "Reoriented",
            "triangle",
            "triangles",
        ),
        (
            LineIcon::Fixed,
            report.flipped_components,
            "Flipped",
            "inside-out part",
            "inside-out parts",
        ),
        (
            LineIcon::Removed,
            report.removed_debris_components,
            "Removed",
            "debris part",
            "debris parts",
        ),
        (
            LineIcon::Closed,
            report.filled_holes,
            "Closed",
            "pinhole",
            "pinholes",
        ),
        (
            LineIcon::Removed,
            report.removed_unreferenced_vertices,
            "Removed",
            "unused vertex",
            "unused vertices",
        ),
    ];
    passes
        .into_iter()
        .filter(|&(_, count, ..)| count > 0)
        .map(|(icon, count, verb, singular, plural_noun)| ReportLine {
            icon,
            text: format!(
                "{verb} {} {}",
                group_thousands(count),
                plural(count, singular, plural_noun)
            ),
        })
        .collect()
}

/// Informational line for rims left open on purpose (the scan's natural
/// boundary is not damage). `None` when every rim was closed or oversized-none.
#[must_use]
pub(crate) fn open_rims_line(report: &RepairReport) -> Option<String> {
    let rims = report.open_rims_left;
    (rims > 0).then(|| {
        format!(
            "{} open {} left (scan boundary)",
            group_thousands(rims),
            plural(rims, "rim", "rims")
        )
    })
}

/// Informational line for rims the fill pass refused because they were not
/// simple closed loops. `None` when there were no such warnings.
#[must_use]
pub(crate) fn skipped_rims_line(report: &RepairReport) -> Option<String> {
    let count = report.warnings.len();
    (count > 0).then(|| {
        format!(
            "{count} {} could not be filled (non-simple)",
            plural(count, "rim", "rims")
        )
    })
}

/// Full per-pass dump for the clipboard: before/after counts and every pass
/// including the zeros, so a support ticket carries the whole picture.
#[must_use]
pub(crate) fn copy_details(layer_label: &str, report: &RepairReport) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "Repair report — {layer_label}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Before: {} vertices, {} triangles",
        group_thousands(report.input_vertices),
        group_thousands(report.input_triangles)
    );
    let _ = writeln!(
        out,
        "After:  {} vertices, {} triangles",
        group_thousands(report.output_vertices),
        group_thousands(report.output_triangles)
    );
    let _ = writeln!(out);

    let rows: [(&str, usize); 13] = [
        ("Welded duplicate vertices", report.welded_vertices),
        ("Removed sliver faces", report.removed_degenerate_triangles),
        (
            "Removed duplicate faces",
            report.removed_duplicate_triangles,
        ),
        ("Fixed non-manifold edges", report.split_nonmanifold_edges),
        ("Split bowtie vertices", report.split_bowtie_vertices),
        ("Reoriented triangles", report.reoriented_triangles),
        ("Flipped inside-out parts", report.flipped_components),
        ("Removed debris parts", report.removed_debris_components),
        ("Removed debris faces", report.removed_debris_triangles),
        ("Closed pinholes", report.filled_holes),
        (
            "Removed unused vertices",
            report.removed_unreferenced_vertices,
        ),
        ("Open rims left (scan boundary)", report.open_rims_left),
        ("Rims skipped (non-simple)", report.warnings.len()),
    ];
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, count) in rows {
        let _ = writeln!(out, "{label:<width$}  {}", group_thousands(count));
    }
    out
}

/// Group digits into thousands with a plain space ("1240" -> "1 240"). Locale
/// neutral and copy-paste friendly; small counts pass through unchanged.
#[must_use]
fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(char::from(*byte));
    }
    out
}

/// Singular or plural noun for `count`.
fn plural<'a>(count: usize, singular: &'a str, plural_noun: &'a str) -> &'a str {
    if count == 1 {
        singular
    } else {
        plural_noun
    }
}

/// The card currently on screen.
struct Card {
    /// Layer the repair ran on (the card title).
    layer_label: String,
    /// The kernel report, consumed read-only.
    report: RepairReport,
}

/// State machine for the post-repair report card: closed by default, opened by
/// [`present`](RepairReportDialog::present) for either a repaired or an
/// already-clean mesh, and persistent until the operator closes it (a fresh
/// repair replaces the card, About-window style).
#[derive(Default)]
pub(crate) struct RepairReportDialog {
    card: Option<Card>,
}

const REPAIR_MODAL_ID: &str = "repair_report_card";
const REPAIR_MODAL_DEFAULT_SIZE: egui::Vec2 = egui::vec2(440.0, 320.0);
const REPAIR_MODAL_CONTENT_WIDTH: f32 = 400.0;
const REPAIR_MODAL_BODY_ROW_HEIGHT: f32 = 22.0;
const REPAIR_MODAL_BODY_MIN_HEIGHT: f32 = 42.0;
const REPAIR_MODAL_BODY_MAX_HEIGHT: f32 = 230.0;

/// Use the same bounded, centered modal surface as About and the license
/// viewer. The old free-positioned Window could grow from its report rows and
/// drift toward the viewport edge, which made the repair result look broken.
fn show_repair_modal<T>(
    ctx: &egui::Context,
    add_contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    show_information_modal(
        ctx,
        egui::Id::new(REPAIR_MODAL_ID),
        REPAIR_MODAL_DEFAULT_SIZE,
        add_contents,
    )
}

fn repair_body_height(
    changed: bool,
    report_line_count: usize,
    has_open_rim: bool,
    has_skipped_rim: bool,
) -> f32 {
    let main_rows = if changed { report_line_count } else { 1 };
    let info_rows = usize::from(has_open_rim) + usize::from(has_skipped_rim);
    let info_gap = if info_rows > 0 { 6.0 } else { 0.0 };
    let row_height =
        (0..main_rows + info_rows).fold(0.0, |height, _| height + REPAIR_MODAL_BODY_ROW_HEIGHT);
    (row_height + info_gap).clamp(REPAIR_MODAL_BODY_MIN_HEIGHT, REPAIR_MODAL_BODY_MAX_HEIGHT)
}

impl RepairReportDialog {
    /// Show the card for `report` on `layer_label`. Works for both a real
    /// repair and a clean no-op; the body wording is derived from
    /// [`RepairReport::changed_content`].
    pub(crate) fn present(&mut self, layer_label: &str, report: RepairReport) {
        self.card = Some(Card {
            layer_label: layer_label.to_owned(),
            report,
        });
    }

    /// Dismiss the card.
    pub(crate) fn close(&mut self) {
        self.card = None;
    }

    /// Whether a card is on screen. Test probe: production code draws the card
    /// unconditionally in `ui()` and never branches on its open state.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_open(&self) -> bool {
        self.card.is_some()
    }

    /// Test probe: `None` when closed, else `Some(true)` if the open card is
    /// the clean-confirmation variant, `Some(false)` if it lists repairs.
    #[cfg(test)]
    pub(crate) fn showing_clean(&self) -> Option<bool> {
        self.card
            .as_ref()
            .map(|card| !card.report.changed_content())
    }

    /// Draw the card if open. Closing (X or the Close button) clears it.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn ui(&mut self, ctx: &egui::Context) {
        let Some(card) = self.card.as_ref() else {
            return;
        };

        // Pull everything the window body needs into owned locals so the
        // borrow of `self.card` ends before we may clear it below.
        let changed = card.report.changed_content();
        let layer_label = card.layer_label.clone();
        let lines = if changed {
            report_lines(&card.report)
        } else {
            Vec::new()
        };
        let open_rims = open_rims_line(&card.report);
        let skipped = skipped_rims_line(&card.report);
        let details = copy_details(&card.layer_label, &card.report);
        let body_height =
            repair_body_height(changed, lines.len(), open_rims.is_some(), skipped.is_some());

        let mut close_clicked = false;
        let mut copy_clicked = false;

        let modal_response = show_repair_modal(ctx, |ui| {
            ui.set_width(REPAIR_MODAL_CONTENT_WIDTH.min(ui.available_width()));

            ui.horizontal(|ui| {
                gutter_icon(ui, LineIcon::Fixed);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Mesh Repair")
                            .size(14.0)
                            .strong()
                            .color(ui_theme::text()),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&layer_label)
                                .size(11.0)
                                .color(ui_theme::text_weak()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&layer_label);
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Give the report body a content-sized viewport before adding its
            // rows. A bare ScrollArea can inherit the modal's unconstrained
            // height during egui's sizing pass and expand the card to the
            // whole window; this capped slot keeps long reports scrollable
            // without reserving a large blank area for short reports.
            let body_width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(body_width, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("repair-report-body")
                        .max_height(body_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if changed {
                                for line in &lines {
                                    line_row(ui, line.icon, &line.text);
                                }
                            } else {
                                ui.horizontal(|ui| {
                                    gutter_icon(ui, LineIcon::Fixed);
                                    ui.label(
                                        egui::RichText::new(CLEAN_HEADLINE).color(ui_theme::text()),
                                    );
                                });
                            }

                            if open_rims.is_some() || skipped.is_some() {
                                ui.add_space(6.0);
                                for info in
                                    [open_rims.as_ref(), skipped.as_ref()].into_iter().flatten()
                                {
                                    ui.label(
                                        egui::RichText::new(info)
                                            .size(11.0)
                                            .color(ui_theme::text_weak()),
                                    );
                                }
                            }
                        });
                },
            );

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close_clicked = true;
                }
                if ui
                    .button("Copy details")
                    .on_hover_text("Copy the full per-pass report to the clipboard")
                    .clicked()
                {
                    copy_clicked = true;
                }
            });
        });

        if copy_clicked {
            ctx.copy_text(details);
        }
        if close_clicked || modal_response.should_close() {
            self.close();
        }
    }
}

/// One report row: gutter glyph followed by the sentence, in body ink.
fn line_row(ui: &mut egui::Ui, icon: LineIcon, text: &str) {
    ui.horizontal(|ui| {
        gutter_icon(ui, icon);
        ui.label(egui::RichText::new(text).color(ui_theme::text()));
    });
}

/// Allocate a fixed 16 px gutter cell and paint `icon` into it.
fn gutter_icon(ui: &mut egui::Ui, icon: LineIcon) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    match icon {
        LineIcon::Removed => {
            crate::icons::paint(painter, rect, AppIcon::Delete, ui_theme::text_weak());
        }
        LineIcon::Closed => {
            crate::icons::paint(painter, rect, AppIcon::CloseHoles, ui_theme::text_weak());
        }
        LineIcon::Fixed => crate::icons::paint(painter, rect, AppIcon::Check, ui_theme::accent()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::MeshEditWarning;

    fn multi_report() -> RepairReport {
        RepairReport {
            input_vertices: 10_000,
            input_triangles: 20_000,
            output_vertices: 8_760,
            output_triangles: 19_900,
            welded_vertices: 1_240,
            removed_degenerate_triangles: 86,
            removed_duplicate_triangles: 12,
            split_nonmanifold_edges: 3,
            filled_holes: 12,
            removed_debris_components: 4,
            removed_debris_triangles: 40,
            open_rims_left: 2,
            ..RepairReport::default()
        }
    }

    #[test]
    fn group_thousands_inserts_spaces_only_past_a_thousand() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(7), "7");
        assert_eq!(group_thousands(86), "86");
        assert_eq!(group_thousands(1_240), "1 240");
        assert_eq!(group_thousands(1_000_000), "1 000 000");
    }

    #[test]
    fn report_lines_suppress_zeros_and_group_digits() {
        let lines = report_lines(&multi_report());
        let text: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            text,
            vec![
                "Welded 1 240 duplicate vertices",
                "Removed 86 sliver faces",
                "Removed 12 duplicate faces",
                "Fixed 3 non-manifold edges",
                "Removed 4 debris parts",
                "Closed 12 pinholes",
            ]
        );
        // Icons carry the right visual language: removals trash, holes close.
        assert_eq!(lines[0].icon, LineIcon::Fixed);
        assert_eq!(lines[1].icon, LineIcon::Removed);
        assert_eq!(lines[5].icon, LineIcon::Closed);
    }

    #[test]
    fn report_lines_use_singular_nouns_for_a_count_of_one() {
        let report = RepairReport {
            welded_vertices: 1,
            split_nonmanifold_edges: 1,
            filled_holes: 1,
            removed_unreferenced_vertices: 1,
            ..RepairReport::default()
        };
        let lines = report_lines(&report);
        let text: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            text,
            vec![
                "Welded 1 duplicate vertex",
                "Fixed 1 non-manifold edge",
                "Closed 1 pinhole",
                "Removed 1 unused vertex",
            ]
        );
    }

    #[test]
    fn a_clean_report_produces_no_lines() {
        assert!(report_lines(&RepairReport::default()).is_empty());
    }

    #[test]
    fn open_and_skipped_rim_lines_handle_plurality_and_absence() {
        assert_eq!(open_rims_line(&RepairReport::default()), None);
        let one = RepairReport {
            open_rims_left: 1,
            ..RepairReport::default()
        };
        assert_eq!(
            open_rims_line(&one).as_deref(),
            Some("1 open rim left (scan boundary)")
        );
        let many = RepairReport {
            open_rims_left: 2,
            ..RepairReport::default()
        };
        assert_eq!(
            open_rims_line(&many).as_deref(),
            Some("2 open rims left (scan boundary)")
        );

        assert_eq!(skipped_rims_line(&RepairReport::default()), None);
        let warned = RepairReport {
            warnings: vec![MeshEditWarning::DegenerateGeometry],
            ..RepairReport::default()
        };
        assert_eq!(
            skipped_rims_line(&warned).as_deref(),
            Some("1 rim could not be filled (non-simple)")
        );
    }

    #[test]
    fn repair_body_height_is_compact_for_short_reports_and_capped_for_long_ones() {
        let clean_height = repair_body_height(false, 0, false, false);
        let short_height = repair_body_height(true, 2, false, false);
        let long_height = repair_body_height(true, 100, true, true);

        assert!((clean_height - REPAIR_MODAL_BODY_MIN_HEIGHT).abs() < f32::EPSILON);
        assert!(short_height < REPAIR_MODAL_BODY_MAX_HEIGHT);
        assert!((long_height - REPAIR_MODAL_BODY_MAX_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn copy_details_dumps_before_after_and_every_pass_including_zeros() {
        let details = copy_details("scan.stl", &multi_report());
        assert!(details.contains("Repair report — scan.stl"));
        assert!(details.contains("Before: 10 000 vertices, 20 000 triangles"));
        assert!(details.contains("After:  8 760 vertices, 19 900 triangles"));
        // A pass that did nothing still appears with a zero (full dump).
        assert!(details.contains("Reoriented triangles"));
        assert!(details.contains("Split bowtie vertices"));
        assert!(details.lines().any(|line| line.trim_end().ends_with(" 0")));
        // Debris triangles ride along with their parts and are shown here.
        assert!(details.contains("Removed debris faces"));
    }

    #[test]
    fn dialog_opens_on_repair_opens_clean_and_closes() {
        let ctx = egui::Context::default();
        let mut dialog = RepairReportDialog::default();
        assert!(!dialog.is_open());
        assert_eq!(dialog.showing_clean(), None);

        // Repaired: lists fixes, stays open across a frame (persistent).
        dialog.present("scan.stl", multi_report());
        assert!(dialog.is_open());
        assert_eq!(dialog.showing_clean(), Some(false));
        ctx.run_ui(egui::RawInput::default(), |ui| dialog.ui(ui.ctx()))
            .drop_without_applying_deltas();
        assert!(dialog.is_open());

        // Clean: positive confirmation, still an open card.
        dialog.present("scan.stl", RepairReport::default());
        assert_eq!(dialog.showing_clean(), Some(true));
        ctx.run_ui(egui::RawInput::default(), |ui| dialog.ui(ui.ctx()))
            .drop_without_applying_deltas();
        assert!(dialog.is_open());

        // Close clears it; drawing while closed is a no-op.
        dialog.close();
        assert!(!dialog.is_open());
        ctx.run_ui(egui::RawInput::default(), |ui| dialog.ui(ui.ctx()))
            .drop_without_applying_deltas();
        assert!(!dialog.is_open());
    }

    #[test]
    fn repair_report_is_centered_and_bounded_like_the_other_information_modals() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut dialog = RepairReportDialog::default();
        dialog.present("very-long-dental-scan-layer-name.stl", multi_report());

        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| dialog.ui(ui.ctx()),
        )
        .drop_without_applying_deltas();
        // egui deliberately uses the first frame as a sizing pass for areas;
        // the visible frame must be checked after that pass has positioned the
        // measured card around the center anchor.
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| dialog.ui(ui.ctx()),
        )
        .drop_without_applying_deltas();
        let rect = ctx.memory(|memory| memory.area_rect(egui::Id::new(REPAIR_MODAL_ID)));
        assert!(rect.is_some(), "Repair Mesh report should render an area");
        let Some(rect) = rect else {
            return;
        };
        assert!(
            screen.contains_rect(rect),
            "report modal {rect:?} escaped the screen {screen:?}"
        );
        assert!(
            (rect.center().x - screen.center().x).abs() < 1.0
                && (rect.center().y - screen.center().y).abs() < 1.0,
            "report modal {rect:?} should be centered in {screen:?}"
        );
        assert!(
            rect.width() <= 480.0,
            "report modal spread too wide: {}",
            rect.width()
        );
        assert!(
            rect.height() <= 440.0,
            "report modal spread too tall: {}",
            rect.height()
        );
    }
}

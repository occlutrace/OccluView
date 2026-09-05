//! The Brush tool window: the "Exclude selected parts" convention dental CAD
//! software uses, control for control.
//!
//! A **separate movable window**, opened by the "Matching: Exclude selected
//! parts" checkbox on the automatic tab, exactly as the operator's dental CAD
//! software opens it. It is not a section of the main window and not a mode
//! of the manual tab: the operator is painting on the mesh with one hand
//! while reading the alignment controls with the other, so the two have to
//! be positionable independently.

use eframe::egui;

use crate::align_brush::AlignBrush;
use crate::align_markings::MaskCommand;
use crate::align_panel::chip;
use crate::icons::AppIcon;
use crate::ui_theme;

/// Fixed window width. Narrower than the main window: it holds one slider's
/// worth of content and floats over the mesh being painted.
const WINDOW_WIDTH: f32 = 236.0;

/// Ink for the line that says the whole mesh has been marked out.
///
/// Derived from the colour the marked surface itself is painted, not copied from
/// it: the two were separate literals in separate files, each with a comment
/// claiming they agreed.
const MARKED_OUT_INK: egui::Color32 = {
    let ink = crate::align_markings::MARKED_OUT_COLOR;
    egui::Color32::from_rgb(ink[0], ink[1], ink[2])
};

/// What the Brush tool window asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrushPanelAction {
    /// Run a whole-mesh command.
    Mask(MaskCommand),
    /// Close the window — the same as clearing the checkbox that opened it.
    Close,
}

/// Show the Brush tool window; returns what the operator asked for.
// Six inherently (ui/ctx + data + locale); bundling would fake an abstraction.
#[expect(clippy::too_many_arguments)]
pub(crate) fn show(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    brush: &mut AlignBrush,
    marked: Option<f32>,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<BrushPanelAction> {
    // Opens to the LEFT of the main window's default corner, so the two do not
    // land on top of each other the first time the checkbox is ticked.
    let default_pos = viewport_rect.right_top() + egui::vec2(-WINDOW_WIDTH - 300.0, 16.0);
    let mut action = None;
    egui::Window::new("Brush tool")
        .id(egui::Id::new("occluview_align_brush_window"))
        .default_pos(default_pos)
        .constrain_to(viewport_rect)
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(WINDOW_WIDTH - 24.0);
            ui.set_width(WINDOW_WIDTH - 24.0);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            ui.style_mut().animation_time = 0.05;
            action = body(ui, brush, marked, enabled, locale);
        });
    action
}

/// The window body.
fn body(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    marked: Option<f32>,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<BrushPanelAction> {
    let mut action = header(ui, locale);
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(locale.tr("align-brush-subtitle"))
            .size(11.0)
            .color(ui_theme::text()),
    );
    ui.label(
        egui::RichText::new(if brush.is_inverse() {
            locale.tr("align-brush-hint-inverse")
        } else {
            locale.tr("align-brush-hint-mark")
        })
        .size(11.0)
        .color(ui_theme::text_muted()),
    );
    ui.add_space(4.0);

    action = action.or(commands(ui, enabled, locale));
    ui.add_space(2.0);
    size(ui, brush, enabled, locale);
    automatic(ui, brush, enabled, locale);
    coverage(ui, marked, locale);
    action
}

/// The title strip, with the only way out of the window that is not the
/// checkbox that opened it.
fn header(ui: &mut egui::Ui, locale: &crate::i18n::LocaleManager) -> Option<BrushPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover())
            .0;
        crate::icons::paint(ui.painter(), glyph, AppIcon::MaskBrush, ui_theme::accent());
        ui.label(
            egui::RichText::new(locale.tr("align-brush-title"))
                .size(13.0)
                .color(ui_theme::text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (close_rect, close_response) =
                ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::click());
            crate::icons::paint(
                ui.painter(),
                close_rect,
                AppIcon::Close,
                if close_response.hovered() {
                    ui_theme::text()
                } else {
                    ui_theme::text_weak()
                },
            );
            if close_response
                .on_hover_text(locale.tr("align-brush-close-hint"))
                .clicked()
            {
                action = Some(BrushPanelAction::Close);
            }
        });
    });
    action
}

/// The same whole-mesh commands the operator's dental CAD software offers,
/// driven off the command list itself so a new one cannot be added to the
/// enum and forgotten here.
fn commands(
    ui: &mut egui::Ui,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<BrushPanelAction> {
    let mut action = None;
    for command in MaskCommand::ALL {
        // A match rather than a lookup table: a new command stops the build here
        // instead of quietly rendering without a picture.
        let icon = match command {
            MaskCommand::FitEverywhere => AppIcon::SelectNone,
            MaskCommand::FitNowhere => AppIcon::SelectAll,
            MaskCommand::InvertMarkings => AppIcon::SelectInvert,
            MaskCommand::MarkAutomatic => AppIcon::AlignFit,
        };
        if chip(
            ui,
            ui.available_width(),
            Some(icon),
            &locale.tr(command.label_key()),
            enabled,
            false,
        )
        .on_hover_text(locale.tr(command.hint_key()))
        .clicked()
        {
            action = Some(BrushPanelAction::Mask(command));
        }
    }
    action
}

/// Brush size and the standing stroke direction.
fn size(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    let mut radius = brush.radius_mm();
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut radius, 0.1..=20.0)
                .suffix(" mm")
                .text(locale.tr("align-brush-size").as_str()),
        )
        .changed()
    {
        brush.set_radius_mm(radius);
    }
    let mut inverse = brush.is_inverse();
    if ui
        .add_enabled(
            enabled,
            egui::Checkbox::new(&mut inverse, locale.tr("align-brush-inverse").as_str()),
        )
        .on_hover_text(locale.tr("align-brush-inverse-hint"))
        .changed()
    {
        brush.set_inverse(inverse);
    }
}

/// The same "Mark automatic" control and radius the operator's dental CAD
/// software uses.
fn automatic(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    ui.add_space(2.0);
    let mut radius = brush.auto_radius_mm();
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut radius, 0.1..=20.0)
                .suffix(" mm")
                .text(locale.tr("align-brush-auto-radius").as_str()),
        )
        .on_hover_text(locale.tr("align-brush-auto-radius-hint"))
        .changed()
    {
        brush.set_auto_radius_mm(radius);
    }
}

/// How much of the two meshes is currently marked.
///
/// The one number that says whether the brush did what the operator meant.
/// "Fit nowhere" and a slip of the hand look identical on a shaded surface at a
/// glance, and both make best-fit matching do nothing.
fn coverage(ui: &mut egui::Ui, marked: Option<f32>, locale: &crate::i18n::LocaleManager) {
    let Some(marked) = marked else {
        return;
    };
    let percent = (marked * 100.0).clamp(0.0, 100.0);
    let (text, ink) = if marked >= 1.0 {
        (locale.tr("align-brush-all-marked"), MARKED_OUT_INK)
    } else if marked <= 0.0 {
        (
            locale.tr("align-brush-nothing-marked"),
            ui_theme::text_muted(),
        )
    } else {
        (
            locale.tr_with(
                "align-brush-percent-marked",
                &[("pct", &format!("{percent:.0}"))],
            ),
            ui_theme::text_muted(),
        )
    };
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(10.5).color(ink));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use crate::align_markings::MaskCommand;

    fn production() -> &'static str {
        let source =
            crate::primary_ui_tests::production_source(include_str!("align_panel_brush.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// The operator's dental CAD software opens this as its own window, and
    /// the reason is practical: the operator paints on the mesh with one hand
    /// and reads the alignment controls with the other, so the two have to
    /// move independently.
    #[test]
    fn the_brush_is_its_own_movable_window() {
        let source = production();
        assert!(source.contains("egui::Window::new(\"Brush tool\")"));
        assert!(source.contains(".constrain_to(viewport_rect)"));
        assert!(
            !source.contains(".anchor("),
            "an anchored window cannot be moved off the mesh being painted"
        );
    }

    /// Every command the operator's dental CAD brush window offers has to be
    /// here, or an operator who reaches for one finds a gap.
    #[test]
    fn every_whole_mesh_command_is_offered() {
        let source = production();
        for command in [
            MaskCommand::FitEverywhere,
            MaskCommand::FitNowhere,
            MaskCommand::InvertMarkings,
            MaskCommand::MarkAutomatic,
        ] {
            let name = format!("MaskCommand::{command:?}");
            assert!(source.contains(&name), "{name} is never offered");
        }
        // Control captions resolve through the catalog; the keys are
        // what the window must reference.
        for control in [
            "align-brush-size",
            "align-brush-inverse",
            "align-brush-auto-radius",
        ] {
            assert!(source.contains(control), "the brush needs {control}");
        }
    }

    /// "Fit nowhere" and a slip of the hand look identical on a shaded surface,
    /// and both make best-fit matching silently do nothing.
    #[test]
    fn the_window_says_how_much_of_the_mesh_is_marked() {
        let source = production();
        assert!(source.contains("align-brush-all-marked"));
        assert!(source.contains("align-brush-percent-marked"));
    }
}

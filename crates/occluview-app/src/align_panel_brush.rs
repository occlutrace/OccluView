//! The Brush tool window: exocad's "Exclude selected parts", control for
//! control.
//!
//! A **separate movable window**, opened by the "Matching: Exclude selected
//! parts" checkbox on the automatic tab, exactly as exocad opens it. It is not
//! a section of the main window and not a mode of the manual tab: the operator
//! is painting on the mesh with one hand while reading the alignment controls
//! with the other, so the two have to be positionable independently.

use eframe::egui;

use crate::align_brush::AlignBrush;
use crate::align_markings::MaskCommand;
use crate::align_panel::chip;
use crate::mesh_editor_icons::{self, EditorIcon};
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
pub(crate) fn show(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    brush: &mut AlignBrush,
    marked: Option<f32>,
    enabled: bool,
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
            action = body(ui, brush, marked, enabled);
        });
    action
}

/// The window body.
fn body(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    marked: Option<f32>,
    enabled: bool,
) -> Option<BrushPanelAction> {
    let mut action = header(ui);
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Paint the surface best-fit matching must ignore, on either mesh")
            .size(11.0)
            .color(ui_theme::TEXT),
    );
    ui.label(
        egui::RichText::new(if brush.is_inverse() {
            "Drag clears · hold Shift to mark · Shift+wheel resizes"
        } else {
            "Drag marks · hold Shift to clear · Shift+wheel resizes"
        })
        .size(11.0)
        .color(ui_theme::TEXT_MUTED),
    );
    ui.add_space(4.0);

    action = action.or(commands(ui, enabled));
    ui.add_space(2.0);
    size(ui, brush, enabled);
    automatic(ui, brush, enabled);
    coverage(ui, marked);
    action
}

/// The title strip, with the only way out of the window that is not the
/// checkbox that opened it.
fn header(ui: &mut egui::Ui) -> Option<BrushPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover())
            .0;
        mesh_editor_icons::paint(
            ui.painter(),
            glyph,
            EditorIcon::MaskBrush,
            ui_theme::ACCENT,
            true,
        );
        ui.label(
            egui::RichText::new("Brush tool")
                .size(12.0)
                .color(ui_theme::TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("✕").min_size(egui::vec2(22.0, 20.0)))
                .on_hover_text("Close the brush — the markings are kept")
                .clicked()
            {
                action = Some(BrushPanelAction::Close);
            }
        });
    });
    action
}

/// exocad's whole-mesh commands, driven off the command list itself so a new
/// one cannot be added to the enum and forgotten here.
fn commands(ui: &mut egui::Ui, enabled: bool) -> Option<BrushPanelAction> {
    let mut action = None;
    for command in MaskCommand::ALL {
        // A match rather than a lookup table: a new command stops the build here
        // instead of quietly rendering without a picture.
        let icon = match command {
            MaskCommand::FitEverywhere => EditorIcon::SelectNone,
            MaskCommand::FitNowhere => EditorIcon::SelectAll,
            MaskCommand::InvertMarkings => EditorIcon::SelectInvert,
            MaskCommand::MarkAutomatic => EditorIcon::AlignFit,
        };
        if chip(
            ui,
            ui.available_width(),
            Some(icon),
            command.label(),
            enabled,
            false,
        )
        .on_hover_text(command.hint())
        .clicked()
        {
            action = Some(BrushPanelAction::Mask(command));
        }
    }
    action
}

/// Brush size and the standing stroke direction.
fn size(ui: &mut egui::Ui, brush: &mut AlignBrush, enabled: bool) {
    let mut radius = brush.radius_mm();
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut radius, 0.1..=20.0)
                .suffix(" mm")
                .text("brush size"),
        )
        .changed()
    {
        brush.set_radius_mm(radius);
    }
    let mut inverse = brush.is_inverse();
    if ui
        .add_enabled(enabled, egui::Checkbox::new(&mut inverse, "Brush inverse"))
        .on_hover_text("A plain drag clears instead of marks. Shift inverses it again")
        .changed()
    {
        brush.set_inverse(inverse);
    }
}

/// exocad's "Mark automatic" and the radius it uses.
fn automatic(ui: &mut egui::Ui, brush: &mut AlignBrush, enabled: bool) {
    ui.add_space(2.0);
    let mut radius = brush.auto_radius_mm();
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut radius, 0.1..=20.0)
                .suffix(" mm")
                .text("automatic radius"),
        )
        .on_hover_text("The radius of the mesh area kept at each arrow end")
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
fn coverage(ui: &mut egui::Ui, marked: Option<f32>) {
    let Some(marked) = marked else {
        return;
    };
    let percent = (marked * 100.0).clamp(0.0, 100.0);
    let (text, ink) = if marked >= 1.0 {
        (
            "Everything is marked — best-fit matching will have no effect".to_owned(),
            MARKED_OUT_INK,
        )
    } else if marked <= 0.0 {
        ("Nothing marked".to_owned(), ui_theme::TEXT_MUTED)
    } else {
        (
            format!("{percent:.0}% marked out of the match"),
            ui_theme::TEXT_MUTED,
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
        let source = include_str!("align_panel_brush.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// exocad opens this as its own window, and the reason is practical: the
    /// operator paints on the mesh with one hand and reads the alignment
    /// controls with the other, so the two have to move independently.
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

    /// Every command exocad's brush window offers has to be here, or an
    /// operator who reaches for one finds a gap.
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
        for control in ["brush size", "Brush inverse", "automatic radius"] {
            assert!(source.contains(control), "the brush needs {control}");
        }
    }

    /// "Fit nowhere" and a slip of the hand look identical on a shaded surface,
    /// and both make best-fit matching silently do nothing.
    #[test]
    fn the_window_says_how_much_of_the_mesh_is_marked() {
        let source = production();
        assert!(source.contains("best-fit matching will have no effect"));
        assert!(source.contains("marked out of the match"));
    }
}

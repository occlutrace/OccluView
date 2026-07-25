//! The Align Scans window.
//!
//! A movable window built from the same pieces as the mesh editor: no title
//! bar, icon buttons, and a commit row that ends in Cancel and Done. Nothing
//! here names a target, a source, or a role — the pair comes from the points
//! the operator clicks in the viewport, so this window's job is to report what
//! was clicked, offer the two fits, and read the map honestly.

use eframe::egui;
use occluview_align::{DeviationStats, Orientation, RampMode};

use crate::align_brush::{AlignBrush, MaskCommand};
use crate::align_drag::DragConstraint;
use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::mesh_editor_icons::{self, EditorIcon};
use crate::{align_overlay, ui_theme};

/// Fixed window width, matching the mesh editor so the two read as one family.
const WINDOW_WIDTH: f32 = 268.0;
/// Height of the two big fit buttons.
const FIT_BUTTON_HEIGHT: f32 = 34.0;
/// Width of a small icon button.
const ICON_BUTTON: f32 = 40.0;

/// What the operator asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignPanelAction {
    /// Fit the clicked pairs.
    Align,
    /// Seat the surfaces against each other.
    Refine,
    /// Remove the last point or pair.
    Back,
    /// Drop the pair and start over.
    Clear,
    /// Re-measure with the current settings.
    Measure,
    /// Stop showing the map.
    HideMap,
    /// Paint the map on the other surface instead.
    SwapMapped,
    /// Run a whole-mask command.
    Mask(MaskCommand),
    /// Put every scan back where it was and close.
    Cancel,
    /// Keep the alignment and close.
    Done,
}

/// Everything the window needs to draw itself.
pub(crate) struct AlignPanelView<'a> {
    /// The click model.
    pub(crate) tool: &'a AlignTool,
    /// Live settings, edited in place.
    pub(crate) settings: &'a mut AlignSettings,
    /// Which directions a hand drag may move in, edited in place.
    pub(crate) constraint: &'a mut DragConstraint,
    /// The exclusion brush, edited in place.
    pub(crate) brush: &'a mut AlignBrush,
    /// The last thing that happened, in a sentence.
    pub(crate) status: Option<&'a str>,
    /// The measurement summary, when there is one.
    pub(crate) stats: Option<DeviationStats>,
    /// Whether a job is in flight.
    pub(crate) busy: bool,
    /// Whether anything has actually moved this session.
    pub(crate) moved: bool,
}

/// Show the movable window; returns what the operator asked for.
pub(crate) fn show(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    view: AlignPanelView<'_>,
) -> Option<AlignPanelAction> {
    let default_pos = viewport_rect.right_top() + egui::vec2(-WINDOW_WIDTH - 16.0, 16.0);
    let mut action = None;
    egui::Window::new("Align Scans")
        .id(egui::Id::new("occluview_align_window"))
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
            action = body(ui, view);
        });
    action
}

/// The window body, top to bottom in the order the work happens.
fn body(ui: &mut egui::Ui, view: AlignPanelView<'_>) -> Option<AlignPanelAction> {
    let enabled = !view.busy;
    header(ui, view.tool);
    let mut action = fits(ui, view.tool, enabled);
    action = action.or(points(ui, enabled));
    ui.separator();
    action = action.or(heatmap(ui, view.settings, view.stats, enabled));
    ui.separator();
    action = action.or(handling(ui, view.constraint, view.brush, enabled));
    status(ui, view.status);
    action.or(commit(ui, enabled, view.moved))
}

/// What the tool is waiting for, in one line.
fn header(ui: &mut egui::Ui, tool: &AlignTool) {
    let placed = tool.pairs().len();
    let prompt = if tool.moving_layer().is_none() {
        "Click a point on the scan that should move".to_owned()
    } else if tool.pending().is_some() {
        "Click the matching spot on the other scan".to_owned()
    } else if placed == 0 {
        "Click a point on each scan to pair them".to_owned()
    } else {
        format!("{placed} pair{} placed", if placed == 1 { "" } else { "s" })
    };
    ui.label(egui::RichText::new(prompt).size(12.0).color(ui_theme::TEXT));
    ui.add_space(2.0);
}

/// The two fits. Refine is the primary action and is sized like one: the point
/// fit only gets the scan close, the surface fit is what actually seats it.
fn fits(ui: &mut egui::Ui, tool: &AlignTool, enabled: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    let width = ui.available_width();
    if fit_button(
        ui,
        width,
        EditorIcon::AlignFit,
        "Align on points",
        tool.can_align() && enabled,
        false,
    )
    .on_hover_text("Bring the scan close using the clicked pairs")
    .clicked()
    {
        action = Some(AlignPanelAction::Align);
    }
    if fit_button(
        ui,
        width,
        EditorIcon::AlignRefine,
        "Refine on surfaces",
        tool.can_measure() && enabled,
        true,
    )
    .on_hover_text("Seat the surfaces against each other and measure the result")
    .clicked()
    {
        action = Some(AlignPanelAction::Refine);
    }
    action
}

/// Taking a click back.
fn points(ui: &mut egui::Ui, enabled: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if small_button(ui, width, "Back", enabled)
            .on_hover_text("Remove the last point")
            .clicked()
        {
            action = Some(AlignPanelAction::Back);
        }
        if small_button(ui, width, "Clear", enabled)
            .on_hover_text("Drop the pair and start over")
            .clicked()
        {
            action = Some(AlignPanelAction::Clear);
        }
    });
    action
}

/// The heatmap: the colour bar, the one slider that scales it, and the numbers.
///
/// Compact on purpose. Lab software puts a false-colour bar and a single
/// scaling slider in front of the operator; everything else is a detail that
/// only matters when something looks wrong, and lives behind the fold.
fn heatmap(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    stats: Option<DeviationStats>,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let glyph = ui
            .allocate_exact_size(egui::vec2(17.0, 17.0), egui::Sense::hover())
            .0;
        mesh_editor_icons::paint(
            ui.painter(),
            glyph,
            EditorIcon::Heatmap,
            if settings.show_deviation {
                ui_theme::ACCENT
            } else {
                ui_theme::TEXT_MUTED
            },
            settings.show_deviation,
        );
        let mut shown = settings.show_deviation;
        if ui
            .checkbox(&mut shown, "Heatmap")
            .on_hover_text("Colour the aligned scan by how far it sits from the other")
            .changed()
        {
            settings.show_deviation = shown;
            action = Some(if shown {
                AlignPanelAction::Measure
            } else {
                AlignPanelAction::HideMap
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if mesh_editor_icons::icon_button(
                ui,
                egui::vec2(ICON_BUTTON, 24.0),
                EditorIcon::Swap,
                "",
                "Paint the map on the other scan instead",
                settings.show_deviation && enabled,
                false,
            )
            .clicked()
            {
                action = Some(AlignPanelAction::SwapMapped);
            }
        });
    });
    if !settings.show_deviation {
        return action;
    }

    align_overlay::paint_legend(ui, *settings);
    // One slider, directly under the bar it scales — the arrangement every
    // metrology tool uses, because the bar is the legend for the slider.
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut settings.scale_mm, 0.05..=2.0)
                .suffix(" mm")
                .text("scale"),
        )
        .drag_stopped()
    {
        action = Some(AlignPanelAction::Measure);
    }

    if let Some(stats) = stats {
        numbers(ui, stats, settings.tolerance_mm);
    }

    ui.collapsing("Heatmap details", |ui| {
        if details(ui, settings) {
            action = Some(AlignPanelAction::Measure);
        }
    });
    action
}

/// The knobs that only matter when something looks wrong.
fn details(ui: &mut egui::Ui, settings: &mut AlignSettings) -> bool {
    let mut changed = false;
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.tolerance_mm, 0.01..=1.0)
                .suffix(" mm")
                .text("tolerance"),
        )
        .drag_stopped();
    changed |= ui
        .add(
            egui::Slider::new(&mut settings.influence_radius_mm, 0.2..=10.0)
                .suffix(" mm")
                .text("max distance"),
        )
        .drag_stopped();

    let mut banded = settings.bands.is_some();
    if ui
        .checkbox(&mut banded, "Stepped bands")
        .on_hover_text("Step the ramp instead of blending it")
        .changed()
    {
        settings.bands = banded.then_some(10);
        changed = true;
    }
    changed |= colours(ui, &mut settings.ramp_mode);
    changed |= facing(ui, &mut settings.orientation);
    changed
}

/// Which colour scheme the map paints with.
fn colours(ui: &mut egui::Ui, mode: &mut RampMode) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Colours")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        for (value, label, hint) in [
            (
                RampMode::Magnitude,
                "distance",
                "Cool where the scans agree, hot where they do not",
            ),
            (
                RampMode::Signed,
                "signed",
                "Blue below the surface, green nominal, red above",
            ),
        ] {
            if ui
                .selectable_label(*mode == value, label)
                .on_hover_text(hint)
                .clicked()
                && *mode != value
            {
                *mode = value;
                changed = true;
            }
        }
    });
    changed
}

/// The surface-orientation rule. An inverted mesh flips the whole signed map,
/// so this is the escape hatch for a scan whose winding disagrees with itself.
fn facing(ui: &mut egui::Ui, orientation: &mut Orientation) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Facing")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        for (value, label, hint) in [
            (Orientation::Match, "as is", "Surfaces face the same way"),
            (
                Orientation::Inverted,
                "flipped",
                "The other scan's winding is inverted",
            ),
            (Orientation::Ignored, "either", "Accept either facing"),
        ] {
            if ui
                .selectable_label(*orientation == value, label)
                .on_hover_text(hint)
                .clicked()
                && *orientation != value
            {
                *orientation = value;
                changed = true;
            }
        }
    });
    changed
}

/// The numbers behind the colours, including what could not be measured.
fn numbers(ui: &mut egui::Ui, stats: DeviationStats, tolerance_mm: f64) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{:.0}% within {tolerance_mm:.2} mm",
                stats.within_tolerance * 100.0
            ))
            .size(11.0)
            .color(ui_theme::TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("rms {:.3}", stats.rms))
                    .size(11.0)
                    .color(ui_theme::TEXT_MUTED),
            );
        });
    });
    if stats.skipped > 0 {
        ui.label(
            egui::RichText::new(format!("{} vertices had nothing to measure", stats.skipped))
                .size(10.0)
                .color(ui_theme::TEXT_MUTED),
        );
    }
}

/// Hand movement and the exclusion brush.
fn handling(
    ui: &mut egui::Ui,
    constraint: &mut DragConstraint,
    brush: &mut AlignBrush,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Drag")
                .size(11.0)
                .color(ui_theme::TEXT_MUTED),
        );
        for value in [
            DragConstraint::Free,
            DragConstraint::ZOnly,
            DragConstraint::XyPlane,
        ] {
            if ui
                .selectable_label(*constraint == value, value.label())
                .clicked()
            {
                *constraint = value;
            }
        }
    })
    .response
    .on_hover_text("Drag a scan to move it, Ctrl+drag to turn it about its own centre");

    ui.horizontal(|ui| {
        let mut armed = brush.is_armed();
        if mesh_editor_icons::icon_button(
            ui,
            egui::vec2(ICON_BUTTON, 26.0),
            EditorIcon::MaskBrush,
            "",
            "Paint a region out of the comparison; hold Shift to erase",
            enabled,
            armed,
        )
        .clicked()
        {
            armed = !armed;
            brush.set_armed(armed);
        }
        if armed {
            let mut radius = brush.radius_mm();
            if ui
                .add(egui::Slider::new(&mut radius, 0.1..=20.0).suffix(" mm"))
                .changed()
            {
                brush.set_radius_mm(radius);
            }
        } else {
            ui.label(
                egui::RichText::new("Exclude by brush")
                    .size(11.0)
                    .color(ui_theme::TEXT_MUTED),
            );
        }
    });
    if brush.is_armed() {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - ui.spacing().item_spacing.x * 3.0) / 4.0;
            for (label, command, hint) in [
                ("None", MaskCommand::Nowhere, "Compare the whole scan"),
                ("All", MaskCommand::Everywhere, "Mask the whole scan"),
                ("Invert", MaskCommand::Invert, "Flip the mask"),
                (
                    "Points",
                    MaskCommand::AroundPoints,
                    "Take the clicked spots out of the surface fit",
                ),
            ] {
                if small_button(ui, width, label, enabled)
                    .on_hover_text(hint)
                    .clicked()
                {
                    action = Some(AlignPanelAction::Mask(command));
                }
            }
        });
    }
    action
}

/// The last thing that happened.
fn status(ui: &mut egui::Ui, status: Option<&str>) {
    let Some(status) = status else {
        return;
    };
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(status)
            .size(11.0)
            .color(ui_theme::TEXT_WEAK),
    );
}

/// Cancel and Done, the same commit pair the mesh editor ends on.
///
/// Cancel means what it says: every scan goes back where it was. Closing a tool
/// and silently keeping what it did is how an operator loses work they thought
/// they had discarded.
fn commit(ui: &mut egui::Ui, enabled: bool, moved: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if tall_button(ui, width, "Cancel", enabled, false)
            .on_hover_text(if moved {
                "Put every scan back where it was and close"
            } else {
                "Close without changing anything"
            })
            .clicked()
        {
            action = Some(AlignPanelAction::Cancel);
        }
        if tall_button(ui, width, "Done", enabled, true)
            .on_hover_text("Keep the alignment and close")
            .clicked()
        {
            action = Some(AlignPanelAction::Done);
        }
    });
    action
}

/// A full-width fit button: glyph, then label, at a size that says which of the
/// two is the one that matters.
// A button needs its width, glyph, label, and both state flags; bundling them
// into a struct would only add ceremony at each of the two call sites.
#[allow(clippy::too_many_arguments)]
fn fit_button(
    ui: &mut egui::Ui,
    width: f32,
    icon: EditorIcon,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, FIT_BUTTON_HEIGHT), sense);
    let ink = if enabled {
        if primary {
            ui_theme::ACCENT
        } else {
            ui_theme::TEXT
        }
    } else {
        ui.visuals().weak_text_color()
    };
    let painter = ui.painter();
    if enabled && primary {
        painter.rect_filled(rect, 5.0, ui_theme::ACCENT.gamma_multiply(0.14));
    }
    if enabled && response.hovered() {
        painter.rect_filled(rect, 5.0, ui_theme::ACCENT.gamma_multiply(0.10));
    }
    painter.rect_stroke(
        rect,
        5.0,
        egui::Stroke::new(1.0, ink.gamma_multiply(if primary { 0.75 } else { 0.35 })),
    );
    let glyph = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 21.0, rect.center().y),
        egui::Vec2::splat(17.0),
    );
    mesh_editor_icons::paint(painter, glyph, icon, ink, primary);
    painter.text(
        egui::pos2(glyph.right() + 9.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(if primary { 13.0 } else { 12.0 }),
        ink,
    );
    response
}

/// A short text button.
fn small_button(ui: &mut egui::Ui, width: f32, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).size(11.5)).min_size(egui::vec2(width, 22.0)),
    )
}

/// A tall commit button.
fn tall_button(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    let text = egui::RichText::new(label).size(12.5).color(if primary {
        ui_theme::ACCENT
    } else {
        ui_theme::TEXT
    });
    ui.add_enabled(
        enabled,
        egui::Button::new(text).min_size(egui::vec2(width, 28.0)),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    fn production() -> &'static str {
        let source = include_str!("align_panel.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// The whole point of this tool is that there is no object picker. If a
    /// control ever names a target or a role, the simplification is gone.
    #[test]
    fn no_control_in_the_window_names_a_target_a_source_or_a_role() {
        for literal in production().split('"').skip(1).step_by(2) {
            let lowered = literal.to_lowercase();
            for banned in ["target", "source object", "primary object", "role"] {
                assert!(
                    !lowered.contains(banned),
                    "a control says {literal:?}, which names {banned}"
                );
            }
        }
    }

    /// The window has to be draggable like the mesh editor: a panel pinned to a
    /// corner covers the very geometry the operator is clicking on.
    #[test]
    fn the_window_is_movable_and_constrained_to_the_viewport() {
        let source = production();
        assert!(source.contains("egui::Window::new(\"Align Scans\")"));
        assert!(source.contains(".default_pos(default_pos)"));
        assert!(source.contains(".constrain_to(viewport_rect)"));
        assert!(
            !source.contains(".anchor("),
            "an anchored window cannot be moved out of the way"
        );
    }

    /// Closing a tool and silently keeping what it did is how an operator loses
    /// work they thought they had discarded.
    #[test]
    fn the_window_ends_in_cancel_and_done() {
        let commit = production()
            .split_once("fn commit(")
            .map(|(_, rest)| rest)
            .expect("a commit row");
        assert!(commit.contains("AlignPanelAction::Cancel"));
        assert!(commit.contains("AlignPanelAction::Done"));
    }

    /// The measurement is only honest if the window says how much of the scan
    /// it could not measure.
    #[test]
    fn the_numbers_report_unmeasured_vertices() {
        assert!(production().contains("had nothing to measure"));
    }
}

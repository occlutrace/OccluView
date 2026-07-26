//! The Align Scans window.
//!
//! A movable window built from the same pieces as the mesh editor: no title
//! bar, icon buttons, and a commit row that ends in Cancel and Done. Nothing
//! here names a target, a source, or a role — the pair comes from the points
//! the operator clicks in the viewport.
//!
//! The window is three stacked blocks, in the order the work happens:
//!
//! 1. **How** the scan is placed — the two tabs. *Automatically* collects
//!    point pairs; *Manually* moves the scan by hand and paints the region the
//!    match runs on.
//! 2. **The fits.** Shared by both tabs on purpose: an operator who nudged a
//!    scan by hand still wants to seat it, and a Best fit button that vanished
//!    when they switched tabs is a button they cannot find.
//! 3. **The Hitmap**, and then Cancel / Done.

use eframe::egui;
use occluview_align::DeviationStats;

use crate::align_brush::{AlignBrush, BrushPaint, MaskCommand};
use crate::align_drag::DragConstraint;
use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::mesh_editor_icons::{self, EditorIcon};
use crate::{align_panel_map, ui_theme};

/// Fixed window width, matching the mesh editor so the two read as one family.
const WINDOW_WIDTH: f32 = 268.0;
/// Height of the two big fit buttons.
const FIT_BUTTON_HEIGHT: f32 = 34.0;
/// Height of a small labelled control.
const CHIP_HEIGHT: f32 = 26.0;
/// Corner radius shared by every control in the window.
const CHIP_ROUNDING: f32 = 5.0;

/// The two ways exocad's Align Meshes works, and the two this window offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignTab {
    /// Click matching points, then let the software fit them.
    #[default]
    Automatically,
    /// Drag the scan into place by hand, and paint what the match runs on.
    Manually,
}

impl AlignTab {
    /// The label on the tab.
    fn label(self) -> &'static str {
        match self {
            Self::Automatically => "Automatically",
            Self::Manually => "Manually",
        }
    }
}

/// What the operator asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignPanelAction {
    /// Fit the clicked pairs.
    Align,
    /// Seat the surfaces against each other.
    Refine,
    /// Remove the last point or pair.
    Back,
    /// Drop the points and start over.
    Clear,
    /// Re-measure with the current settings.
    Measure,
    /// Stop showing the map.
    HideMap,
    /// Run a whole-region command.
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
    /// The region brush, edited in place.
    pub(crate) brush: &'a mut AlignBrush,
    /// The last thing that happened, in a sentence.
    pub(crate) status: Option<&'a str>,
    /// The measurement summary, when there is one.
    pub(crate) stats: Option<DeviationStats>,
    /// Whether a job is in flight.
    pub(crate) busy: bool,
    /// Whether anything has actually moved this session.
    pub(crate) moved: bool,
    /// The open tab, switched in place.
    pub(crate) tab: &'a mut AlignTab,
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

/// The window body: the open tab, then the blocks both tabs share.
fn body(ui: &mut egui::Ui, view: AlignPanelView<'_>) -> Option<AlignPanelAction> {
    let enabled = !view.busy;
    tab_strip(ui, view.tab);
    // The brush belongs to the Manually tab. Leaving it armed after a tab
    // switch would keep swallowing viewport clicks from a tab that has no
    // brush control on it at all.
    if *view.tab != AlignTab::Manually {
        view.brush.set_armed(false);
    }
    ui.add_space(4.0);

    let mut action = match *view.tab {
        AlignTab::Automatically => automatically(ui, view.tool, enabled),
        AlignTab::Manually => manually(ui, view.constraint, view.brush, enabled),
    };

    ui.add_space(4.0);
    ui.separator();
    action = action.or(fits(ui, view.tool, enabled));
    ui.separator();
    action = action.or(align_panel_map::show(
        ui,
        view.settings,
        view.stats,
        enabled,
    ));
    status(ui, view.status);
    // Deliberately not gated on `enabled`: a refine on a full arch takes real
    // time, and a window whose only two exits are greyed out reads as a hang.
    action.or(commit(ui, view.moved))
}

/// The two-tab strip, sized so both halves are equally reachable.
fn tab_strip(ui: &mut egui::Ui, tab: &mut AlignTab) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        for value in [AlignTab::Automatically, AlignTab::Manually] {
            let active = *tab == value;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, CHIP_HEIGHT), egui::Sense::click());
            let ink = if active {
                ui_theme::ACCENT
            } else {
                ui_theme::TEXT_WEAK
            };
            let painter = ui.painter();
            if active {
                painter.rect_filled(rect, 4.0, ui_theme::ACCENT.gamma_multiply(0.14));
            } else if response.hovered() {
                painter.rect_filled(rect, 4.0, ui_theme::ACCENT.gamma_multiply(0.07));
            }
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                value.label(),
                egui::FontId::proportional(12.5),
                ink,
            );
            if active {
                painter.hline(
                    egui::Rangef::new(rect.left() + 6.0, rect.right() - 6.0),
                    rect.bottom() - 1.0,
                    egui::Stroke::new(1.6, ui_theme::ACCENT),
                );
            }
            if response.clicked() {
                *tab = value;
            }
        }
    });
}

/// The Automatically tab: click matching points, then take one back.
fn automatically(ui: &mut egui::Ui, tool: &AlignTool, enabled: bool) -> Option<AlignPanelAction> {
    prompt(ui, tool);
    let mut action = None;
    let placed = tool.pending().is_some() || !tool.pairs().is_empty();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(EditorIcon::Undo),
            "Undo point",
            enabled && placed,
            false,
        )
        .on_hover_text("Take the last point back — right-clicking in the view does the same")
        .clicked()
        {
            action = Some(AlignPanelAction::Back);
        }
        if chip(
            ui,
            width,
            Some(EditorIcon::Delete),
            "Clear",
            enabled && placed,
            false,
        )
        .on_hover_text("Drop every point and start over")
        .clicked()
        {
            action = Some(AlignPanelAction::Clear);
        }
    });
    action
}

/// The Manually tab: move the scan, or paint the region the match runs on.
fn manually(
    ui: &mut egui::Ui,
    constraint: &mut DragConstraint,
    brush: &mut AlignBrush,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut painting = brush.is_armed();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(EditorIcon::MoveLayer),
            "Move",
            enabled,
            !painting,
        )
        .on_hover_text("Drag the scan into place by hand")
        .clicked()
        {
            painting = false;
        }
        if chip(
            ui,
            width,
            Some(EditorIcon::MaskBrush),
            "Paint",
            enabled,
            painting,
        )
        .on_hover_text("Paint the part of the scan the match runs on")
        .clicked()
        {
            painting = true;
        }
    });
    brush.set_armed(painting);
    ui.add_space(2.0);
    if painting {
        region(ui, brush, enabled)
    } else {
        moving(ui, constraint);
        None
    }
}

/// Moving the scan by hand.
fn moving(ui: &mut egui::Ui, constraint: &mut DragConstraint) {
    hint(ui, "Drag to move · Ctrl+drag to turn");
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0;
        for value in [
            DragConstraint::Free,
            DragConstraint::ZOnly,
            DragConstraint::XyPlane,
        ] {
            if chip(ui, width, None, value.label(), true, *constraint == value)
                .on_hover_text(value.hint())
                .clicked()
            {
                *constraint = value;
            }
        }
    });
}

/// Choosing which part of the scan the match runs on.
///
/// The three whole-region buttons are always in view, not hidden behind the
/// brush: "match on this and nothing else" starts with None and then paints,
/// so a None the operator has to find first is a workflow they never discover.
fn region(ui: &mut egui::Ui, brush: &mut AlignBrush, enabled: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        for value in [BrushPaint::Ignore, BrushPaint::Use] {
            if chip(
                ui,
                width,
                None,
                value.label(),
                enabled,
                brush.paint() == value,
            )
            .on_hover_text(value.hint())
            .clicked()
            {
                brush.set_paint(value);
            }
        }
    });
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0;
        for (command, icon) in [
            (MaskCommand::Everything, EditorIcon::SelectAll),
            (MaskCommand::Nothing, EditorIcon::SelectNone),
            (MaskCommand::Invert, EditorIcon::SelectInvert),
        ] {
            if chip(ui, width, Some(icon), command.label(), enabled, false)
                .on_hover_text(command.hint())
                .clicked()
            {
                action = Some(AlignPanelAction::Mask(command));
            }
        }
    });
    let mut radius = brush.radius_mm();
    if ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut radius, 0.1..=20.0)
                .suffix(" mm")
                .text("size"),
        )
        .changed()
    {
        brush.set_radius_mm(radius);
    }
    hint(ui, "Shift+wheel resizes the brush");
    action
}

/// What the tool is waiting for, in one line.
fn prompt(ui: &mut egui::Ui, tool: &AlignTool) {
    let placed = tool.pairs().len();
    let text = if tool.moving_layer().is_none() {
        "Click a point on the scan that should move".to_owned()
    } else if tool.pending().is_some() {
        "Click the matching spot on the other scan".to_owned()
    } else if placed == 0 {
        "Click a point on each scan to pair them".to_owned()
    } else {
        format!("{placed} pair{} placed", if placed == 1 { "" } else { "s" })
    };
    ui.label(egui::RichText::new(text).size(12.0).color(ui_theme::TEXT));
    ui.add_space(2.0);
}

/// The two fits, shown under both tabs.
///
/// Refine is the primary action and is sized like one: the point fit only gets
/// the scan close, the surface fit is what actually seats it.
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
    .on_hover_text("Move the scan onto the clicked pairs — a rough placement")
    .clicked()
    {
        action = Some(AlignPanelAction::Align);
    }
    if fit_button(
        ui,
        width,
        EditorIcon::AlignRefine,
        "Best fit matching",
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

/// A short muted line of guidance.
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(11.0)
            .color(ui_theme::TEXT_MUTED),
    );
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
fn commit(ui: &mut egui::Ui, moved: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if tall_button(ui, width, "Cancel", false)
            .on_hover_text(if moved {
                "Put every scan back where it was and close"
            } else {
                "Close without changing anything"
            })
            .clicked()
        {
            action = Some(AlignPanelAction::Cancel);
        }
        if tall_button(ui, width, "Done", true)
            .on_hover_text("Keep the alignment and close")
            .clicked()
        {
            action = Some(AlignPanelAction::Done);
        }
    });
    action
}

/// A compact control: optional glyph, then a label, on a rounded plate.
///
/// One widget for every small control in the window, so a toggle, a command,
/// and a mode read as the same kind of thing and differ only in whether they
/// stay lit.
// A control needs its width, glyph, label, and both state flags. Bundling them
// into a struct would only add ceremony at each of the eight call sites.
#[allow(clippy::too_many_arguments)]
fn chip(
    ui: &mut egui::Ui,
    width: f32,
    icon: Option<EditorIcon>,
    label: &str,
    enabled: bool,
    active: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, CHIP_HEIGHT), sense);
    let ink = if enabled {
        if active {
            ui_theme::ACCENT
        } else {
            ui_theme::TEXT
        }
    } else {
        ui.visuals().weak_text_color()
    };
    let painter = ui.painter();
    if enabled && active {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::ACCENT.gamma_multiply(0.16));
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::ACCENT.gamma_multiply(0.08));
    }
    painter.rect_stroke(
        rect,
        CHIP_ROUNDING,
        egui::Stroke::new(1.0, ink.gamma_multiply(if active { 0.70 } else { 0.30 })),
    );
    let font = egui::FontId::proportional(11.5);
    match icon {
        Some(icon) => {
            let text_width = painter
                .layout_no_wrap(label.to_owned(), font.clone(), ink)
                .rect
                .width();
            let glyph_side = 15.0;
            let block = glyph_side + 5.0 + text_width;
            let left = rect.center().x - block / 2.0;
            let glyph = egui::Rect::from_center_size(
                egui::pos2(left + glyph_side / 2.0, rect.center().y),
                egui::Vec2::splat(glyph_side),
            );
            mesh_editor_icons::paint(painter, glyph, icon, ink, active);
            painter.text(
                egui::pos2(glyph.right() + 5.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                font,
                ink,
            );
        }
        None => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, label, font, ink);
        }
    }
    response
}

/// A full-width fit button: glyph, then label, at a size that says which of the
/// two is the one that matters.
// Same shape as `chip`, and the same reason for taking its parts loose.
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
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::ACCENT.gamma_multiply(0.14));
    }
    if enabled && response.hovered() {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::ACCENT.gamma_multiply(0.10));
    }
    painter.rect_stroke(
        rect,
        CHIP_ROUNDING,
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

/// A tall commit button.
fn tall_button(ui: &mut egui::Ui, width: f32, label: &str, primary: bool) -> egui::Response {
    let text = egui::RichText::new(label).size(12.5).color(if primary {
        ui_theme::ACCENT
    } else {
        ui_theme::TEXT
    });
    ui.add(egui::Button::new(text).min_size(egui::vec2(width, 28.0)))
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

    /// The bug this test exists for: the fits lived inside the Automatically
    /// tab, so switching to Manually left an operator who had just nudged a
    /// scan by hand with no way to seat it and no way to measure it. Both are
    /// drawn once, by the shared body, after whichever tab is open.
    #[test]
    fn both_tabs_reach_the_fits() {
        let source = production();
        let body = source
            .split_once("fn body(")
            .and_then(|(_, rest)| rest.split_once("\n/// The two-tab strip"))
            .map(|(body, _)| body)
            .expect("a shared window body");
        assert!(
            body.contains("action.or(fits(ui, view.tool, enabled))"),
            "the fits must be drawn by the shared body, not by one tab"
        );
        for tab in ["fn automatically(", "fn manually("] {
            let block = source
                .split_once(tab)
                .and_then(|(_, rest)| rest.split_once("\n/// "))
                .map(|(block, _)| block)
                .expect("a tab body");
            assert!(
                !block.contains("AlignPanelAction::Refine"),
                "{tab} draws its own Refine, so the two tabs would disagree"
            );
        }
    }

    /// The operator's words: "Paint the map on the other scan instead" is
    /// "полное бредятина". It asked them which surface should carry a colour,
    /// which is a rendering question dressed up as a measurement one.
    #[test]
    fn the_window_never_asks_which_surface_carries_the_map() {
        let source = production();
        for gone in ["SwapMapped", "EditorIcon::Swap", "other scan instead"] {
            assert!(!source.contains(gone), "{gone} is back in the window");
        }
    }

    /// Every small control carries a glyph or is one of a labelled set. A bare
    /// row of text buttons is what the window looked like when the operator
    /// called it a mess.
    #[test]
    fn the_region_commands_are_drawn_with_icons() {
        let region = production()
            .split_once("fn region(")
            .map(|(_, rest)| rest)
            .expect("a region block");
        for icon in [
            "EditorIcon::SelectAll",
            "EditorIcon::SelectNone",
            "EditorIcon::SelectInvert",
        ] {
            assert!(region.contains(icon), "the region commands need {icon}");
        }
    }
}

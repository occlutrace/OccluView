//! The Align Scans window, laid out the way the operator's dental CAD
//! software lays out its own align-meshes window.
//!
//! A movable window built from the same pieces as the mesh editor: no title
//! bar, icon buttons, and a commit row that ends in Cancel and Done. Nothing
//! here names a target, a source, or a role — the pair comes from the points
//! the operator clicks in the viewport.
//!
//! The two tabs carry different work, exactly as the operator's tools do:
//!
//! * **Automatically** is where the alignment happens. Arrows, Back, Perform
//!   alignment, Best fit matching, the two matching sliders, the orientation
//!   rule, and the two checkboxes that open the Brush tool window and the
//!   distance map.
//! * **Manually** is only hand movement: the three drag constraints and
//!   Undo/Redo. No brush lives here — an earlier build put one here and it was
//!   in the wrong place twice over, because the region it paints is an input to
//!   *best-fit matching*, which is an automatic-tab action.

use eframe::egui;
use occluview_align::{DeviationStats, Orientation};

use crate::align_drag::DragConstraint;
use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::mesh_editor_icons::{self, EditorIcon};
use crate::{align_panel_map, ui_theme};

/// Fixed window width, matching the mesh editor so the two read as one family.
const WINDOW_WIDTH: f32 = 272.0;
/// Height of the two big fit buttons.
const FIT_BUTTON_HEIGHT: f32 = 34.0;
/// Height of a small labelled control.
pub(crate) const CHIP_HEIGHT: f32 = 26.0;
/// Corner radius shared by every control in the window.
pub(crate) const CHIP_ROUNDING: f32 = 5.0;

/// The two ways the operator's dental CAD software's align-meshes works, and
/// the two this window offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignTab {
    /// Click matching points, then let the software fit them.
    #[default]
    Automatically,
    /// Drag the scan into place by hand.
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
    /// Fit the clicked pairs — the operator's dental CAD "Perform alignment".
    Align,
    /// Seat the surfaces against each other — the operator's dental CAD
    /// "Best fit matching".
    Refine,
    /// Remove the last arrow — the operator's dental CAD "Back".
    Back,
    /// Turn the pair around: the scan that was staying put is the one that moves.
    SwapRoles,
    /// Drop every arrow and both scan names, so a different pair can be picked.
    Clear,
    /// Re-measure with the current settings.
    Measure,
    /// Stop showing the map.
    HideMap,
    /// Step back through the scene history.
    Undo,
    /// Step forward through the scene history.
    Redo,
    /// Put every scan back where it was and close.
    Cancel,
    /// Keep the alignment and close.
    Done,
}

/// Everything the window needs to draw itself.
// Four INDEPENDENT facts about the session (a job is running, something moved,
// history can step back, history can step forward). They are not a state
// machine an enum would simplify — every combination of them occurs.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct AlignPanelView<'a> {
    /// The click model.
    pub(crate) tool: &'a AlignTool,
    /// Live settings, edited in place.
    pub(crate) settings: &'a mut AlignSettings,
    /// Which directions a hand drag may move in, edited in place.
    pub(crate) constraint: &'a mut DragConstraint,
    /// Whether the Brush tool window is open, edited in place.
    pub(crate) excluding: &'a mut bool,
    /// Set when a half-placed arrow has to go, because the tab that places
    /// arrows is no longer open.
    pub(crate) drop_pending: &'a mut bool,
    /// The last thing that happened, in a sentence.
    pub(crate) status: Option<&'a str>,
    /// The measurement summary, when there is one.
    pub(crate) stats: Option<DeviationStats>,
    /// Which scan moves onto which, once both are named.
    pub(crate) roles: Option<crate::align_panel_roles::AlignRoles>,
    /// Whether a job is in flight.
    pub(crate) busy: bool,
    /// Whether anything has actually moved this session.
    pub(crate) moved: bool,
    /// Whether the scene history has anything to step back to.
    pub(crate) can_undo: bool,
    /// Whether the scene history has anything to step forward to.
    pub(crate) can_redo: bool,
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

/// The window body: the open tab, then the commit row both tabs share.
fn body(ui: &mut egui::Ui, mut view: AlignPanelView<'_>) -> Option<AlignPanelAction> {
    let enabled = !view.busy;
    tab_strip(ui, view.tab);
    // The brush is an input to best-fit matching, which lives on the automatic
    // tab. Left open behind the manual tab it would keep swallowing the drags
    // that tab exists to receive.
    if *view.tab != AlignTab::Automatically {
        *view.excluding = false;
    }
    // A half-placed arrow belongs to the tab that places arrows. Left behind it
    // draws a rubber band to a cursor that is now dragging the mesh, and its
    // other half lands on whatever the operator clicks next time they come
    // back — a pair they never meant to make.
    *view.drop_pending = *view.tab != AlignTab::Automatically && view.tool.pending().is_some();
    ui.add_space(4.0);

    let mut action = match *view.tab {
        AlignTab::Automatically => automatically(ui, &mut view, enabled),
        AlignTab::Manually => manually(ui, view.constraint, view.can_undo, view.can_redo, enabled),
    };
    status(ui, view.status);
    // Deliberately not gated on `enabled`: a refine on a full arch takes real
    // time, and a window whose only two exits are greyed out reads as a hang.
    action = action.or(commit(ui, view.moved));
    action
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

/// The Automatically tab: arrows, the two fits, and what feeds them.
fn automatically(
    ui: &mut egui::Ui,
    view: &mut AlignPanelView<'_>,
    enabled: bool,
) -> Option<AlignPanelAction> {
    let mut action = if crate::align_panel_roles::show(ui, view.roles.as_ref(), enabled) {
        Some(AlignPanelAction::SwapRoles)
    } else {
        None
    };
    prompt(ui, view.tool);
    action = action.or(back(ui, view.tool, enabled));
    action = action.or(fits(ui, view.tool, enabled));
    ui.add_space(2.0);
    matching(ui, view.settings, enabled);
    ui.separator();
    exclude(ui, view.excluding, enabled);
    action.or(align_panel_map::show(
        ui,
        view.settings,
        view.stats,
        enabled,
    ))
}

/// The Manually tab: the three drag constraints and the history buttons.
fn manually(
    ui: &mut egui::Ui,
    constraint: &mut DragConstraint,
    can_undo: bool,
    can_redo: bool,
    enabled: bool,
) -> Option<AlignPanelAction> {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0;
        for value in [
            DragConstraint::Free,
            DragConstraint::ZOnly,
            DragConstraint::XyPlane,
        ] {
            if chip(
                ui,
                width,
                Some(value.icon()),
                "",
                true,
                *constraint == value,
            )
            .on_hover_text(value.hint())
            .clicked()
            {
                *constraint = value;
            }
        }
    });
    hint(ui, constraint.label());
    ui.add_space(2.0);
    // States the rule, because the rule is not what the other tab does. There
    // the roles are fixed and named; here the scan under the cursor is the one
    // that moves, the fixed scan included — and an operator who grabbed the arch
    // they did not mean to had nothing on screen to tell them so.
    hint(ui, "Drags whichever scan you grab · Ctrl+drag turns it");
    ui.add_space(4.0);

    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(EditorIcon::Undo),
            "Undo",
            enabled && can_undo,
            false,
        )
        .on_hover_text("Go back one step")
        .clicked()
        {
            action = Some(AlignPanelAction::Undo);
        }
        if chip(
            ui,
            width,
            Some(EditorIcon::Redo),
            "Redo",
            enabled && can_redo,
            false,
        )
        .on_hover_text("Go forward one step")
        .clicked()
        {
            action = Some(AlignPanelAction::Redo);
        }
    });
    action
}

/// What the tool is waiting for, in one line.
fn prompt(ui: &mut egui::Ui, tool: &AlignTool) {
    let placed = tool.pairs().len();
    let text = if tool.moving_layer().is_none() {
        "Click a point on the mesh that should move".to_owned()
    } else if tool.pending().is_some() {
        "Click the same position on the other mesh".to_owned()
    } else if placed == 0 {
        "Click alternating points at the same positions on the two meshes".to_owned()
    } else {
        format!(
            "{placed} arrow{} placed",
            if placed == 1 { "" } else { "s" }
        )
    };
    ui.label(egui::RichText::new(text).size(12.0).color(ui_theme::TEXT));
    ui.add_space(2.0);
}

/// The same "Back" the operator's dental CAD software offers, and the way out
/// to a different pair of scans.
///
/// **Clear** is not decoration. Once two scans are paired, a click on a third is
/// refused — and the refusal used to tell the operator to "press Clear", which
/// was not a control that existed. Aligning a third file in the same session
/// meant closing the tool and opening it again.
fn back(ui: &mut egui::Ui, tool: &AlignTool, enabled: bool) -> Option<AlignPanelAction> {
    let placed = tool.pending().is_some() || !tool.pairs().is_empty();
    let paired = tool.moving_layer().is_some();
    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(EditorIcon::Undo),
            "Back",
            enabled && placed,
            false,
        )
        .on_hover_text("Undo an arrow — a right-click in the view does the same")
        .clicked()
        {
            action = Some(AlignPanelAction::Back);
        }
        if chip(ui, width, None, "Clear", enabled && paired, false)
            .on_hover_text(
                "Drop every arrow and pick two scans again — the scans stay where they are",
            )
            .clicked()
        {
            action = Some(AlignPanelAction::Clear);
        }
    });
    action
}

/// The two fits. Best fit matching is the primary action and is sized like one:
/// the point fit only gets the mesh close, the surface fit is what seats it.
fn fits(ui: &mut egui::Ui, tool: &AlignTool, enabled: bool) -> Option<AlignPanelAction> {
    let mut action = None;
    let width = ui.available_width();
    if fit_button(
        ui,
        width,
        EditorIcon::AlignFit,
        "Perform alignment",
        tool.can_align() && enabled,
        false,
    )
    .on_hover_text("Move the mesh onto the arrows — needs at least two arrows")
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
    .on_hover_text("Seat the surfaces against each other. Only for identically shaped meshes")
    .clicked()
    {
        action = Some(AlignPanelAction::Refine);
    }
    action
}

/// The two sliders and the orientation rule that steer best-fit matching.
fn matching(ui: &mut egui::Ui, settings: &mut AlignSettings, enabled: bool) {
    ui.add_enabled(
        enabled,
        egui::Slider::new(&mut settings.matching_ratio, 0.1..=1.0)
            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
            .text("matching parts"),
    )
    .on_hover_text(
        "The share of surface that exists on both meshes. 70-80% suits mesh pairs \
         whose topology is largely the same",
    );
    ui.add_enabled(
        enabled,
        egui::Slider::new(&mut settings.influence_radius_mm, 0.2..=10.0)
            .suffix(" mm")
            .text("max influence"),
    )
    .on_hover_text(
        "Only surface below this distance influences the matching. A large value can \
         worsen the result",
    );
    ui.collapsing("Surfaces orientation shall match", |ui| {
        facing(ui, &mut settings.orientation);
    });
}

/// The surface-orientation rule. An inverted mesh flips the whole signed map,
/// so this is the escape hatch for a scan whose winding disagrees with itself.
fn facing(ui: &mut egui::Ui, orientation: &mut Orientation) {
    for (value, label) in [
        (Orientation::Match, "Surfaces orientation shall match"),
        (
            Orientation::Inverted,
            "Surfaces orientation shall match inverted",
        ),
        (
            Orientation::Ignored,
            "Surfaces orientation shall be ignored",
        ),
    ] {
        if ui
            .radio(*orientation == value, label)
            .on_hover_text(if value == Orientation::Ignored {
                "Accepts either facing. The calculation often takes significantly longer"
            } else {
                "How the two surfaces are taken to face each other"
            })
            .clicked()
        {
            *orientation = value;
        }
    }
}

/// The same "Matching: Exclude selected parts" the operator's dental CAD
/// software uses: the checkbox that opens the Brush tool window.
fn exclude(ui: &mut egui::Ui, excluding: &mut bool, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.checkbox(excluding, "Matching: Exclude selected parts")
            .on_hover_text("Paint the surface best-fit matching must ignore");
    });
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
            // Spelt out when there is something to lose. An operator who reads
            // Cancel as "close the window" loses every move they made in the
            // session, and the only clue afterwards was a status line they had
            // already scrolled past. Ctrl+Z does bring it back — the restore is
            // one history step — so that is said here, where the decision is.
            .on_hover_text(if moved {
                "Put every scan back where it was and close — Ctrl+Z brings the alignment back"
            } else {
                "Close without changing anything"
            })
            .clicked()
        {
            action = Some(AlignPanelAction::Cancel);
        }
        if tall_button(ui, width, "Done", true)
            .on_hover_text("Keep the alignment and close — export the scan to write it to disk")
            .clicked()
        {
            action = Some(AlignPanelAction::Done);
        }
    });
    action
}

/// A compact control: optional glyph, then a label, on a rounded plate.
///
/// One widget for every small control in the window and in the Brush tool, so
/// a toggle, a command, and a mode read as the same kind of thing and differ
/// only in whether they stay lit.
// A control needs its width, glyph, label, and both state flags. Bundling them
// into a struct would only add ceremony at each call site.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chip(
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
    match (icon, label.is_empty()) {
        (Some(icon), true) => {
            let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0));
            mesh_editor_icons::paint(painter, glyph, icon, ink, active);
        }
        (Some(icon), false) => {
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
        (None, _) => {
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

    /// The operator's dental CAD software's labels, verbatim. The operator
    /// works in that dialog daily, and a control that does the same job under
    /// a different name is a control they have to translate before they can
    /// use it.
    #[test]
    fn the_controls_carry_the_labels_operators_already_know() {
        let source = production();
        for label in [
            "\"Back\"",
            "\"Perform alignment\"",
            "\"Best fit matching\"",
            "\"matching parts\"",
            "\"max influence\"",
            "\"Surfaces orientation shall match\"",
            "\"Surfaces orientation shall match inverted\"",
            "\"Surfaces orientation shall be ignored\"",
            "\"Matching: Exclude selected parts\"",
        ] {
            assert!(source.contains(label), "the window is missing {label}");
        }
    }

    /// The bug this test exists for: the brush was put on the manual tab. The
    /// region it paints is an input to BEST-FIT MATCHING, which is an automatic
    /// tab action, so on the manual tab it was both unreachable at the moment
    /// it mattered and in the way of the drags that tab exists for.
    #[test]
    fn the_exclusion_brush_belongs_to_the_automatic_tab() {
        let source = production();
        let manual = source
            .split_once("fn manually(")
            .and_then(|(_, rest)| rest.split_once("\n/// What the tool is waiting for"))
            .map(|(block, _)| block)
            .expect("a manual tab body");
        for absent in ["excluding", "brush", "Brush"] {
            assert!(
                !manual.contains(absent),
                "the manual tab mentions {absent}, which belongs to the automatic tab"
            );
        }
        let automatic = source
            .split_once("fn automatically(")
            .and_then(|(_, rest)| rest.split_once("\n/// The Manually tab"))
            .map(|(block, _)| block)
            .expect("an automatic tab body");
        assert!(automatic.contains("exclude(ui, view.excluding, enabled)"));
    }

    /// The operator's report was that the manual tab had no action on it at
    /// all. The operator's dental CAD software has Undo and Redo, and so does
    /// this one.
    #[test]
    fn the_manual_tab_offers_the_history_buttons() {
        let manual = production()
            .split_once("fn manually(")
            .map(|(_, rest)| rest)
            .expect("a manual tab body");
        assert!(manual.contains("AlignPanelAction::Undo"));
        assert!(manual.contains("AlignPanelAction::Redo"));
    }

    /// "Paint the map on the other scan instead" asked the operator which
    /// surface should carry a colour, which is a rendering question dressed up
    /// as a measurement one.
    #[test]
    fn the_window_never_asks_which_surface_carries_the_map() {
        let source = production();
        for gone in ["SwapMapped", "EditorIcon::Swap", "other scan instead"] {
            assert!(!source.contains(gone), "{gone} is back in the window");
        }
    }
}

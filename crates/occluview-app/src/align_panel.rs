//! The Align Scans window.
//!
//! The window uses the mesh-editor controls and derives scan roles from the
//! points selected in the viewport.
//!
//! Automatic alignment and manual movement are kept on separate tabs. The
//! exclusion brush belongs to automatic matching because its mask is an input
//! to the fit.

use eframe::egui;
use occluview_align::DeviationStats;

use crate::align_drag::DragConstraint;
use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::icons::AppIcon;
use crate::{align_panel_map, ui_theme};

/// Fixed window width, matching the mesh editor so the two read as one family.
const WINDOW_WIDTH: f32 = 272.0;
/// Height of the two big fit buttons.
const FIT_BUTTON_HEIGHT: f32 = 34.0;
/// Height of a small labelled control.
pub(crate) const CHIP_HEIGHT: f32 = 26.0;
/// Corner radius shared by every control in the window.
pub(crate) const CHIP_ROUNDING: f32 = 5.0;

/// Alignment modes exposed by the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignTab {
    /// Click matching points, then let the software fit them.
    #[default]
    Automatically,
    /// Drag the scan into place by hand.
    Manually,
}

impl AlignTab {
    /// The English label on the tab (operator vocabulary reference, pinned
    /// by the en-wording lock test).
    #[cfg(test)]
    fn label(self) -> &'static str {
        match self {
            Self::Automatically => "Automatically",
            Self::Manually => "Manually",
        }
    }

    /// Catalog key rendering the localized tab label.
    fn label_key(self) -> &'static str {
        match self {
            Self::Automatically => "align-tab-auto",
            Self::Manually => "align-tab-manual",
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
// These flags represent independent session facts.
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
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let default_pos = viewport_rect.right_top() + egui::vec2(-WINDOW_WIDTH - 16.0, 16.0);
    let mut action = None;
    egui::Window::new(locale.text("align-panel-title"))
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
            action = body(ui, view, locale);
        });
    action
}

/// The window body: the open tab, then the commit row both tabs share.
fn body(
    ui: &mut egui::Ui,
    mut view: AlignPanelView<'_>,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let enabled = !view.busy;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), rect, AppIcon::Align, ui_theme::accent());
        ui.label(
            egui::RichText::new(locale.tr("align-title"))
                .strong()
                .size(14.0),
        );
    });
    ui.add_space(2.0);
    tab_strip(ui, view.tab, locale);
    // The brush is available only on the automatic tab.
    if *view.tab != AlignTab::Automatically {
        *view.excluding = false;
    }
    // Discard a pending point when leaving the tab that places points.
    *view.drop_pending = *view.tab != AlignTab::Automatically && view.tool.pending().is_some();
    ui.add_space(4.0);

    let mut action = match *view.tab {
        AlignTab::Automatically => automatically(ui, &mut view, enabled, locale),
        AlignTab::Manually => manually(
            ui,
            view.constraint,
            view.can_undo,
            view.can_redo,
            enabled,
            locale,
        ),
    };
    status(ui, view.status);
    // Keep Cancel and Done available while work is in flight.
    action = action.or(commit(ui, view.moved, locale));
    action
}

/// The two-tab strip, sized so both halves are equally reachable.
fn tab_strip(ui: &mut egui::Ui, tab: &mut AlignTab, locale: &crate::i18n::LocaleManager) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        for value in [AlignTab::Automatically, AlignTab::Manually] {
            let active = *tab == value;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, CHIP_HEIGHT), egui::Sense::click());
            let ink = if active {
                ui_theme::accent()
            } else {
                ui_theme::text_weak()
            };
            let painter = ui.painter();
            if active {
                painter.rect_filled(rect, 4.0, ui_theme::accent().gamma_multiply(0.14));
            } else if response.hovered() {
                painter.rect_filled(rect, 4.0, ui_theme::accent().gamma_multiply(0.07));
            }
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                locale.tr(value.label_key()).as_str(),
                egui::FontId::proportional(12.5),
                ink,
            );
            if active {
                painter.hline(
                    egui::Rangef::new(rect.left() + 6.0, rect.right() - 6.0),
                    rect.bottom() - 1.0,
                    egui::Stroke::new(1.6_f32, ui_theme::accent()),
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
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let mut action = if crate::align_panel_roles::show(ui, view.roles.as_ref(), enabled, locale) {
        Some(AlignPanelAction::SwapRoles)
    } else {
        None
    };
    prompt(ui, view.tool, locale);
    action = action.or(back(ui, view.tool, enabled, locale));
    action = action.or(fits(ui, view.tool, enabled, locale));
    ui.add_space(2.0);
    crate::align_panel_settings::matching(ui, view.settings, enabled, locale);
    ui.separator();
    crate::align_panel_settings::exclude(ui, view.excluding, enabled, locale);
    action.or(align_panel_map::show(
        ui,
        view.settings,
        view.stats,
        enabled,
        locale,
    ))
}

/// The Manually tab: the three drag constraints and the history buttons.
// Six inherently (ui/ctx + data + locale); bundling would fake an abstraction.
#[expect(clippy::too_many_arguments)]
fn manually(
    ui: &mut egui::Ui,
    constraint: &mut DragConstraint,
    can_undo: bool,
    can_redo: bool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
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
            .on_hover_text(locale.tr(value.hint_key()))
            .clicked()
            {
                *constraint = value;
            }
        }
    });
    hint(ui, &locale.tr(constraint.label_key()));
    ui.add_space(2.0);
    // States the rule, because the rule is not what the other tab does. There
    // the roles are fixed and named; here the scan under the cursor is the one
    // that moves, the fixed scan included — and an operator who grabbed the arch
    // they did not mean to had nothing on screen to tell them so.
    hint(ui, &locale.tr("align-manual-drag-hint"));
    ui.add_space(4.0);

    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(AppIcon::Undo),
            &locale.tr("align-undo"),
            enabled && can_undo,
            false,
        )
        .on_hover_text(locale.tr("align-undo-hint"))
        .clicked()
        {
            action = Some(AlignPanelAction::Undo);
        }
        if chip(
            ui,
            width,
            Some(AppIcon::Redo),
            &locale.tr("align-redo"),
            enabled && can_redo,
            false,
        )
        .on_hover_text(locale.tr("align-redo-hint"))
        .clicked()
        {
            action = Some(AlignPanelAction::Redo);
        }
    });
    action
}

/// What the tool is waiting for, in one line.
fn prompt(ui: &mut egui::Ui, tool: &AlignTool, locale: &crate::i18n::LocaleManager) {
    let placed = tool.pairs().len();
    let text = if tool.moving_layer().is_none() {
        locale.tr("align-prompt-moving")
    } else if tool.pending().is_some() {
        locale.tr("align-prompt-other")
    } else if placed == 0 {
        locale.tr("align-prompt-alternate")
    } else {
        locale.tr_plural("align-prompt-placed", &[], &[("count", placed)])
    };
    ui.label(egui::RichText::new(text).size(12.0).color(ui_theme::text()));
    ui.add_space(2.0);
}

/// Point-pair history controls and pair reset.
fn back(
    ui: &mut egui::Ui,
    tool: &AlignTool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let placed = tool.pending().is_some() || !tool.pairs().is_empty();
    let paired = tool.moving_layer().is_some();
    let mut action = None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if chip(
            ui,
            width,
            Some(AppIcon::Undo),
            &locale.tr("align-back"),
            enabled && placed,
            false,
        )
        .on_hover_text(locale.tr("align-back-hint"))
        .clicked()
        {
            action = Some(AlignPanelAction::Back);
        }
        if chip(
            ui,
            width,
            None,
            &locale.tr("align-clear"),
            enabled && paired,
            false,
        )
        .on_hover_text(locale.tr("align-clear-hint").as_str())
        .clicked()
        {
            action = Some(AlignPanelAction::Clear);
        }
    });
    action
}

/// The two fits. Best fit matching is the primary action and is sized like one:
/// the point fit only gets the mesh close, the surface fit is what seats it.
fn fits(
    ui: &mut egui::Ui,
    tool: &AlignTool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let mut action = None;
    let width = ui.available_width();
    if fit_button(
        ui,
        width,
        AppIcon::AlignFit,
        &locale.tr("align-fit-perform"),
        tool.can_align() && enabled,
        false,
    )
    .on_hover_text(locale.tr("align-fit-perform-hint"))
    .clicked()
    {
        action = Some(AlignPanelAction::Align);
    }
    if fit_button(
        ui,
        width,
        AppIcon::AlignRefine,
        &locale.tr("align-fit-refine"),
        tool.can_measure() && enabled,
        true,
    )
    .on_hover_text(locale.tr("align-fit-refine-hint"))
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
            .color(ui_theme::text_muted()),
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
            .color(ui_theme::text_weak()),
    );
}

/// Cancel and Done, the same commit pair the mesh editor ends on.
///
/// Cancel means what it says: every scan goes back where it was. Closing a tool
/// and silently keeping what it did is how an operator loses work they thought
/// they had discarded.
fn commit(
    ui: &mut egui::Ui,
    moved: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let mut action = None;
    let cancel_hint_moved = locale.tr("align-commit-cancel-hint-moved");
    let cancel_hint_clean = locale.tr("align-commit-cancel-hint-clean");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if tall_button(ui, width, &locale.tr("align-commit-cancel"), false)
            // Spelt out when there is something to lose. An operator who reads
            // Cancel as "close the window" loses every move they made in the
            // session, and the only clue afterwards was a status line they had
            // already scrolled past. Ctrl+Z does bring it back — the restore is
            // one history step — so that is said here, where the decision is.
            .on_hover_text(if moved {
                cancel_hint_moved.as_str()
            } else {
                cancel_hint_clean.as_str()
            })
            .clicked()
        {
            action = Some(AlignPanelAction::Cancel);
        }
        if tall_button(ui, width, &locale.tr("align-commit-done"), true)
            .on_hover_text(locale.tr("align-commit-done-hint").as_str())
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
    icon: Option<AppIcon>,
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
            ui_theme::accent()
        } else {
            ui_theme::text()
        }
    } else {
        ui.visuals().weak_text_color()
    };
    let painter = ui.painter();
    if enabled && active {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::accent().gamma_multiply(0.16));
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::accent().gamma_multiply(0.08));
    }
    painter.rect_stroke(
        rect,
        CHIP_ROUNDING,
        egui::Stroke::new(
            1.0_f32,
            ink.gamma_multiply(if active { 0.70 } else { 0.30 }),
        ),
        egui::StrokeKind::Middle,
    );
    let font = egui::FontId::proportional(11.5);
    match (icon, label.is_empty()) {
        (Some(icon), true) => {
            let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0));
            crate::icons::paint(painter, glyph, icon, ink);
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
            crate::icons::paint(painter, glyph, icon, ink);
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
    icon: AppIcon,
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
            ui_theme::accent()
        } else {
            ui_theme::text()
        }
    } else {
        ui.visuals().weak_text_color()
    };
    let painter = ui.painter();
    if enabled && primary {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::accent().gamma_multiply(0.14));
    }
    if enabled && response.hovered() {
        painter.rect_filled(rect, CHIP_ROUNDING, ui_theme::accent().gamma_multiply(0.10));
    }
    painter.rect_stroke(
        rect,
        CHIP_ROUNDING,
        egui::Stroke::new(
            1.0_f32,
            ink.gamma_multiply(if primary { 0.75 } else { 0.35 }),
        ),
        egui::StrokeKind::Middle,
    );
    let glyph = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 21.0, rect.center().y),
        egui::Vec2::splat(17.0),
    );
    crate::icons::paint(painter, glyph, icon, ink);
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
    if primary {
        let button = egui::Button::new(
            egui::RichText::new(label)
                .size(12.5)
                .strong()
                .color(ui_theme::on_accent()),
        )
        .fill(ui_theme::accent())
        .corner_radius(CHIP_ROUNDING);
        ui.add(button.min_size(egui::vec2(width, 28.0)))
    } else {
        let text = egui::RichText::new(label)
            .size(12.5)
            .color(ui_theme::text());
        ui.add(egui::Button::new(text).min_size(egui::vec2(width, 28.0)))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::AlignTab;

    /// The kept English tab labels render from the catalog verbatim.
    #[test]
    fn english_tab_labels_match_source_wording() {
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        for tab in [AlignTab::Automatically, AlignTab::Manually] {
            assert_eq!(catalog.text(tab.label_key()).as_deref(), Some(tab.label()));
        }
    }

    fn production() -> &'static str {
        let source = crate::primary_ui_tests::production_source(include_str!("align_panel.rs"));
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
        // Title resolves through `align-panel-title`; the stable explicit
        // id is what makes the window movable/persistent.
        assert!(source.contains("align-panel-title"));
        assert!(source.contains(".default_pos(default_pos)"));
        assert!(source.contains(".constrain_to(viewport_rect)"));
        assert!(
            !source.contains(".anchor("),
            "an anchored window cannot be moved out of the way"
        );
    }

    /// The window exposes explicit Cancel and Done actions.
    #[test]
    fn the_window_ends_in_cancel_and_done() {
        let commit = production()
            .split_once("fn commit(")
            .map(|(_, rest)| rest)
            .expect("a commit row");
        assert!(commit.contains("AlignPanelAction::Cancel"));
        assert!(commit.contains("AlignPanelAction::Done"));
    }

    /// Preserve the established control labels.
    #[test]
    fn the_controls_carry_the_labels_operators_already_know() {
        // The settings cluster lives in the sibling file under the same
        // per-file line budget; both files carry window controls.
        let source = format!(
            "{}{}",
            production(),
            crate::primary_ui_tests::production_source(include_str!("align_panel_settings.rs"))
        );
        // Control captions resolve through the catalog; the keys are what
        // the window must reference.
        for label in [
            "\"align-back\"",
            "\"align-fit-perform\"",
            "\"align-fit-refine\"",
            "\"align-matching-parts\"",
            "\"align-max-influence\"",
            "\"align-orientation-match\"",
            "\"align-orientation-inverted\"",
            "\"align-orientation-ignored\"",
            "\"align-exclude\"",
        ] {
            assert!(source.contains(label), "the window is missing {label}");
        }
    }

    /// The exclusion brush belongs to the automatic tab.
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
        assert!(automatic.contains("exclude(ui, view.excluding, enabled, locale)"));
    }

    /// The manual tab exposes Undo and Redo.
    #[test]
    fn the_manual_tab_offers_the_history_buttons() {
        let manual = production()
            .split_once("fn manually(")
            .map(|(_, rest)| rest)
            .expect("a manual tab body");
        assert!(manual.contains("AlignPanelAction::Undo"));
        assert!(manual.contains("AlignPanelAction::Redo"));
    }

    /// The map target is fixed by the measurement model.
    #[test]
    fn the_window_never_asks_which_surface_carries_the_map() {
        let source = production();
        for gone in ["SwapMapped", "AppIcon::Swap", "other scan instead"] {
            assert!(!source.contains(gone), "{gone} is back in the window");
        }
    }
}

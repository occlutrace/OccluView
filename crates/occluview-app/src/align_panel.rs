//! The Align Scans window.
//!
//! The window uses the mesh-editor controls and derives scan roles from the
//! points selected in the viewport.
//!
//! Automatic alignment and manual movement are kept on separate tabs. The
//! exclusion brush belongs to automatic matching because its mask is an input
//! to the fit.

use eframe::egui;

use crate::align_drag::DragConstraint;
use crate::align_tool::AlignTool;
use crate::align_worker::AlignSettings;
use crate::icons::AppIcon;
use crate::{align_panel_map, ui_theme};

/// Fixed window width, matching the mesh editor so the two read as one family.
const WINDOW_WIDTH: f32 = 320.0;
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
    /// Visible scene layers, used to keep the panel clear of Layers.
    pub(crate) layer_count: usize,
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
    /// Whether a landed Best fit matching result authorizes a heatmap.
    pub(crate) refined_match_ready: bool,
    /// Which scan moves onto which, once both are named.
    pub(crate) roles: Option<crate::align_panel_roles::AlignRoles>,
    /// Whether a job is in flight.
    pub(crate) busy: bool,
    /// Whether the worker stopped and cannot accept another job.
    pub(crate) worker_failed: bool,
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
    let default_pos = panel_default_pos(viewport_rect, view.layer_count);
    let mut action = None;
    let id = egui::Id::new("occluview_align_window");
    let previous_rect = ctx.memory(|memory| memory.area_rect(id));
    let mut window = egui::Window::new(locale.text("align-panel-title"))
        .id(id)
        .default_pos(default_pos)
        .constrain_to(viewport_rect)
        .resizable(false)
        .collapsible(false)
        .title_bar(false);
    if panel_needs_reanchor(previous_rect, viewport_rect, view.layer_count) {
        window = window.current_pos(default_pos);
    }
    window.show(ctx, |ui| {
        ui.set_min_width(WINDOW_WIDTH - 24.0);
        ui.set_width(WINDOW_WIDTH - 24.0);
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        ui.style_mut().animation_time = 0.05;
        action = body(ui, view, locale);
    });
    action
}

/// A dragged panel can outlive a viewport resize. Move it only when its saved
/// position would cover Layers or leave the available viewport.
fn panel_needs_reanchor(
    previous_rect: Option<egui::Rect>,
    viewport: egui::Rect,
    layer_count: usize,
) -> bool {
    previous_rect.is_some_and(|rect| {
        !viewport.contains_rect(rect)
            || rect.intersects(crate::layers_overlay::layer_overlay_rect(
                viewport,
                layer_count,
            ))
    })
}

/// Put the window at the top right when there is room, and below Layers in a
/// narrow viewport. The window remains draggable after its first appearance.
fn panel_default_pos(viewport: egui::Rect, layer_count: usize) -> egui::Pos2 {
    let left = (viewport.right() - WINDOW_WIDTH - 16.0).max(viewport.left() + 8.0);
    let layers = crate::layers_overlay::layer_overlay_rect(viewport, layer_count);
    let top = if left < layers.right() + 8.0 {
        layers.bottom() + 12.0
    } else {
        viewport.top() + 16.0
    };
    egui::pos2(left, top.min(viewport.bottom() - 40.0))
}

/// The window body: the open tab, then the commit row both tabs share.
fn body(
    ui: &mut egui::Ui,
    mut view: AlignPanelView<'_>,
    locale: &crate::i18n::LocaleManager,
) -> Option<AlignPanelAction> {
    let enabled = !view.busy && !view.worker_failed;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), rect, AppIcon::Align, ui_theme::accent());
        ui.label(
            egui::RichText::new(locale.tr("align-title"))
                .strong()
                .size(15.0)
                .color(ui_theme::text()),
        );
    });
    ui.add_space(4.0);
    ui.separator();
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
            if response.has_focus() {
                painter.rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.2_f32, ui_theme::accent()),
                    egui::StrokeKind::Inside,
                );
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
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    true,
                    active,
                    locale.tr(value.label_key()),
                )
            });
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
    action = action.or(fits(ui, view.tool, enabled, locale));
    ui.add_space(4.0);
    prompt(ui, view.tool, locale);
    action = action.or(back(ui, view.tool, enabled, locale));
    ui.add_space(2.0);
    crate::align_panel_settings::matching(ui, view.settings, enabled, locale);
    ui.separator();
    crate::align_panel_settings::exclude(ui, view.excluding, enabled, locale);
    action.or(align_panel_map::show(
        ui,
        view.settings,
        view.refined_match_ready,
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
            if compact_icon_chip(
                ui,
                width,
                value.icon(),
                &locale.tr(value.label_key()),
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
    chip_with_accessibility(ui, width, icon, label, label, enabled, active)
}

/// An icon-only chip keeps the compact visual layout while exposing the
/// localized command name to AccessKit.
#[allow(clippy::too_many_arguments)]
fn compact_icon_chip(
    ui: &mut egui::Ui,
    width: f32,
    icon: AppIcon,
    accessibility_label: &str,
    enabled: bool,
    active: bool,
) -> egui::Response {
    chip_with_accessibility(
        ui,
        width,
        Some(icon),
        "",
        accessibility_label,
        enabled,
        active,
    )
}

// The visual label and semantic label intentionally remain separate: the
// three constraint controls are compact icon chips, but never anonymous to a
// keyboard or screen-reader user.
#[allow(clippy::too_many_arguments)]
fn chip_with_accessibility(
    ui: &mut egui::Ui,
    width: f32,
    icon: Option<AppIcon>,
    label: &str,
    accessibility_label: &str,
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
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            CHIP_ROUNDING,
            egui::Stroke::new(1.2_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
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
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            enabled,
            active,
            accessibility_label,
        )
    });
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
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            CHIP_ROUNDING,
            egui::Stroke::new(1.2_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
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
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, primary, label)
    });
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
#[path = "align_panel_tests.rs"]
mod tests;

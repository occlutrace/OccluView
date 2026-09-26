//! The Brush tool window: the "Exclude selected parts" convention dental CAD
//! software uses, control for control.
//!
//! A **separate movable window**, opened by the "Matching: Exclude selected
//! parts" checkbox on the automatic tab, as dental CAD software opens it. It
//! is not a section of the main window and not a mode of the manual tab: the
//! operator is painting on the mesh with one hand while reading the alignment
//! controls with the other, so the two have to be positionable independently.

use eframe::egui;

use crate::align_brush::{AlignBrush, BrushTarget};
use crate::align_markings::{AlignSide, MaskCommand};
use crate::align_panel::chip;
use crate::align_panel_roles::AlignRoles;
use crate::icons::AppIcon;
use crate::ui_theme;

/// Fixed window width. Narrower than the main window: it holds one slider's
/// worth of content and floats over the mesh being painted.
const WINDOW_WIDTH: f32 = 236.0;

/// What the Brush tool window asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrushPanelAction {
    /// Run a whole-mesh command.
    Mask(MaskCommand),
    /// Close the window — the same as clearing the checkbox that opened it.
    Close,
}

/// Inputs for the compact Brush window. Grouping these values keeps the
/// caller's UI transaction readable without hiding which state is mutable.
pub(crate) struct BrushPanelView<'a> {
    pub(crate) viewport_rect: egui::Rect,
    pub(crate) brush: &'a mut AlignBrush,
    pub(crate) roles: Option<&'a AlignRoles>,
    pub(crate) enabled: bool,
    pub(crate) locale: &'a crate::i18n::LocaleManager,
}

/// Show the Brush tool window; returns what the operator asked for.
pub(crate) fn show(ctx: &egui::Context, view: BrushPanelView<'_>) -> Option<BrushPanelAction> {
    // Opens to the left of the main window's default corner, so the two do not
    // land on top of each other the first time the checkbox is ticked.
    let default_pos = view.viewport_rect.right_top() + egui::vec2(-WINDOW_WIDTH - 300.0, 16.0);
    let mut action = None;
    egui::Window::new(view.locale.tr("align-brush-title"))
        .id(egui::Id::new("occluview_align_brush_window"))
        .default_pos(default_pos)
        .constrain_to(view.viewport_rect)
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(WINDOW_WIDTH - 24.0);
            ui.set_width(WINDOW_WIDTH - 24.0);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            ui.style_mut().animation_time = 0.05;
            action = body(ui, view.brush, view.roles, view.enabled, view.locale);
        });
    action
}

/// The window body.
fn body(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    roles: Option<&AlignRoles>,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<BrushPanelAction> {
    let mut action = header(ui, locale);
    ui.add_space(2.0);
    if let Some(roles) = roles {
        mesh_selection(ui, brush, roles, enabled, locale);
        ui.add_space(2.0);
    }
    action = action.or(commands(ui, enabled, locale));
    ui.add_space(2.0);
    size(ui, brush, enabled, locale);
    automatic(ui, brush, enabled, locale);
    action
}

/// Select the physical mesh the brush edits.
///
/// The two scans overlap by design, so nearest-surface picking is not a safe
/// substitute: it can paint the other scan. Exocad exposes this as an
/// explicit Mesh selection, and keeping it visible in the Brush window makes
/// the target unambiguous while the operator works.
///
/// The window opens on **Both**, because the markings decide what matching
/// ignores on either surface and an operator who presses Fit nowhere with two
/// scans on screen means the pair. Naming one scan stays available for the
/// overlapping case, where it is the only way to stop the wrong surface taking
/// the stroke.
fn mesh_selection(
    ui: &mut egui::Ui,
    brush: &mut AlignBrush,
    roles: &AlignRoles,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    ui.label(
        egui::RichText::new(locale.tr("align-brush-mesh-selection"))
            .size(11.0)
            .color(ui_theme::text_muted()),
    );
    for target in BrushTarget::ALL {
        let label = match target {
            BrushTarget::Both => locale.tr("align-brush-both"),
            BrushTarget::Moving => {
                format!(
                    "{} · {}",
                    locale.tr("align-brush-moving"),
                    roles.side_name(AlignSide::Moving)
                )
            }
            BrushTarget::Fixed => {
                format!(
                    "{} · {}",
                    locale.tr("align-brush-fixed"),
                    roles.side_name(AlignSide::Fixed)
                )
            }
        };
        if chip(
            ui,
            ui.available_width(),
            None,
            &label,
            enabled,
            brush.target() == target,
        )
        .on_hover_text(if target == BrushTarget::Both {
            locale.tr("align-brush-both-hint")
        } else {
            String::new()
        })
        .clicked()
        {
            brush.set_target(target);
        }
    }
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
            if close_response.has_focus() {
                ui.painter().rect_stroke(
                    close_rect,
                    4.0,
                    egui::Stroke::new(1.2_f32, ui_theme::accent()),
                    egui::StrokeKind::Inside,
                );
            }
            close_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    locale.tr("align-brush-close-hint"),
                )
            });
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

/// The same whole-mesh commands dental CAD software offers,
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

/// The same "Mark automatic" control and radius dental CAD software uses.
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
}

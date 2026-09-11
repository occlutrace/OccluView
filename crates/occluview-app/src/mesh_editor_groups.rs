//! Section rendering for the mesh editor window (see `mesh_editor_overlay`).
//!
//! Extracted so the window file stays small. The layout follows the same "3D
//! Data Editor" presentation dental CAD software uses: calm section captions
//! with a thin hairline instead of shouting bold headers, one uniform
//! icon-cell grid, an unmistakable lit state for the mode toggles, and an
//! OK/Cancel-style commit bar with the primary `Done` pinned bottom-right.
//! Presentation only — each cell emits exactly one [`MeshEditorAction`].

use eframe::egui;

use super::{EditorTab, MeshEditorAction, MeshEditorPanelState};
use crate::icons::AppIcon;
use crate::mesh_editor_icons::{self, CELL_ROUNDING};
use crate::sculpt_tool::{
    SculptToolKind, SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN, SCULPT_SIZE_MAX, SCULPT_SIZE_MIN,
};
use crate::ui_theme;

/// Height of the tab strip / its pills.
const TAB_H: f32 = 28.0;

/// Height of one tool cell: a glyph over a small caption (the same toolbar
/// button style dental CAD software uses). The text commit buttons share the
/// height so the bottom row aligns. Trimmed to keep the palette compact
/// while the glyphs stay legible.
pub(super) const ROW_H: f32 = 46.0;

/// The Sculpt / Mesh Editing tab strip plus the window close button. Doubles as
/// the window's top bar (the native title bar is off).
pub(super) fn tab_strip(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    let gap = 4.0;
    let close_w = 24.0;
    let tab_w = ((ui.available_width() - close_w - gap * 2.0) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if tab_pill(
            ui,
            &locale.tr("meshedit-tab-edit"),
            tab_w,
            state.active_tab == EditorTab::EditMesh,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::SwitchTab(EditorTab::EditMesh));
        }
        if tab_pill(
            ui,
            &locale.tr("meshedit-tab-sculpt"),
            tab_w,
            state.active_tab == EditorTab::Sculpt,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::SwitchTab(EditorTab::Sculpt));
        }
        if close_cross(ui, close_w, locale).clicked() {
            action = Some(MeshEditorAction::Cancel);
        }
    });
    action
}

/// One rounded tab pill: accent-filled when active, a faint accent wash on hover.
fn tab_pill(ui: &mut egui::Ui, label: &str, width: f32, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, TAB_H), egui::Sense::click());
    let fill = if active {
        ui_theme::accent()
    } else if response.hovered() {
        ui_theme::accent().gamma_multiply(0.16)
    } else {
        egui::Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, TAB_H * 0.5, fill);
    let text = if active {
        ui_theme::on_accent()
    } else {
        ui_theme::text_weak()
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        text,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            TAB_H * 0.5,
            egui::Stroke::new(1.5_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    response
}

/// The window close cross cancels the session. Applying edits is explicit via
/// `Done`; closing a native-looking editor must never silently commit changes.
fn close_cross(
    ui: &mut egui::Ui,
    size: f32,
    locale: &crate::i18n::LocaleManager,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, TAB_H), egui::Sense::click());
    crate::icons::paint(
        ui.painter(),
        rect.shrink(3.0),
        AppIcon::Close,
        if response.hovered() {
            ui_theme::text()
        } else {
            ui_theme::text_weak()
        },
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            CELL_ROUNDING,
            egui::Stroke::new(1.5_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    let label = locale.tr("meshedit-cancel-session");
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    response.on_hover_text(label)
}

/// Selection mode (lasso + surface/through radio pair) and the dental CAD
/// All / None / Invert commands.
pub(super) fn selection(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, locale, "meshedit-section-selection");
    // Surface / Through refine Lasso and Marquee, but not Object (a whole
    // connected component is picked regardless of facing), so they grey out
    // while Object pick is armed.
    let depth_enabled = enabled && !state.object_mode;
    row(ui, 4, |ui, width| {
        if icon(
            ui,
            width,
            AppIcon::Lasso,
            &locale.tr("meshedit-cell-lasso"),
            &locale.tr("meshedit-cell-lasso-hint"),
            enabled,
            state.lasso_armed,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ToggleLasso);
        }
        // Object pick: click one whole object of a multi-object STL. Mutually
        // exclusive with Lasso; both fall back to the marquee when toggled off.
        if icon(
            ui,
            width,
            AppIcon::Object,
            &locale.tr("meshedit-cell-object"),
            &locale.tr("meshedit-cell-object-hint"),
            enabled,
            state.object_mode,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ToggleObject);
        }
        // Surface / Through are a radio pair: clicking the inactive one flips
        // the mode, clicking the active one is a no-op.
        if icon(
            ui,
            width,
            AppIcon::SurfaceMode,
            &locale.tr("meshedit-cell-surface"),
            &locale.tr("meshedit-cell-surface-hint"),
            depth_enabled,
            !state.through_mesh,
        )
        .clicked()
            && state.through_mesh
        {
            action = Some(MeshEditorAction::ToggleThroughMesh);
        }
        if icon(
            ui,
            width,
            AppIcon::ThroughMode,
            &locale.tr("meshedit-cell-through"),
            &locale.tr("meshedit-cell-through-hint"),
            depth_enabled,
            state.through_mesh,
        )
        .clicked()
            && !state.through_mesh
        {
            action = Some(MeshEditorAction::ToggleThroughMesh);
        }
    });
    action.or(selection_bulk(ui, enabled, locale))
}

/// The dental CAD All / None / Invert bulk-marking row. Split out of
/// [`selection`] so that function stays within the line budget after the
/// Object cell landed.
fn selection_bulk(
    ui: &mut egui::Ui,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    row(ui, 3, |ui, width| {
        if icon(
            ui,
            width,
            AppIcon::SelectAll,
            &locale.tr("meshedit-cell-all"),
            &locale.tr("meshedit-cell-all-hint"),
            enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::SelectAll);
        }
        if icon(
            ui,
            width,
            AppIcon::SelectNone,
            &locale.tr("meshedit-cell-none"),
            &locale.tr("meshedit-cell-none-hint"),
            enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ClearSelection);
        }
        if icon(
            ui,
            width,
            AppIcon::SelectInvert,
            &locale.tr("meshedit-cell-invert"),
            &locale.tr("meshedit-cell-invert-hint"),
            enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::InvertSelection);
        }
    });
    action
}

/// Destructive, selection-scoped operations (Delete / Crop / Cut / Divide,
/// matching the dental CAD convention). All are disabled until something is
/// marked.
pub(super) fn edit_selection(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    let selection_enabled = enabled && state.selected_face_count > 0;
    section(ui, locale, "meshedit-section-edit-selection");
    row(ui, 4, |ui, width| {
        if icon(
            ui,
            width,
            AppIcon::Delete,
            &locale.tr("meshedit-cell-delete"),
            &locale.tr("meshedit-cell-delete-hint"),
            selection_enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Delete);
        }
        if icon(
            ui,
            width,
            AppIcon::Keep,
            &locale.tr("meshedit-cell-crop"),
            &locale.tr("meshedit-cell-crop-hint"),
            selection_enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Crop);
        }
        if icon(
            ui,
            width,
            AppIcon::Cut,
            &locale.tr("meshedit-cell-cut"),
            &locale.tr("meshedit-cell-cut-hint"),
            selection_enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Cut);
        }
        if icon(
            ui,
            width,
            AppIcon::Separate,
            &locale.tr("meshedit-cell-separate"),
            &locale.tr("meshedit-cell-separate-hint"),
            selection_enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Separate);
        }
    });
    action
}

/// Repair safe interior holes across the visible scene. With marked faces the
/// repair is scoped to those marks; without marks every visible layer is
/// considered. The optional perimeter restraint is deliberately off by
/// default, because outer scan borders are protected by the kernel already.
pub(super) fn close_holes(
    ui: &mut egui::Ui,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, locale, "meshedit-section-close-holes");
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let cell_width = 92.0_f32.min((ui.available_width() - spacing).max(56.0));
        if icon(
            ui,
            cell_width,
            AppIcon::CloseHoles,
            &locale.tr("meshedit-cell-close-holes"),
            &locale.tr("meshedit-cell-close-holes-hint"),
            enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::CloseHoles);
        }
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ROW_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| close_holes_limit_control(ui, enabled, locale),
        );
    });
    ui.add_space(2.0);
    action
}

/// Interactive freeform sculpting (matching the dental CAD Freeforming
/// workflow, applied to scans): two
/// tools only — an Add/Remove clay knife (Shift carves) and a Smooth relaxer
/// (Shift forces it) — plus the shared Size and Strength sliders. Arming a tool
/// takes the primary drag away from the selection gestures until toggled off.
pub(super) fn sculpt(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, locale, "meshedit-section-sculpt");
    row(ui, 2, |ui, width| {
        if icon(
            ui,
            width,
            AppIcon::SculptAdd,
            &locale.tr("meshedit-sculpt-addremove"),
            &locale.tr("meshedit-sculpt-addremove-hint"),
            enabled,
            state.sculpt_armed == Some(SculptToolKind::AddRemove),
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ToggleSculpt(SculptToolKind::AddRemove));
        }
        if icon(
            ui,
            width,
            AppIcon::Smooth,
            &locale.tr("meshedit-sculpt-smooth"),
            &locale.tr("meshedit-sculpt-smooth-hint"),
            enabled,
            state.sculpt_armed == Some(SculptToolKind::Smooth),
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ToggleSculpt(SculptToolKind::Smooth));
        }
    });
    sculpt_settings_row(ui, enabled, locale);
    action
}

/// Size/intensity sliders for the sculpt tools. Both live in egui memory (like
/// the Close Holes limit) so they hold while the editor is open, and both are
/// abstract 0..100 feel sliders — not millimeters — per the operator's request.
fn sculpt_settings_row(ui: &mut egui::Ui, enabled: bool, locale: &crate::i18n::LocaleManager) {
    let ctx = ui.ctx().clone();
    let mut size = super::sculpt_size(&ctx);
    let mut intensity = super::sculpt_intensity(&ctx);
    sculpt_slider_row(
        ui,
        enabled,
        SculptSliderControl {
            label: &locale.tr("meshedit-slider-size"),
            value: &mut size,
            range: SCULPT_SIZE_MIN..=SCULPT_SIZE_MAX,
            tooltip: &locale.tr("meshedit-slider-size-hint"),
        },
    );
    ui.add_space(2.0);
    sculpt_slider_row(
        ui,
        enabled,
        SculptSliderControl {
            label: &locale.tr("meshedit-slider-force"),
            value: &mut intensity,
            range: SCULPT_INTENSITY_MIN..=SCULPT_INTENSITY_MAX,
            tooltip: &locale.tr("meshedit-slider-force-hint"),
        },
    );
    super::set_sculpt_size(&ctx, size);
    super::set_sculpt_intensity(&ctx, intensity);
    ui.add_space(2.0);
}

struct SculptSliderControl<'a> {
    label: &'a str,
    value: &'a mut f32,
    range: std::ops::RangeInclusive<f32>,
    tooltip: &'a str,
}

fn sculpt_slider_row(ui: &mut egui::Ui, enabled: bool, control: SculptSliderControl<'_>) {
    let row_height = ui.spacing().interact_size.y;
    // The caption rides its own line. The rail owns the full content width so
    // the operator gets a stable, easy-to-grab target in the compact panel.
    let slider_width = sculpt_slider_width(ui.available_width());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(control.label).size(11.0).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{:.0}", *control.value))
                    .size(11.0)
                    .weak(),
            );
        });
    });
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                [slider_width, row_height],
                egui::Slider::new(control.value, control.range)
                    .show_value(false)
                    .trailing_fill(true),
            )
        })
        .inner;
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            CELL_ROUNDING,
            egui::Stroke::new(1.25_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::slider(enabled, f64::from(*control.value), control.label)
    });
    response.on_hover_text(control.tooltip);
}

/// Reserve the full available content width for the slider rail. Kept pure so
/// the compact control geometry is explicit and regression-testable.
fn sculpt_slider_width(available_width: f32) -> f32 {
    available_width.max(0.0)
}

fn close_holes_limit_control(
    ui: &mut egui::Ui,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    let id = super::close_holes_limit_id();
    let mut armed = super::close_holes_limit_enabled(ui.ctx());
    let mut limit = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(super::CLOSE_HOLES_LIMIT_DEFAULT_MM);
    let checkbox = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut armed));
    if checkbox.has_focus() {
        ui.painter().rect_stroke(
            checkbox.rect.shrink(1.0),
            CELL_ROUNDING,
            egui::Stroke::new(1.25_f32, ui_theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    checkbox.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            enabled,
            armed,
            locale.tr("meshedit-limit-label"),
        )
    });
    checkbox.on_hover_text(locale.tr("meshedit-limit-checkbox-hint"));
    ui.label(
        egui::RichText::new(locale.tr("meshedit-limit-label"))
            .size(11.0)
            .weak(),
    );
    let drag_value = ui.add_enabled(
        enabled && armed,
        egui::DragValue::new(&mut limit)
            .range(super::CLOSE_HOLES_LIMIT_MIN_MM..=super::CLOSE_HOLES_LIMIT_MAX_MM)
            .speed(0.5)
            .suffix(" mm"),
    );
    drag_value.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::DragValue,
            enabled && armed,
            locale.tr("meshedit-limit-label"),
        )
    });
    drag_value.on_hover_text(locale.tr("meshedit-limit-drag-hint"));
    super::set_close_holes_limit_enabled(ui.ctx(), armed);
    ui.ctx().data_mut(|data| data.insert_temp(id, limit));
}

/// Draw a tool-panel header.
pub(super) fn header(ui: &mut egui::Ui, title: &str, icon: AppIcon) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), rect, icon, ui_theme::accent());
        ui.label(egui::RichText::new(title).strong().size(14.0));
    });
    ui.add_space(2.0);
}

fn section(ui: &mut egui::Ui, locale: &crate::i18n::LocaleManager, title_key: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label_color = ui.visuals().weak_text_color();
        ui.label(
            egui::RichText::new(locale.tr(title_key))
                .size(10.0)
                .color(label_color),
        );
        let avail = ui.available_width();
        if avail > 6.0 {
            let hairline = ui.visuals().widgets.noninteractive.bg_stroke;
            let (line_rect, _) =
                ui.allocate_exact_size(egui::vec2(avail, 1.0), egui::Sense::hover());
            ui.painter()
                .hline(line_rect.x_range(), line_rect.center().y, hairline);
        }
    });
    ui.add_space(2.0);
}

/// Split a row of `available` width into `count` equal columns separated by
/// `spacing`, never collapsing below a legible minimum. Pure so the grid
/// geometry is unit-testable.
#[allow(clippy::cast_precision_loss)]
fn cell_width(available: f32, count: usize, spacing: f32) -> f32 {
    // Row counts are tiny (2-4 controls); the cast is exact.
    let denominator = count.max(1) as f32;
    ((available - spacing * (denominator - 1.0)) / denominator).max(24.0)
}

/// Lay out `count` equal-width controls on one row. The closure receives the
/// per-control width so every group renders as an aligned grid.
fn row(ui: &mut egui::Ui, count: usize, add_contents: impl FnOnce(&mut egui::Ui, f32)) {
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let width = cell_width(ui.available_width(), count, spacing);
        add_contents(ui, width);
    });
    ui.add_space(2.0);
}

/// One icon tool cell of the given width and the shared row height.
// Thin forwarder to `icon_button`; the arg list mirrors it deliberately.
#[allow(clippy::too_many_arguments)]
pub(super) fn icon(
    ui: &mut egui::Ui,
    width: f32,
    glyph: AppIcon,
    label: &str,
    tooltip: &str,
    enabled: bool,
    active: bool,
) -> egui::Response {
    mesh_editor_icons::icon_button(
        ui,
        egui::vec2(width, ROW_H),
        glyph,
        label,
        tooltip,
        enabled,
        active,
    )
}

/// A text-only session button sized to match the icon rows. `primary` renders
/// the accented commit style (Done): a solid accent fill with light text so it
/// is the one obvious action, mirroring the dental CAD OK button.
pub(super) fn tall_text_button(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    let button = if primary {
        egui::Button::new(
            egui::RichText::new(label)
                .color(ui_theme::on_accent())
                .strong(),
        )
        .fill(ui_theme::accent())
        .corner_radius(CELL_ROUNDING)
    } else {
        egui::Button::new(label).corner_radius(CELL_ROUNDING)
    }
    .min_size(egui::vec2(width, ROW_H));
    ui.add_enabled(enabled, button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_width_splits_a_row_into_equal_columns() {
        let width = cell_width(212.0, 4, 6.0);
        // (212 - 3*6) / 4 = 48.5.
        assert!((width - 48.5).abs() < 0.01, "unexpected cell width {width}");
        // Four cells plus three gaps exactly refill the row (aligned grid).
        assert!((4.0 * width + 3.0 * 6.0 - 212.0).abs() < 0.01);
        // The width has to answer to all three inputs. Calling the same pure
        // function twice with the same arguments says nothing -- a body that
        // ignored the count and the gap passed that.
        assert!(
            cell_width(212.0, 3, 6.0) > cell_width(212.0, 4, 6.0),
            "fewer controls in the same row means wider cells"
        );
        assert!(
            cell_width(212.0, 4, 6.0) > cell_width(212.0, 4, 12.0),
            "a wider gap leaves less for each cell"
        );
        assert!(
            cell_width(300.0, 4, 6.0) > cell_width(212.0, 4, 6.0),
            "a wider row means wider cells"
        );
    }

    #[test]
    fn cell_width_never_collapses_below_a_legible_minimum() {
        assert!(cell_width(10.0, 4, 6.0) >= 24.0);
    }

    #[test]
    fn sculpt_slider_uses_the_full_panel_width() {
        assert!((sculpt_slider_width(212.0) - 212.0).abs() < f32::EPSILON);
        assert!(sculpt_slider_width(0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn selection_sections_follow_the_workflow_order() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("mesh_editor_groups.rs"))
                .replace("\r\n", "\n");
        let production = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);
        // The `section(ui, locale, key)` calls, not the bare titles: the tab
        // strip also spells "Sculpt"/"Mesh Editing" and would collide with
        // a bare search.
        let order = [
            "section(ui, locale, \"meshedit-section-selection\")",
            "section(ui, locale, \"meshedit-section-edit-selection\")",
            "section(ui, locale, \"meshedit-section-close-holes\")",
            "section(ui, locale, \"meshedit-section-sculpt\")",
        ];
        let mut last = 0;
        for title in order {
            let at = production.find(title).unwrap_or(usize::MAX);
            assert!(at != usize::MAX, "section {title} missing");
            assert!(at > last, "section {title} out of workflow order");
            last = at;
        }
    }

    #[test]
    fn every_group_renders_across_states_without_panicking() {
        let states = [
            MeshEditorPanelState::default(),
            MeshEditorPanelState {
                selected_face_count: 5,
                can_undo: true,
                can_redo: true,
                lasso_armed: true,
                object_mode: false,
                through_mesh: true,
                sculpt_armed: None,
                dirty: true,
                busy: false,
                sculpt_pending: false,
                active_tab: EditorTab::Sculpt,
            },
            MeshEditorPanelState {
                object_mode: true,
                ..Default::default()
            },
            MeshEditorPanelState {
                sculpt_armed: Some(SculptToolKind::Smooth),
                ..Default::default()
            },
            MeshEditorPanelState {
                busy: true,
                ..Default::default()
            },
        ];
        for state in states {
            let enabled = !state.busy && !state.sculpt_pending;
            let locale = crate::i18n::LocaleManager::for_tests();
            egui::__run_test_ui(|ui| {
                ui.set_width(212.0);
                let _ = tab_strip(ui, &state, &locale);
                let _ = selection(ui, &state, enabled, &locale);
                let _ = edit_selection(ui, &state, enabled, &locale);
                let _ = close_holes(ui, enabled, &locale);
                let _ = sculpt(ui, &state, enabled, &locale);
                super::super::session_bar::status(ui, &state, &locale);
                let _ = super::super::session_bar::session(ui, &state, !state.busy, &locale);
            });
        }
    }

    #[test]
    fn compact_controls_expose_labels_to_keyboard_and_accessibility_users() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("mesh_editor_groups.rs"));
        assert!(source.contains("WidgetInfo::slider"));
        assert!(source.contains("WidgetInfo::selected"));
        assert!(source.contains("response.has_focus()"));
    }
}

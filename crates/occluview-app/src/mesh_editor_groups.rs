//! Section rendering for the mesh editor window (see `mesh_editor_overlay`).
//!
//! Extracted so the window file stays small. The layout follows exocad's "3D
//! Data Editor" vibe: calm section captions with a thin hairline instead of
//! shouting bold headers, one uniform icon-cell grid, an unmistakable lit state
//! for the mode toggles, and an OK/Cancel-style commit bar with the primary
//! `Done` pinned bottom-right. Presentation only — each cell emits exactly one
//! [`MeshEditorAction`].

use eframe::egui;

use super::{EditorTab, MeshEditorAction, MeshEditorPanelState};
use crate::mesh_editor_icons::{self, EditorIcon, CELL_ROUNDING};
use crate::sculpt_tool::{
    SculptToolKind, SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN, SCULPT_SIZE_MAX, SCULPT_SIZE_MIN,
};
use crate::ui_theme::{ACCENT, TEXT, TEXT_WEAK};

/// Height of the tab strip / its pills.
const TAB_H: f32 = 28.0;

/// Height of one tool cell: a glyph over a small caption (exocad-style toolbar
/// button). The text commit buttons share the height so the bottom row aligns.
/// Trimmed to keep the palette compact while the glyphs stay legible.
const ROW_H: f32 = 46.0;

/// Text color for the primary `Done` button: high contrast on the accent fill
/// in both the light and dark exocad themes.
const PRIMARY_TEXT: egui::Color32 = egui::Color32::WHITE;

/// The Sculpt / Edit Mesh tab strip plus the window close button. Doubles as
/// the window's top bar (the native title bar is off).
pub(super) fn tab_strip(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
) -> Option<MeshEditorAction> {
    let mut action = None;
    let gap = 4.0;
    let close_w = 24.0;
    let tab_w = ((ui.available_width() - close_w - gap * 2.0) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if tab_pill(
            ui,
            "Edit Mesh",
            tab_w,
            state.active_tab == EditorTab::EditMesh,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::SwitchTab(EditorTab::EditMesh));
        }
        if tab_pill(ui, "Sculpt", tab_w, state.active_tab == EditorTab::Sculpt).clicked() {
            action = Some(MeshEditorAction::SwitchTab(EditorTab::Sculpt));
        }
        if close_cross(ui, close_w).clicked() {
            action = Some(MeshEditorAction::Done);
        }
    });
    action
}

/// One rounded tab pill: accent-filled when active, a faint accent wash on hover.
fn tab_pill(ui: &mut egui::Ui, label: &str, width: f32, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, TAB_H), egui::Sense::click());
    let fill = if active {
        ACCENT
    } else if response.hovered() {
        ACCENT.gamma_multiply(0.16)
    } else {
        egui::Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::Rounding::same(TAB_H * 0.5), fill);
    let text = if active {
        egui::Color32::WHITE
    } else {
        TEXT_WEAK
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        text,
    );
    response
}

/// The window close cross (commits the session, like the old native ×).
fn close_cross(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, TAB_H), egui::Sense::click());
    let color = if response.hovered() { TEXT } else { TEXT_WEAK };
    let arm = 4.0;
    let center = rect.center();
    let stroke = egui::Stroke::new(1.4, color);
    ui.painter().line_segment(
        [
            center + egui::vec2(-arm, -arm),
            center + egui::vec2(arm, arm),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(arm, -arm),
            center + egui::vec2(-arm, arm),
        ],
        stroke,
    );
    response
}

/// Selection mode (lasso + surface/through radio pair) and the exocad
/// All / None / Invert commands.
pub(super) fn selection(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, "Selection");
    // Surface / Through refine Lasso and Marquee, but not Object (a whole
    // connected component is picked regardless of facing), so they grey out
    // while Object pick is armed.
    let depth_enabled = enabled && !state.object_mode;
    row(ui, 4, |ui, width| {
        if icon(
            ui,
            width,
            EditorIcon::Lasso,
            "Lasso",
            "Freehand outline: click to place points, double-click to close · Shift unmarks",
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
            EditorIcon::Object,
            "Object",
            "Click a whole object of a multi-part STL to select it · Shift unmarks",
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
            EditorIcon::SurfaceMode,
            "Surface",
            "Mark only the visible front-facing surface",
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
            EditorIcon::ThroughMode,
            "Through",
            "Mark straight through the mesh, including hidden backsides",
            depth_enabled,
            state.through_mesh,
        )
        .clicked()
            && !state.through_mesh
        {
            action = Some(MeshEditorAction::ToggleThroughMesh);
        }
    });
    action.or(selection_bulk(ui, enabled))
}

/// The exocad All / None / Invert bulk-marking row. Split out of [`selection`]
/// so that function stays within the line budget after the Object cell landed.
fn selection_bulk(ui: &mut egui::Ui, enabled: bool) -> Option<MeshEditorAction> {
    let mut action = None;
    row(ui, 3, |ui, width| {
        if icon(
            ui,
            width,
            EditorIcon::SelectAll,
            "All",
            "Mark every face (Ctrl+A)",
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
            EditorIcon::SelectNone,
            "None",
            "Clear the marking",
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
            EditorIcon::SelectInvert,
            "Invert",
            "Swap marked and unmarked faces",
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

/// Destructive, selection-scoped operations (exocad Delete / Crop / Cut /
/// Divide). All are disabled until something is marked.
pub(super) fn edit_selection(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
) -> Option<MeshEditorAction> {
    let mut action = None;
    let selection_enabled = enabled && state.selected_face_count > 0;
    section(ui, "Edit selection");
    row(ui, 4, |ui, width| {
        if icon(
            ui,
            width,
            EditorIcon::Delete,
            "Delete",
            "Delete the marked faces",
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
            EditorIcon::Keep,
            "Crop",
            "Keep only the marked area, remove the rest (exocad Crop)",
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
            EditorIcon::Cut,
            "Cut",
            "Move the marked faces to a new mesh — the original stays put (exocad Cut)",
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
            EditorIcon::Separate,
            "Separate",
            "Split the marked region into one mesh per connected part (exocad Divide)",
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
pub(super) fn close_holes(ui: &mut egui::Ui, enabled: bool) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, "Close holes");
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let cell_width = 92.0_f32.min((ui.available_width() - spacing).max(56.0));
        if icon(
            ui,
            cell_width,
            EditorIcon::CloseHoles,
            "Close holes",
            "Close holes only when the surrounding faces are selected. Scan borders stay open.",
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
            |ui| close_holes_limit_control(ui, enabled),
        );
    });
    ui.add_space(2.0);
    action
}

/// Interactive freeform sculpting (exocad Freeforming applied to scans): two
/// tools only — an Add/Remove clay knife (Shift carves) and a Smooth relaxer
/// (Shift forces it) — plus the shared Size and Strength sliders. Arming a tool
/// takes the primary drag away from the selection gestures until toggled off.
pub(super) fn sculpt(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
) -> Option<MeshEditorAction> {
    let mut action = None;
    section(ui, "Sculpt");
    row(ui, 2, |ui, width| {
        if icon(
            ui,
            width,
            EditorIcon::SculptAdd,
            "Add / Remove  [1]",
            "Build material up by dragging on the scan; hold Shift to carve it away. \
             Shift+wheel resizes, Ctrl+wheel changes intensity. Hotkey: 1.",
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
            EditorIcon::Smooth,
            "Smooth  [2]",
            "Relax the surface by dragging on the scan; hold Shift to force maximum smoothing. \
             Shift+wheel resizes, Ctrl+wheel changes intensity. Hotkey: 2.",
            enabled,
            state.sculpt_armed == Some(SculptToolKind::Smooth),
        )
        .clicked()
        {
            action = Some(MeshEditorAction::ToggleSculpt(SculptToolKind::Smooth));
        }
    });
    sculpt_settings_row(ui, enabled);
    action
}

/// Size/intensity sliders for the sculpt tools. Both live in egui memory (like
/// the Close Holes limit) so they hold while the editor is open, and both are
/// abstract 0..100 feel sliders — not millimeters — per the operator's request.
fn sculpt_settings_row(ui: &mut egui::Ui, enabled: bool) {
    let ctx = ui.ctx().clone();
    let mut size = super::sculpt_size(&ctx);
    let mut intensity = super::sculpt_intensity(&ctx);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("size").size(11.0).weak());
        ui.add_enabled(
            enabled,
            egui::Slider::new(&mut size, SCULPT_SIZE_MIN..=SCULPT_SIZE_MAX).show_value(false),
        )
        .on_hover_text("Brush size (Shift + mouse wheel)");
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("force").size(11.0).weak());
        ui.add_enabled(
            enabled,
            egui::Slider::new(&mut intensity, SCULPT_INTENSITY_MIN..=SCULPT_INTENSITY_MAX)
                .show_value(false),
        )
        .on_hover_text("Brush intensity (Ctrl + mouse wheel)");
    });
    super::set_sculpt_size(&ctx, size);
    super::set_sculpt_intensity(&ctx, intensity);
    ui.add_space(2.0);
}

fn close_holes_limit_control(ui: &mut egui::Ui, enabled: bool) {
    let id = super::close_holes_limit_id();
    let mut armed = super::close_holes_limit_enabled(ui.ctx());
    let mut limit = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(super::CLOSE_HOLES_LIMIT_DEFAULT_MM);
    ui.add_enabled(enabled, egui::Checkbox::without_text(&mut armed))
        .on_hover_text("Restrict repair to rims no larger than this perimeter");
    ui.label(egui::RichText::new("limit").size(11.0).weak());
    ui.add_enabled(
        enabled && armed,
        egui::DragValue::new(&mut limit)
            .range(super::CLOSE_HOLES_LIMIT_MIN_MM..=super::CLOSE_HOLES_LIMIT_MAX_MM)
            .speed(0.5)
            .suffix(" mm"),
    )
    .on_hover_text(
        "Off closes every safe hole inside the selected area; the scan border stays open",
    );
    super::set_close_holes_limit_enabled(ui.ctx(), armed);
    ui.ctx().data_mut(|data| data.insert_temp(id, limit));
}

/// One dim line of operator context: the pending-edits marker (only when it has
/// something to say) and the interaction hint for the active selection mode. The
/// raw selected-face count is intentionally NOT shown — it is noise that just ate
/// an info line (owner request).
pub(super) fn status(ui: &mut egui::Ui, state: &MeshEditorPanelState) {
    ui.add_space(3.0);
    if state.busy {
        ui.spinner();
    } else if state.dirty {
        ui.label(
            egui::RichText::new("● unsaved")
                .color(egui::Color32::from_rgb(0xb5, 0x6a, 0x00))
                .size(11.0),
        )
        .on_hover_text("Uncommitted edits: Done to apply, Cancel to revert");
    }
    let hint = if state.sculpt_armed.is_some() {
        "Drag on the surface to sculpt · RMB orbits"
    } else if state.object_mode {
        "Click an object to select it whole · Shift unmarks"
    } else if state.lasso_armed {
        "Click to outline · double-click closes · Shift unmarks"
    } else {
        "Drag a box to mark · Shift to unmark · Del deletes"
    };
    ui.label(egui::RichText::new(hint).weak().size(10.0));
}

/// History and session boundary, laid out as an exocad OK/Cancel bar: Undo/Redo
/// as light history cells on the left, then `Cancel` and the accented `Done`
/// pinned bottom-right. Done confirms and dismisses; Cancel reverts to baseline.
pub(super) fn session(
    ui: &mut egui::Ui,
    state: &MeshEditorPanelState,
    enabled: bool,
) -> Option<MeshEditorAction> {
    let mut action = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        // History cluster (left).
        let history_w = 42.0;
        if icon(
            ui,
            history_w,
            EditorIcon::Undo,
            "Undo",
            "Undo the last mesh edit (Ctrl+Z)",
            state.can_undo && enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Undo);
        }
        if icon(
            ui,
            history_w,
            EditorIcon::Redo,
            "Redo",
            "Redo the undone mesh edit (Ctrl+Y)",
            state.can_redo && enabled,
            false,
        )
        .clicked()
        {
            action = Some(MeshEditorAction::Redo);
        }
        // Commit cluster (right): Cancel + Done fill the remaining width, so
        // Done lands flush against the right edge as the primary action.
        let commit_w = ((ui.available_width() - spacing) / 2.0).max(48.0);
        if tall_text_button(ui, commit_w, "Cancel", enabled, false)
            .on_hover_text("Discard every edit from this session")
            .clicked()
        {
            action = Some(MeshEditorAction::Cancel);
        }
        if tall_text_button(ui, commit_w, "Done", enabled, true)
            .on_hover_text("Apply the edits and close the editor")
            .clicked()
        {
            action = Some(MeshEditorAction::Done);
        }
    });
    action
}

/// A calm section caption: a small, muted label followed by a thin hairline
/// filling the row. Replaces the old bold header + full-width separator with a
/// single quiet cue (exocad tool windows keep almost no section chrome).
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label_color = ui.visuals().weak_text_color();
        ui.label(egui::RichText::new(title).size(9.5).color(label_color));
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
fn icon(
    ui: &mut egui::Ui,
    width: f32,
    glyph: EditorIcon,
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
/// is the one obvious action, mirroring exocad's OK button.
fn tall_text_button(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    let button = if primary {
        egui::Button::new(egui::RichText::new(label).color(PRIMARY_TEXT).strong())
            .fill(ACCENT)
            .rounding(CELL_ROUNDING)
    } else {
        egui::Button::new(label).rounding(CELL_ROUNDING)
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
        // Uniform and deterministic: same inputs give a bit-identical width.
        assert_eq!(
            cell_width(212.0, 3, 6.0).to_bits(),
            cell_width(212.0, 3, 6.0).to_bits()
        );
    }

    #[test]
    fn cell_width_never_collapses_below_a_legible_minimum() {
        assert!(cell_width(10.0, 4, 6.0) >= 24.0);
    }

    #[test]
    fn selection_sections_follow_the_workflow_order() {
        let source = include_str!("mesh_editor_groups.rs").replace("\r\n", "\n");
        let production = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);
        // The `section(ui, ...)` calls, not the bare titles: the tab strip also
        // spells "Sculpt"/"Edit Mesh" and would collide with a bare search.
        let order = [
            "section(ui, \"Selection\")",
            "section(ui, \"Edit selection\")",
            "section(ui, \"Close holes\")",
            "section(ui, \"Sculpt\")",
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
            let enabled = !state.busy;
            egui::__run_test_ui(|ui| {
                ui.set_width(212.0);
                let _ = tab_strip(ui, &state);
                let _ = selection(ui, &state, enabled);
                let _ = edit_selection(ui, &state, enabled);
                let _ = close_holes(ui, enabled);
                let _ = sculpt(ui, &state, enabled);
                status(ui, &state);
                let _ = session(ui, &state, enabled);
            });
        }
    }
}

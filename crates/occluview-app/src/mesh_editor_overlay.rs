//! The mesh editor tool window (the dental CAD "3D Data Editor" workflow).
//!
//! A movable `egui::Window` with a custom top bar of two tabs — Edit Mesh
//! (selection / repair) and Sculpt (the brushes) — over a shared status +
//! commit bar. Presentation only: every button maps to one [`MeshEditorAction`]
//! the viewport applies.
//!
//! The per-section rendering lives in the sibling [`groups`] module (declared
//! below with an explicit path); this file owns only the window shell and the
//! action vocabulary.

use eframe::egui;

use crate::app::workspace::id::SceneKey;
use crate::ui::icons::AppIcon;

use crate::sculpt_tool::{SculptTip, SculptToolKind};

#[path = "mesh_editor_groups.rs"]
mod groups;
#[path = "mesh_editor_session.rs"]
mod session_bar;

/// The two tabs of the editor window: selection/repair tools, or the sculpt
/// brushes. Exactly one is shown at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum EditorTab {
    #[default]
    EditMesh,
    Sculpt,
}

/// Actions the mesh editor window can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshEditorAction {
    /// Switch the active tab.
    SwitchTab(EditorTab),
    SelectAll,
    InvertSelection,
    ClearSelection,
    /// Arm/disarm the freehand lasso capture (the dental CAD "Edit Mesh" lasso).
    ToggleLasso,
    /// Arm/disarm Object pick: click one whole object of a multi-object STL.
    ToggleObject,
    /// Switch between surface (front-facing) and through-mesh selection.
    ToggleThroughMesh,
    /// Arm/disarm one interactive sculpt tool (the dental CAD Freeforming
    /// workflow: the Add/Remove clay knife or the Smooth relaxer), dragged on
    /// the surface.
    ToggleSculpt(SculptToolKind),
    /// Confirm the edit session: keep edits, close the window.
    Done,
    /// Revert the whole edit session to the captured baseline.
    Cancel,
    Delete,
    Crop,
    Cut,
    Separate,
    CloseHoles,
    Undo,
    Redo,
}

/// Snapshot of the editor state the window renders from. Kept as a struct so
/// the viewport reads each field once (borrow discipline) and the signature
/// stays stable as the window gains richer state. Each bool is an independent
/// flag (tool mode + session phase), not a bitfield of one concept.
#[expect(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshEditorPanelState {
    /// Total selected faces across visible editable layers.
    pub(crate) selected_face_count: usize,
    /// Whether an edit operation can be undone right now.
    pub(crate) can_undo: bool,
    /// Whether an undone edit operation can be re-applied right now.
    pub(crate) can_redo: bool,
    /// Whether the freehand lasso owns the primary viewport drag.
    pub(crate) lasso_armed: bool,
    /// Whether Object pick is armed (a click selects a whole component).
    pub(crate) object_mode: bool,
    /// Lasso mode: false = surface/front-facing, true = through-mesh.
    pub(crate) through_mesh: bool,
    /// The armed sculpt tool, if any (owns the primary drag when set).
    pub(crate) sculpt_armed: Option<SculptToolKind>,
    /// Whether the session carries uncommitted edits (Done is meaningful).
    pub(crate) dirty: bool,
    /// Whether a mesh operation is running (all mutating buttons disabled).
    pub(crate) busy: bool,
    /// Whether Sculpt has an in-flight drag or worker output. Structural mesh
    /// operations must wait for it, while Done/Cancel remain available.
    pub(crate) sculpt_pending: bool,
    /// Which tab is showing.
    pub(crate) active_tab: EditorTab,
}

const WINDOW_WIDTH_MIN: f32 = 200.0;
const WINDOW_WIDTH_MAX: f32 = 320.0;

fn window_width(viewport: egui::Rect) -> f32 {
    let max_width = (viewport.width() - 28.0).max(180.0);
    (viewport.width() * 0.22)
        .clamp(WINDOW_WIDTH_MIN, WINDOW_WIDTH_MAX)
        .min(max_width)
}

fn default_pos(viewport: egui::Rect) -> egui::Pos2 {
    let width = window_width(viewport);
    let estimated_height = 380.0;
    let x = viewport.min.x + 16.0;
    let y = (viewport.max.y - estimated_height - 16.0).max(viewport.min.y + 16.0);
    egui::pos2(x.min(viewport.max.x - width - 16.0), y)
}

/// Default and bounds for the optional Close Holes rim-perimeter restraint.
/// It is off by default: the kernel preserves scan borders and repairs every
/// safe interior hole, matching the normal dental workflow.
pub(super) const CLOSE_HOLES_LIMIT_DEFAULT_MM: f32 = 15.0;
pub(super) const CLOSE_HOLES_LIMIT_MIN_MM: f32 = 1.0;
pub(super) const CLOSE_HOLES_LIMIT_MAX_MM: f32 = 100.0;

fn scoped_id(label: &'static str, scene_key: SceneKey) -> egui::Id {
    egui::Id::new((label, scene_key))
}

pub(super) fn close_holes_limit_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_close_holes_limit_mm", scene_key)
}

fn close_holes_limit_enabled_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_close_holes_limit_enabled", scene_key)
}

/// Optional maximum rim perimeter for Close Holes. The value lives in egui
/// memory so it remains stable while the editor is open without becoming a
/// global application preference.
pub(crate) fn close_holes_limit_mm(ctx: &egui::Context, scene_key: SceneKey) -> Option<f32> {
    let enabled = ctx
        .data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id(scene_key)))
        .unwrap_or(false);
    enabled.then(|| {
        ctx.data(|data| data.get_temp::<f32>(close_holes_limit_id(scene_key)))
            .unwrap_or(CLOSE_HOLES_LIMIT_DEFAULT_MM)
    })
}

pub(super) fn set_close_holes_limit_enabled(
    ctx: &egui::Context,
    scene_key: SceneKey,
    enabled: bool,
) {
    ctx.data_mut(|data| data.insert_temp(close_holes_limit_enabled_id(scene_key), enabled));
}

pub(super) fn close_holes_limit_enabled(ctx: &egui::Context, scene_key: SceneKey) -> bool {
    ctx.data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id(scene_key)))
        .unwrap_or(false)
}

fn sculpt_tip_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_sculpt_tip", scene_key)
}

/// The brush tip the Sculpt panel is offering.
pub(crate) fn sculpt_tip(ctx: &egui::Context, scene_key: SceneKey) -> SculptTip {
    ctx.data(|data| data.get_temp::<SculptTip>(scoped_id("occluview_sculpt_tip", scene_key)))
        .unwrap_or_default()
}

pub(crate) fn set_sculpt_tip(ctx: &egui::Context, scene_key: SceneKey, tip: SculptTip) {
    ctx.data_mut(|data| data.insert_temp(sculpt_tip_id(scene_key), tip));
}

fn sculpt_radius_share_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_sculpt_radius_share", scene_key)
}

fn sculpt_strength_id(scene_key: SceneKey, kind: SculptToolKind) -> egui::Id {
    egui::Id::new(("occluview_sculpt_strength", scene_key, kind))
}

fn radius_share(tip: SculptTip, radius_mm: f32) -> f32 {
    let (min, max) = tip.radius_range_mm();
    ((radius_mm.clamp(min, max) - min) / (max - min)).clamp(0.0, 1.0)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "Catalog arithmetic uses decimal f64; the UI stores the nearest representable f32 radius."
)]
fn radius_for_share(tip: SculptTip, share: f32) -> f32 {
    let (min, max) = tip.radius_range_mm();
    let catalog_decimal = |value: f32| (f64::from(value) * 1_000_000.0).round() / 1_000_000.0;
    let min = catalog_decimal(min);
    let max = catalog_decimal(max);
    let step = catalog_decimal(SculptTip::radius_step_mm());
    let share = f64::from(share.clamp(0.0, 1.0));
    let raw = min + share * (max - min);
    let steps = ((raw - min) / step).round();
    (min + steps * step).clamp(min, max) as f32
}

/// Normalized size shared by the catalog tips and persisted without converting
/// through another tip's coarser physical step grid.
pub(crate) fn sculpt_radius_share(ctx: &egui::Context, scene_key: SceneKey) -> f32 {
    ctx.data(|data| data.get_temp::<f32>(sculpt_radius_share_id(scene_key)))
        .filter(|share| share.is_finite())
        .map_or_else(
            || radius_share(SculptTip::Ball, SculptTip::Ball.default_radius_mm()),
            |share| share.clamp(0.0, 1.0),
        )
}

pub(crate) fn set_sculpt_radius_share(ctx: &egui::Context, scene_key: SceneKey, share: f32) {
    let share = if share.is_finite() {
        share.clamp(0.0, 1.0)
    } else {
        radius_share(SculptTip::Ball, SculptTip::Ball.default_radius_mm())
    };
    ctx.data_mut(|data| data.insert_temp(sculpt_radius_share_id(scene_key), share));
}

/// The radius is a normalized share of the selected tip's range. Switching
/// tips preserves the operator's size choice rather than reusing millimetres.
pub(crate) fn sculpt_radius_mm(ctx: &egui::Context, scene_key: SceneKey, tip: SculptTip) -> f32 {
    radius_for_share(tip, sculpt_radius_share(ctx, scene_key))
}

pub(crate) fn set_sculpt_radius_mm(
    ctx: &egui::Context,
    scene_key: SceneKey,
    tip: SculptTip,
    radius_mm: f32,
) {
    set_sculpt_radius_share(ctx, scene_key, radius_share(tip, radius_mm));
}

/// Per-mode kernel strength, with the donor's Add and Smooth defaults.
pub(crate) fn sculpt_strength(
    ctx: &egui::Context,
    scene_key: SceneKey,
    kind: SculptToolKind,
) -> f32 {
    let (min, max) = kind.strength_range();
    ctx.data(|data| data.get_temp::<f32>(sculpt_strength_id(scene_key, kind)))
        .unwrap_or_else(|| kind.default_strength())
        .clamp(min, max)
}

pub(crate) fn set_sculpt_strength(
    ctx: &egui::Context,
    scene_key: SceneKey,
    kind: SculptToolKind,
    strength: f32,
) {
    let (min, max) = kind.strength_range();
    ctx.data_mut(|data| {
        data.insert_temp(
            sculpt_strength_id(scene_key, kind),
            strength.clamp(min, max),
        );
    });
}

/// Show the movable mesh editor window; returns the requested action, if any.
pub(crate) fn show(
    ctx: &egui::Context,
    scene_key: SceneKey,
    viewport_rect: egui::Rect,
    state: MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let width = window_width(viewport_rect);
    let mut action = None;
    egui::Window::new(locale.text(crate::i18n::message_id!("meshedit-window-title")))
        .id(scoped_id("occluview_mesh_editor_window", scene_key))
        .default_pos(default_pos(viewport_rect))
        .constrain_to(viewport_rect)
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(width - 24.0);
            ui.set_width(width - 24.0);
            action = window_action(ui, scene_key, state, locale);
        });
    action
}

/// Assemble the window body: the tab strip, then the active tab's tools, then
/// the shared status + commit bar. Every section renders in [`groups`]; this
/// function fixes the shared spacing and chains the optional actions.
fn window_action(
    ui: &mut egui::Ui,
    scene_key: SceneKey,
    state: MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
    // Snappier hover/press for this dense tool palette than the global chrome.
    ui.style_mut().animation_time = 0.05;
    // While a mesh operation or Sculpt write runs, structural operations are
    // disabled. The session bar receives the narrower edit-mode flag below so
    // Done/Cancel can still resolve or abort the pending sculpt.
    let ops_enabled = !state.busy && !state.sculpt_pending;

    groups::header(
        ui,
        &locale.tr(crate::i18n::message_id!("meshedit-header-edit")),
        AppIcon::EditMesh,
    );
    let mut action = groups::tab_strip(ui, &state, locale);
    ui.add_space(4.0);
    match state.active_tab {
        EditorTab::EditMesh => {
            action = action.or(groups::selection(ui, &state, ops_enabled, locale));
            action = action.or(groups::edit_selection(ui, &state, ops_enabled, locale));
            action = action.or(groups::close_holes(ui, scene_key, ops_enabled, locale));
        }
        EditorTab::Sculpt => {
            action = action.or(groups::sculpt(ui, scene_key, &state, ops_enabled, locale));
        }
    }
    session_bar::status(ui, &state, locale);
    action = action.or(session_bar::session(ui, &state, !state.busy, locale));
    action
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::{
        close_holes_limit_id, close_holes_limit_mm, scoped_id, sculpt_radius_mm, sculpt_strength,
        sculpt_tip, set_close_holes_limit_enabled, set_sculpt_radius_mm, set_sculpt_strength,
        set_sculpt_tip, CLOSE_HOLES_LIMIT_DEFAULT_MM,
    };
    use crate::app::workspace::id::SceneKey;
    use crate::sculpt_tool::{SculptTip, SculptToolKind};
    use eframe::egui;

    #[test]
    fn mesh_editor_live_settings_are_isolated_by_scene_lifetime() {
        let ctx = egui::Context::default();
        let first = SceneKey::INITIAL;
        let second = SceneKey::from_raw_for_test(2, 2).expect("nonzero scene identity");

        set_sculpt_radius_mm(&ctx, first, SculptTip::Ball, 1.25);
        set_sculpt_strength(&ctx, first, SculptToolKind::AddRemove, 0.6);
        set_sculpt_tip(&ctx, first, SculptTip::Knife);
        set_close_holes_limit_enabled(&ctx, first, true);
        ctx.data_mut(|data| data.insert_temp(close_holes_limit_id(first), 21.0_f32));

        assert_eq!(sculpt_radius_mm(&ctx, first, SculptTip::Ball), 1.25);
        assert_eq!(sculpt_strength(&ctx, first, SculptToolKind::AddRemove), 0.6);
        assert_eq!(sculpt_tip(&ctx, first), SculptTip::Knife);
        assert_eq!(close_holes_limit_mm(&ctx, first), Some(21.0));

        assert_eq!(
            sculpt_radius_mm(&ctx, second, SculptTip::Ball),
            SculptTip::Ball.default_radius_mm()
        );
        assert_eq!(
            sculpt_strength(&ctx, second, SculptToolKind::AddRemove),
            SculptToolKind::AddRemove.default_strength()
        );
        assert_eq!(sculpt_tip(&ctx, second), SculptTip::default());
        assert_eq!(close_holes_limit_mm(&ctx, second), None);
        set_close_holes_limit_enabled(&ctx, second, true);
        assert_eq!(
            close_holes_limit_mm(&ctx, second),
            Some(CLOSE_HOLES_LIMIT_DEFAULT_MM)
        );
        assert_ne!(
            scoped_id("mesh-editor", first),
            scoped_id("mesh-editor", second)
        );
    }
}

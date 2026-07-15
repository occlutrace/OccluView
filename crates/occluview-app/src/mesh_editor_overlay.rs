//! The mesh editor tool window (exocad "3D Data Editor" workflow).
//!
//! A freely movable `egui::Window` whose tools read top-to-bottom the way a
//! dental operator works: pick a selection mode → adjust the selection →
//! edit that selection (delete / crop / cut / separate) → history → commit.
//! Presentation only: every button maps to one
//! [`MeshEditorAction`] the viewport already knows how to apply.
//!
//! The per-section rendering lives in the sibling [`groups`] module (declared
//! below with an explicit path) so this file stays small and only owns the
//! window shell and the action vocabulary.

use eframe::egui;

#[path = "mesh_editor_groups.rs"]
mod groups;

/// Actions the mesh editor window can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshEditorAction {
    SelectAll,
    InvertSelection,
    ClearSelection,
    /// Arm/disarm the freehand lasso capture (exocad "Edit Mesh" lasso).
    ToggleLasso,
    /// Arm/disarm Object pick: click one whole object of a multi-object STL.
    ToggleObject,
    /// Switch between surface (front-facing) and through-mesh selection.
    ToggleThroughMesh,
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
    /// Whether the session carries uncommitted edits (Done is meaningful).
    pub(crate) dirty: bool,
    /// Whether a mesh operation is running (all mutating buttons disabled).
    pub(crate) busy: bool,
}

/// Overall window width. Trimmed to keep the exocad-style tool compact; the
/// icon grid and the OK/Cancel commit bar are both sized off it.
const WINDOW_WIDTH: f32 = 236.0;

/// Default and bounds for the optional Close Holes rim-perimeter restraint.
/// It is off by default: the kernel preserves scan borders and repairs every
/// safe interior hole, matching the normal dental workflow.
pub(super) const CLOSE_HOLES_LIMIT_DEFAULT_MM: f32 = 15.0;
pub(super) const CLOSE_HOLES_LIMIT_MIN_MM: f32 = 1.0;
pub(super) const CLOSE_HOLES_LIMIT_MAX_MM: f32 = 100.0;

fn close_holes_limit_id() -> egui::Id {
    egui::Id::new("occluview_close_holes_limit_mm")
}

fn close_holes_limit_enabled_id() -> egui::Id {
    egui::Id::new("occluview_close_holes_limit_enabled")
}

/// Optional maximum rim perimeter for Close Holes. The value lives in egui
/// memory so it remains stable while the editor is open without becoming a
/// global application preference.
pub(crate) fn close_holes_limit_mm(ctx: &egui::Context) -> Option<f32> {
    let enabled = ctx
        .data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id()))
        .unwrap_or(false);
    enabled.then(|| {
        ctx.data(|data| data.get_temp::<f32>(close_holes_limit_id()))
            .unwrap_or(CLOSE_HOLES_LIMIT_DEFAULT_MM)
    })
}

pub(super) fn set_close_holes_limit_enabled(ctx: &egui::Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(close_holes_limit_enabled_id(), enabled));
}

pub(super) fn close_holes_limit_enabled(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id()))
        .unwrap_or(false)
}

/// Show the movable mesh editor window; returns the requested action, if any.
pub(crate) fn show(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    state: MeshEditorPanelState,
) -> Option<MeshEditorAction> {
    let default_pos = viewport_rect.right_top() + egui::vec2(-WINDOW_WIDTH - 16.0, 16.0);
    let mut action = None;
    let mut open = true;
    egui::Window::new("Mesh editor")
        .id(egui::Id::new("occluview_mesh_editor_window"))
        .default_pos(default_pos)
        .constrain_to(viewport_rect)
        .resizable(false)
        .collapsible(true)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(WINDOW_WIDTH - 24.0);
            action = window_action(ui, state);
        });
    if !open {
        action = Some(MeshEditorAction::Done);
    }
    action
}

/// Assemble the window body in exocad workflow order. Every section renders in
/// [`groups`]; this function only fixes the shared spacing and chains the
/// optional actions each section may return.
fn window_action(ui: &mut egui::Ui, state: MeshEditorPanelState) -> Option<MeshEditorAction> {
    // A single tight item spacing gives the icon grid even gutters and keeps
    // the whole tool calm; the sections manage their own vertical rhythm.
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
    // Snappier hover/press for this window only (the default fade reads as lag on
    // a dense tool palette); the global chrome keeps its calmer timing.
    ui.style_mut().animation_time = 0.05;
    // While a mesh operation runs, every mutating button is disabled;
    // selection-mode toggles stay available so the operator is never locked
    // out of the viewport chrome.
    let ops_enabled = !state.busy;

    let mut action = None;
    action = action.or(groups::selection(ui, &state, ops_enabled));
    action = action.or(groups::edit_selection(ui, &state, ops_enabled));
    action = action.or(groups::close_holes(ui, ops_enabled));
    groups::status(ui, &state);
    action = action.or(groups::session(ui, &state, ops_enabled));
    action
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_groups_follow_the_exocad_workflow_order() {
        let source = include_str!("mesh_editor_overlay.rs").replace("\r\n", "\n");
        let production = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);
        // The operator's top-down workflow: pick a selection mode and mark →
        // edit that selection. History and commit
        // (Undo/Redo/Cancel/Done) render last, in `groups::session`.
        let order = [
            "groups::selection(",
            "groups::edit_selection(",
            "groups::close_holes(",
            "groups::session(",
        ];
        let mut last = 0;
        for call in order {
            let at = production.find(call).unwrap_or(usize::MAX);
            assert!(at != usize::MAX, "group call {call} missing");
            assert!(at > last, "group {call} out of exocad workflow order");
            last = at;
        }
        assert!(
            production.contains("egui::Window::new"),
            "the editor must be a movable egui window, not a pinned overlay"
        );
    }
}

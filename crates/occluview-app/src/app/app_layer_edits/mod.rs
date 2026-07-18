//! Layer-context mesh-edit orchestration: routes context/panel actions to the
//! whole-mesh, selection-scoped, structural, and undo/redo executors.

mod repair;
mod selection_batch;
mod selection_ops;
mod structural;
#[cfg(test)]
mod structural_tests;
#[cfg(test)]
mod tests;
mod undo_redo;
mod whole_mesh;

pub(super) use undo_redo::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
};

use super::{
    layer_actions, layers_overlay, LayerContextAction, LayerContextApply, LayerContextRequest,
    OccluViewApp, PathBuf, Scene,
};
use occluview_core::SceneMesh;
use repair::apply_layer_repair_action_with_status;
use selection_ops::apply_selected_face_mesh_edit_action_with_status;

#[cfg(test)]
pub(crate) use selection_batch::apply_visible_selected_face_mesh_edit_action;
pub(crate) use selection_batch::apply_visible_selected_face_mesh_edit_action_with_limit;
use undo_redo::apply_layer_mesh_undo_action_with_status;
use whole_mesh::apply_layer_mesh_edit_action_with_status;

#[derive(Clone, Copy)]
struct SelectedFaceEditContext {
    index: usize,
    layer_id: occluview_core::SceneMeshId,
    token: crate::edit_mode::EditSessionToken,
}

pub(super) fn apply_layer_context_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    if app.bridge_split_active() {
        app.status_message = Some("Finish or cancel Bridge split first".to_string());
        return LayerContextApply::default();
    }

    if request.action == LayerContextAction::UndoLastMeshEdit {
        return apply_layer_mesh_undo_action_with_status(app, scene, paths, request);
    }

    if request.action == LayerContextAction::BridgeSplit {
        app.begin_bridge_split_from_layer(scene, request.layer_id);
        return LayerContextApply::default();
    }

    if request.action == LayerContextAction::EditMesh {
        begin_face_selection_with_status(app, scene, paths, request);
        return LayerContextApply::default();
    }

    if matches!(
        request.action,
        LayerContextAction::DeleteSelectedFaces
            | LayerContextAction::CropToSelectedFaces
            | LayerContextAction::CutSelectionToNewLayer
            | LayerContextAction::SeparateSelectedComponents
    ) {
        return apply_selected_face_mesh_edit_action_with_status(app, scene, paths, request);
    }

    if matches!(
        request.action,
        LayerContextAction::CloseHoles
            | LayerContextAction::InvertNormals
            | LayerContextAction::SmoothSelection
    ) {
        return apply_layer_mesh_edit_action_with_status(app, scene, paths, request);
    }

    if request.action == LayerContextAction::RepairMesh {
        return apply_layer_repair_action_with_status(app, scene, paths, request);
    }

    if request.action == LayerContextAction::ExportLayer {
        app.save_layer_export_dialog(scene, paths, request);
        return LayerContextApply::default();
    }

    if request.action != LayerContextAction::Remove {
        return layer_actions::apply_layer_context_action(scene, request);
    }

    let Some((_, removed_label)) = resolve_layer(scene, paths, &request) else {
        return LayerContextApply::default();
    };
    let apply = layer_actions::apply_layer_context_action(scene, request);
    if apply.scene_changed {
        app.status_message = Some(format!("Removed layer: {removed_label}"));
    }
    apply
}

fn begin_face_selection_with_status(
    app: &mut OccluViewApp,
    scene: &Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) {
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request) else {
        return;
    };
    let switching_target = app.edit_mode.selected_layer_id() != Some(entry.id());
    if app.edit_mode.begin_face_selection(entry, scene) {
        if switching_target {
            // A lasso's screen points belong to its previous mesh. Do not let
            // a layer-row context action carry that outline into a new target.
            app.mesh_selection_drag = None;
        }
        app.selection_overlay_dirty = true;
        app.needs_render = true;
        app.status_message = Some(format!("Face selection: {layer_label}"));
    } else {
        app.status_message = Some(format!("Cannot select faces: {layer_label}"));
    }
}

/// Resolve a context-request's layer index+id against the live scene and
/// build its display label — the lookup-check-label sequence shared by the
/// layer-edit executors.
pub(super) fn resolve_layer<'s>(
    scene: &'s Scene,
    paths: &[PathBuf],
    request: &LayerContextRequest,
) -> Option<(&'s SceneMesh, String)> {
    let entry = scene.meshes().get(request.index)?;
    if entry.id() != request.layer_id {
        return None;
    }
    Some((
        entry,
        layers_overlay::layer_label(paths, entry, request.index),
    ))
}

/// Append the "not undoable" note when the last edit's pre-op snapshot was
/// skipped (oversized) — the suffix shared by the mesh-edit status lines.
pub(super) fn with_undoable_note(app: &OccluViewApp, status: String) -> String {
    if app.edit_mode.last_edit_undoable() {
        status
    } else {
        format!("{status} (not undoable: snapshot too large)")
    }
}

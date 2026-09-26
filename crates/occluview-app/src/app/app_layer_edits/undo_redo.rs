//! Undo/redo orchestration shared by the panel button and the viewport
//! keyboard shortcuts (Ctrl+Z / Ctrl+Y): structural scene history first, then
//! single-layer mesh history.

use super::super::{
    layers_overlay, EditModeController, LayerContextAction, LayerContextApply, LayerContextRequest,
    OccluViewApp, PathBuf, Scene,
};
use super::resolve_layer;
use super::structural::structural_scene_apply;
use crate::edit_mode::StructuralHistoryStep;

pub(in crate::app) fn apply_last_mesh_edit_undo_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
) -> LayerContextApply {
    let Some(layer_id) = app.document.edit_mode.undo_layer_id() else {
        return LayerContextApply::default();
    };
    let Some(index) = scene
        .meshes()
        .iter()
        .position(|entry| entry.id() == layer_id)
    else {
        app.ui.status_message = Some(app.ui.locale.tr("undo-nothing"));
        return LayerContextApply::default();
    };
    apply_layer_mesh_undo_action_with_status(
        app,
        scene,
        paths,
        LayerContextRequest {
            index,
            layer_id,
            action: LayerContextAction::UndoLastMeshEdit,
        },
    )
}

/// Re-apply the last undone mesh edit (Ctrl+Y / Ctrl+Shift+Z). Mirrors the
/// undo path: structural (whole-scene) redo first, then single-layer redo.
pub(in crate::app) fn apply_last_mesh_edit_redo_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
) -> LayerContextApply {
    let Some(layer_id) = app.document.edit_mode.redo_layer_id() else {
        return LayerContextApply::default();
    };
    let Some(index) = scene
        .meshes()
        .iter()
        .position(|entry| entry.id() == layer_id)
    else {
        app.ui.status_message = Some(app.ui.locale.tr("redo-nothing"));
        return LayerContextApply::default();
    };
    let Some(current) = scene.meshes().get(index).cloned() else {
        return LayerContextApply::default();
    };
    let layer_label = layers_overlay::layer_label(paths, &current, index, &app.ui.locale);

    match app.document.edit_mode.redo_last_scene_edit(scene, layer_id) {
        StructuralHistoryStep::Restored(restored_scene) => {
            *scene = restored_scene;
            app.document.mark_mesh_edits_unsaved(layer_id);
            app.ui.status_message = Some(
                app.ui
                    .locale
                    .tr_with("redo-redid", &[("layer", &layer_label)]),
            );
            return structural_scene_apply();
        }
        StructuralHistoryStep::SceneChanged => {
            app.ui.status_message = Some(
                app.ui
                    .locale
                    .tr_with("redo-unavailable", &[("layer", &layer_label)]),
            );
            return LayerContextApply::default();
        }
        StructuralHistoryStep::NotAvailable => {}
    }
    let Some(restored) = app.document.edit_mode.redo_last_layer_edit(&current) else {
        return LayerContextApply::default();
    };
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    if entry.id() != restored.id() {
        return LayerContextApply::default();
    }
    entry.mesh = restored.mesh;
    // The restored mesh comes from the cold snapshot the sculpt session keeps,
    // so its bounding box is not cached. `Scene::bbox` is read twice a frame
    // and falls back to walking every vertex -- 2.1 ms a call on a
    // million-vertex layer, indefinitely. Undo is a click, not a frame, so the
    // walk is paid once here instead.
    let _ = entry.mesh.bbox();
    app.document.mark_mesh_edits_unsaved(layer_id);
    app.ui.status_message = Some(
        app.ui
            .locale
            .tr_with("redo-redid", &[("layer", &layer_label)]),
    );
    structural_scene_apply()
}

pub(super) fn apply_layer_mesh_undo_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let Some((_, layer_label)) = resolve_layer(scene, paths, &request, &app.ui.locale) else {
        return LayerContextApply::default();
    };
    // Structural (whole-scene) undo first. It refuses when the scene changed
    // since the snapshot was recorded: a blind restore would drop a layer
    // appended (or resurrect one removed) since.
    match app
        .document
        .edit_mode
        .undo_last_scene_edit(scene, request.layer_id)
    {
        StructuralHistoryStep::Restored(restored) => {
            *scene = restored;
            app.document.mark_mesh_edits_unsaved(request.layer_id);
            app.ui.status_message = Some(
                app.ui
                    .locale
                    .tr_with("undo-undid", &[("layer", &layer_label)]),
            );
            return structural_scene_apply();
        }
        StructuralHistoryStep::SceneChanged => {
            app.ui.status_message = Some(
                app.ui
                    .locale
                    .tr_with("undo-unavailable", &[("layer", &layer_label)]),
            );
            return LayerContextApply::default();
        }
        StructuralHistoryStep::NotAvailable => {}
    }
    // Single-layer undo (id-keyed, so it is append-safe on its own).
    let apply = apply_layer_mesh_undo_action(scene, request, &mut app.document.edit_mode);
    if apply.scene_changed {
        app.document.mark_mesh_edits_unsaved(request.layer_id);
        app.ui.status_message = Some(
            app.ui
                .locale
                .tr_with("undo-undid", &[("layer", &layer_label)]),
        );
    }
    apply
}

pub(super) fn apply_layer_mesh_undo_action(
    scene: &mut Scene,
    request: LayerContextRequest,
    edit_mode: &mut EditModeController,
) -> LayerContextApply {
    let LayerContextRequest {
        index,
        layer_id,
        action,
    } = request;
    if action != LayerContextAction::UndoLastMeshEdit {
        return LayerContextApply::default();
    }
    let Some(current) = scene.meshes().get(index).cloned() else {
        return LayerContextApply::default();
    };
    if current.id() != layer_id {
        return LayerContextApply::default();
    }
    match edit_mode.undo_last_scene_edit(scene, layer_id) {
        StructuralHistoryStep::Restored(restored_scene) => {
            *scene = restored_scene;
            return structural_scene_apply();
        }
        // The caller (the `_with_status` wrapper) reports the refusal; this
        // path only leaves the scene untouched.
        StructuralHistoryStep::SceneChanged => return LayerContextApply::default(),
        StructuralHistoryStep::NotAvailable => {}
    }
    let Some(restored) = edit_mode.undo_last_layer_edit(&current) else {
        return LayerContextApply::default();
    };
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    if entry.id() != restored.id() {
        return LayerContextApply::default();
    }

    entry.mesh = restored.mesh;
    // Cold snapshot again: pay the bbox walk here rather than twice a frame
    // for the rest of the session.
    let _ = entry.mesh.bbox();
    structural_scene_apply()
}

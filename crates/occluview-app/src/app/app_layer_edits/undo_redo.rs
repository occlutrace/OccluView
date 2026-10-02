//! Undo/redo orchestration shared by the panel button and the viewport
//! keyboard shortcuts (Ctrl+Z / Ctrl+Y): structural scene history first, then
//! single-layer mesh history.

use super::super::{
    layers_overlay, EditModeController, LayerContextAction, LayerContextApply, LayerContextRequest,
    PathBuf, Scene, SceneContext,
};
use super::resolve_layer;
use super::structural::structural_scene_apply;
use crate::edit_mode::StructuralHistoryStep;

pub(in crate::app) fn apply_last_mesh_edit_undo_with_status(
    app: &mut SceneContext<'_>,
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
        app.scene_ui.status_message =
            Some(app.ui.locale.tr(crate::i18n::message_id!("undo-nothing")));
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
    app: &mut SceneContext<'_>,
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
        app.scene_ui.status_message =
            Some(app.ui.locale.tr(crate::i18n::message_id!("redo-nothing")));
        return LayerContextApply::default();
    };
    let Some(current) = scene.meshes().get(index).cloned() else {
        return LayerContextApply::default();
    };
    let layer_label = layers_overlay::layer_label(paths, &current, index, &app.ui.locale);

    match app.document.edit_mode.redo_last_scene_edit(scene, layer_id) {
        StructuralHistoryStep::Restored(restored_scene) => {
            mark_restored_scene_unsaved(app.document, scene, &restored_scene);
            *scene = restored_scene;
            app.scene_ui.status_message = Some(app.ui.locale.tr_with(
                crate::i18n::message_id!("redo-redid"),
                &[("layer", &layer_label)],
            ));
            return structural_scene_apply();
        }
        StructuralHistoryStep::SceneChanged => {
            app.scene_ui.status_message = Some(app.ui.locale.tr_with(
                crate::i18n::message_id!("redo-unavailable"),
                &[("layer", &layer_label)],
            ));
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
    app.scene_ui.status_message = Some(app.ui.locale.tr_with(
        crate::i18n::message_id!("redo-redid"),
        &[("layer", &layer_label)],
    ));
    structural_scene_apply()
}

fn mark_restored_scene_unsaved(
    document: &mut super::super::state_document::DocumentState,
    current: &Scene,
    restored: &Scene,
) {
    let previous = current
        .meshes()
        .iter()
        .map(|entry| (entry.id(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    for entry in restored.meshes() {
        if previous.get(&entry.id()).is_none_or(|before| {
            !std::sync::Arc::ptr_eq(&before.mesh, &entry.mesh)
                || before.transform != entry.transform
        }) {
            document.mark_mesh_edits_unsaved(entry.id());
        }
    }
}

pub(super) fn apply_layer_mesh_undo_action_with_status(
    app: &mut SceneContext<'_>,
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
            mark_restored_scene_unsaved(app.document, scene, &restored);
            *scene = restored;
            app.scene_ui.status_message = Some(app.ui.locale.tr_with(
                crate::i18n::message_id!("undo-undid"),
                &[("layer", &layer_label)],
            ));
            return structural_scene_apply();
        }
        StructuralHistoryStep::SceneChanged => {
            app.scene_ui.status_message = Some(app.ui.locale.tr_with(
                crate::i18n::message_id!("undo-unavailable"),
                &[("layer", &layer_label)],
            ));
            return LayerContextApply::default();
        }
        StructuralHistoryStep::NotAvailable => {}
    }
    // Single-layer undo (id-keyed, so it is append-safe on its own).
    let apply = apply_layer_mesh_undo_action(scene, request, &mut app.document.edit_mode);
    if apply.scene_changed {
        app.document.mark_mesh_edits_unsaved(request.layer_id);
        app.scene_ui.status_message = Some(app.ui.locale.tr_with(
            crate::i18n::message_id!("undo-undid"),
            &[("layer", &layer_label)],
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OccluViewApp;
    use glam::Vec3;
    use occluview_core::{Mesh, SceneMesh, ScenePickHit, Vertex};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    #[test]
    fn scene_history_marks_every_restored_mesh_unsaved() {
        for action in [
            LayerContextAction::DeleteSelectedFaces,
            LayerContextAction::CutSelectionToNewLayer,
        ] {
            let Ok(mesh) = Mesh::new(
                Some("scan".into()),
                vec![
                    Vertex::at(Vec3::ZERO),
                    Vertex::at(Vec3::X),
                    Vertex::at(Vec3::Y),
                    Vertex::at(Vec3::Z),
                ],
                vec![0, 1, 2, 0, 3, 1],
            ) else {
                panic!("valid mesh");
            };
            let mut scene = Scene::new();
            for _ in 0..3 {
                scene.add(SceneMesh::new(mesh.clone()));
            }
            let untouched_id = scene.meshes()[2].id();
            let original_ids = scene.meshes()[..2]
                .iter()
                .map(SceneMesh::id)
                .collect::<BTreeSet<_>>();
            let mut app = OccluViewApp::new_for_tests(egui::Context::default());
            let Some(mut context) = app.active_context() else {
                panic!("active scene");
            };
            context.document.scene = Some(Arc::new(scene.clone()));
            for (index, entry) in scene.meshes().iter().take(2).enumerate() {
                assert!(context.document.edit_mode.select_face_hit(
                    &scene,
                    ScenePickHit {
                        layer_index: index,
                        layer_id: entry.id(),
                        triangle_index: 0,
                        point: Vec3::ZERO,
                        distance: 1.0,
                    },
                ));
            }
            let Ok(apply) = super::super::selection_batch::apply_visible_selected_face_mesh_edit_action_with_limit(
                &mut scene,
                &mut context.document.edit_mode,
                action,
                None,
            ) else {
                panic!("selection edit");
            };
            assert!(apply.apply.scene_changed);
            let edited_ids = scene
                .meshes()
                .iter()
                .map(SceneMesh::id)
                .filter(|id| *id != untouched_id)
                .collect::<BTreeSet<_>>();

            // A completed export clears dirty flags, but keeps the history.
            context.document.clear_unsaved_mesh_edits();
            assert!(
                apply_last_mesh_edit_undo_with_status(&mut context, &mut scene, &[]).scene_changed
            );
            assert_eq!(context.document.unsaved_edit_layer_ids, original_ids);

            context.document.clear_unsaved_mesh_edits();
            assert!(
                apply_last_mesh_edit_redo_with_status(&mut context, &mut scene, &[]).scene_changed
            );
            assert_eq!(context.document.unsaved_edit_layer_ids, edited_ids);
        }
    }
}

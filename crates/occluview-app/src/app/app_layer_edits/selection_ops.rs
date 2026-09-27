//! Selection-scoped ops (Delete / Crop and the Cut / Separate entry): guards
//! for empty and whole-mesh selections, then routing to the executors.

use super::super::{
    AppErrorAction, AppErrorDialog, EditModeController, LayerContextAction, LayerContextApply,
    LayerContextRequest, OccluViewApp, PathBuf, Scene,
};
use super::selection_batch::apply_visible_selected_face_mesh_edit_action_with_limit;
use super::structural::{
    apply_cut_selection_to_new_layer, apply_separate_selected_components, structural_scene_apply,
    MAX_SEPARATE_COMPONENTS,
};
use super::whole_mesh::{edit_command_for_layer_action, layer_edit_status};
use super::{resolve_layer, with_undoable_note, SelectedFaceEditContext};
use occluview_core::{
    crop_mesh_to_selected_faces, delete_selected_faces_in_mesh,
    selected_connected_components_in_mesh, CoreError, MeshEditOptions, SceneMeshId,
};
use std::sync::Arc;

pub(super) fn apply_selected_face_mesh_edit_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let layer_label = resolve_layer(scene, paths, &request, &app.ui.locale).map_or_else(
        || {
            app.ui
                .locale
                .tr_with("layer-unnamed", &[("n", &(request.index + 1).to_string())])
        },
        |(_, label)| label,
    );

    if selection_covers_whole_mesh(scene, &request, &app.document.edit_mode) {
        app.ui.status_message = Some(match request.action {
            LayerContextAction::CropToSelectedFaces => app
                .ui
                .locale
                .tr_with("select-covers-all", &[("layer", &layer_label)]),
            _ => app
                .ui
                .locale
                .tr_with("select-covers-remove", &[("layer", &layer_label)]),
        });
        return LayerContextApply::default();
    }

    // Refuse an exploding Separate before begin_scene_edit clones the whole
    // scene for the undo snapshot — the refusal needs no snapshot at all.
    if request.action == LayerContextAction::SeparateSelectedComponents {
        if let Some(parts) = separate_component_overflow(scene, &request, &app.document.edit_mode) {
            app.ui.status_message = Some(app.ui.locale.tr_with(
                "select-splits",
                &[("parts", &parts.to_string()), ("layer", &layer_label)],
            ));
            return LayerContextApply::default();
        }
    }

    // Cut/Separate spawn new layers that exist on no disk file yet; snapshot
    // the pre-op ids so every spawned layer is also marked unsaved below.
    let ids_before: Vec<_> = scene
        .meshes()
        .iter()
        .map(occluview_core::SceneMesh::id)
        .collect();
    match apply_selected_face_mesh_edit_action(scene, request, &mut app.document.edit_mode) {
        Ok(apply) => {
            if apply.scene_changed {
                app.document.mark_mesh_edits_unsaved(request.layer_id);
                if apply.structural_scene_change {
                    let spawned: Vec<_> = scene
                        .meshes()
                        .iter()
                        .map(occluview_core::SceneMesh::id)
                        .filter(|id| !ids_before.contains(id))
                        .collect();
                    for id in spawned {
                        app.document.mark_mesh_edits_unsaved(id);
                    }
                }
                let status = layer_edit_status(&layer_label, request.action, None, &app.ui.locale);
                app.ui.status_message = Some(with_undoable_note(
                    &app.document.edit_mode,
                    &app.ui.locale,
                    status,
                ));
            } else {
                let has_selection = app
                    .document
                    .edit_mode
                    .selected_faces_for_layer(request.layer_id)
                    .is_some_and(|selection| selection.selected_count() > 0);
                app.ui.status_message = Some(
                    if let Some(parts) =
                        separate_component_overflow(scene, &request, &app.document.edit_mode)
                    {
                        app.ui.locale.tr_with(
                            "select-splits",
                            &[("parts", &parts.to_string()), ("layer", &layer_label)],
                        )
                    } else if has_selection {
                        app.ui
                            .locale
                            .tr_with("edit-no-changes", &[("layer", &layer_label)])
                    } else {
                        app.ui.locale.tr("edit-select-faces-first")
                    },
                );
            }
            apply
        }
        Err(error) => {
            let summary = app.ui.locale.tr_with(
                "edit-apply-failed-summary",
                &[("detail", &error.to_string())],
            );
            app.ui.status_message = Some(summary.clone());
            app.ui.app_error = Some(AppErrorDialog {
                title: app.ui.locale.tr("edit-apply-failed-title"),
                summary,
                details: format!(
                    "Selection edit failed\n\nLayer:\n{layer_label}\n\nError:\n{error:#}"
                ),
                action: AppErrorAction::None,
            });
            LayerContextApply::default()
        }
    }
}

/// Run a menu selection action across every layer the operator marked.
///
/// This matches the Mesh Editor's own buttons: an action taken from the layer
/// context menu applies to every marked layer, not only the layer the menu was
/// opened on. The menu chooses which action runs; the visible selection plan
/// decides what it runs on.
pub(super) fn apply_visible_selection_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let plan = app.document.edit_mode.visible_selection_plan(scene);
    if plan.is_empty() {
        // Not a multi-layer request after all: the menu may have been opened on
        // a layer whose marks belong to the page the operator is not looking at.
        // Fall back to the single-layer path so the refusal copy stays exact.
        return apply_selected_face_mesh_edit_action_with_status(app, scene, paths, request);
    }
    let target_layers: Vec<SceneMeshId> = plan.iter().map(|selection| selection.layer_id).collect();
    let ids_before: Vec<SceneMeshId> = scene
        .meshes()
        .iter()
        .map(occluview_core::SceneMesh::id)
        .collect();
    let action = request.action;
    match apply_visible_selected_face_mesh_edit_action_with_limit(
        scene,
        &mut app.document.edit_mode,
        action,
        None,
    ) {
        Ok(apply) if apply.scene_changed => {
            for layer_id in &target_layers {
                app.document.mark_mesh_edits_unsaved(*layer_id);
            }
            for id in scene
                .meshes()
                .iter()
                .map(occluview_core::SceneMesh::id)
                .filter(|id| !ids_before.contains(id))
            {
                app.document.mark_mesh_edits_unsaved(id);
            }
            app.ui.status_message = Some(app.ui.locale.tr_plural(
                "batchedit-status",
                &[(
                    "label",
                    &super::whole_mesh::batch_action_label(action, &app.ui.locale),
                )],
                &[("n", target_layers.len())],
            ));
            apply
        }
        Ok(_) => {
            app.ui.status_message = Some(app.ui.locale.tr("edit-no-changes-hidden"));
            LayerContextApply::default()
        }
        Err(error) => {
            let summary = app.ui.locale.tr_with(
                "edit-apply-failed-summary",
                &[("detail", &error.to_string())],
            );
            app.ui.status_message = Some(summary.clone());
            app.ui.app_error = Some(AppErrorDialog {
                title: app.ui.locale.tr("edit-apply-failed-title"),
                summary,
                details: format!("Multi-layer selection edit failed\n\nError:\n{error:#}"),
                action: AppErrorAction::None,
            });
            LayerContextApply::default()
        }
    }
}

pub(super) fn apply_selected_face_mesh_edit_action(
    scene: &mut Scene,
    request: LayerContextRequest,
    edit_mode: &mut EditModeController,
) -> Result<LayerContextApply, CoreError> {
    let LayerContextRequest {
        index,
        layer_id,
        action,
    } = request;
    if !matches!(
        action,
        LayerContextAction::DeleteSelectedFaces
            | LayerContextAction::CropToSelectedFaces
            | LayerContextAction::CutSelectionToNewLayer
            | LayerContextAction::SeparateSelectedComponents
    ) {
        return Ok(LayerContextApply::default());
    }
    let Some(entry) = scene.meshes().get(index) else {
        return Ok(LayerContextApply::default());
    };
    if entry.id() != layer_id {
        return Ok(LayerContextApply::default());
    }
    let Some(selection) = edit_mode.selected_faces_for_layer(layer_id) else {
        return Ok(LayerContextApply::default());
    };
    if selection.selected_count() == 0 {
        return Ok(LayerContextApply::default());
    }
    // A whole-mesh selection would leave an empty source mesh behind
    // (delete/cut/separate) or change nothing (crop): refuse instead of
    // producing a dead zero-triangle layer.
    if selection_covers_whole_mesh(scene, &request, edit_mode) {
        return Ok(LayerContextApply::default());
    }
    let Some(command) = edit_command_for_layer_action(action) else {
        return Ok(LayerContextApply::default());
    };
    let token = if matches!(
        action,
        LayerContextAction::CutSelectionToNewLayer | LayerContextAction::SeparateSelectedComponents
    ) {
        edit_mode.begin_scene_edit(scene, layer_id, command)
    } else {
        edit_mode.begin_layer_edit(entry, command)
    };
    let Some(token) = token else {
        return Ok(LayerContextApply::default());
    };

    match action {
        LayerContextAction::DeleteSelectedFaces | LayerContextAction::CropToSelectedFaces => {
            apply_single_layer_selected_face_edit(
                scene,
                SelectedFaceEditContext {
                    index,
                    layer_id,
                    token,
                },
                action,
                &selection,
                edit_mode,
            )
        }
        LayerContextAction::CutSelectionToNewLayer => apply_cut_selection_to_new_layer(
            scene,
            SelectedFaceEditContext {
                index,
                layer_id,
                token,
            },
            &selection,
            edit_mode,
        ),
        LayerContextAction::SeparateSelectedComponents => apply_separate_selected_components(
            scene,
            SelectedFaceEditContext {
                index,
                layer_id,
                token,
            },
            &selection,
            edit_mode,
        ),
        _ => Ok(LayerContextApply::default()),
    }
}

fn apply_single_layer_selected_face_edit(
    scene: &mut Scene,
    context: SelectedFaceEditContext,
    action: LayerContextAction,
    selection: &occluview_core::FaceSelection,
    edit_mode: &mut EditModeController,
) -> Result<LayerContextApply, CoreError> {
    let Some(entry) = scene.meshes_mut().get_mut(context.index) else {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    };
    if entry.id() != context.layer_id {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    let edited = selected_face_edit_result(&entry.mesh, selection, action);
    let edited = match edited {
        Ok(edited) => edited,
        Err(error) => {
            let _ = edit_mode.finish_layer_edit_error(context.token, error.to_string());
            return Err(error);
        }
    };

    // Delete/Crop always change content: a non-empty, non-whole-mesh selection
    // is guaranteed by the caller, so the edited mesh is always a real change.
    entry.mesh = Arc::new(edited.mesh);
    let _ = edit_mode.finish_layer_edit_success(context.token);
    Ok(structural_scene_apply())
}

pub(super) fn selected_face_edit_result(
    mesh: &occluview_core::Mesh,
    selection: &occluview_core::FaceSelection,
    action: LayerContextAction,
) -> Result<occluview_core::CoreMeshEditResult, CoreError> {
    match action {
        LayerContextAction::DeleteSelectedFaces => delete_selected_faces_in_mesh(
            mesh,
            selection,
            MeshEditOptions {
                compact_vertices: true,
                ..MeshEditOptions::default()
            },
        ),
        LayerContextAction::CropToSelectedFaces => crop_mesh_to_selected_faces(
            mesh,
            selection,
            MeshEditOptions {
                compact_vertices: true,
                ..MeshEditOptions::default()
            },
        ),
        // Only the two face-edit actions above ever reach this adapter (the
        // callers gate on exactly those). A different action here is an internal
        // routing error: log it and fail the edit rather than abort the
        // process (the build ships `panic = "abort"`, so an `unreachable!` would
        // be a hard crash). Callers already surface this `Err` as a failed edit.
        other => {
            tracing::error!(?other, "non-face-edit action routed to face-edit adapter");
            Err(CoreError::Geometry(format!(
                "internal: {other:?} is not a face edit"
            )))
        }
    }
}

/// When a Separate request is refused because its selection fragmented past the
/// cap, the number of components to report in the status. Returns `None` for any
/// other action or when the selection is within the cap. This labels the mesh
/// only on the (rare) refusal path — a successful Separate never reaches it, so
/// the hot path stays single-label.
fn separate_component_overflow(
    scene: &Scene,
    request: &LayerContextRequest,
    edit_mode: &EditModeController,
) -> Option<usize> {
    if request.action != LayerContextAction::SeparateSelectedComponents {
        return None;
    }
    let entry = scene
        .meshes()
        .get(request.index)
        .filter(|entry| entry.id() == request.layer_id)?;
    let selection = edit_mode.selected_faces_for_layer(request.layer_id)?;
    let components = selected_connected_components_in_mesh(&entry.mesh, &selection).ok()?;
    (components.len() > MAX_SEPARATE_COMPONENTS).then_some(components.len())
}

fn selection_covers_whole_mesh(
    scene: &Scene,
    request: &LayerContextRequest,
    edit_mode: &EditModeController,
) -> bool {
    let Some(entry) = scene
        .meshes()
        .get(request.index)
        .filter(|entry| entry.id() == request.layer_id)
    else {
        return false;
    };
    let total = entry.mesh.triangle_count();
    total > 0
        && edit_mode
            .selected_faces_for_layer(request.layer_id)
            .is_some_and(|selection| selection.selected_count() == total)
}

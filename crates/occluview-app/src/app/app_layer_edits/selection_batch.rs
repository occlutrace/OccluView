//! Atomic operations over the canonical visible multi-layer selection plan.

use super::super::{EditModeController, LayerContextAction, LayerContextApply, Scene};
use super::selection_ops::selected_face_edit_result;
use super::structural::{
    clone_layer_with_mesh, cut_selection_meshes, split_selection_into_meshes,
    structural_scene_apply, MAX_SEPARATE_COMPONENTS,
};
use super::whole_mesh::{close_holes_in_mesh, edit_command_for_layer_action};
use occluview_core::{
    selected_connected_components_in_mesh, CoreError, FaceSelection, Mesh, SceneMeshId,
};

enum PlannedEdit {
    Replace {
        layer_id: SceneMeshId,
        mesh: Mesh,
    },
    Cut {
        layer_id: SceneMeshId,
        remainder: Mesh,
        extracted: Mesh,
    },
    Separate {
        layer_id: SceneMeshId,
        remainder: Mesh,
        components: Vec<Mesh>,
    },
}

/// Apply one selection operation to every visible, selected layer.
///
/// The canonical plan is read from the controller in scene order. All target
/// validation and mesh construction happen before the scene-edit token is
/// opened. After the single whole-scene snapshot is admitted, results are
/// applied to a clone and swapped into the caller only after every layer has
/// succeeded.
#[cfg(test)]
pub(crate) fn apply_visible_selected_face_mesh_edit_action(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    action: LayerContextAction,
) -> Result<LayerContextApply, CoreError> {
    apply_visible_selected_face_mesh_edit_action_with_limit(scene, edit_mode, action, None)
}

/// Apply a Mesh Editor operation across the scene. Close Holes intentionally
/// differs from the face-structural tools: marks scope the repair when present,
/// while an empty selection means every visible layer gets safe interior-hole
/// repair. Hidden layers never enter either plan.
pub(crate) fn apply_visible_selected_face_mesh_edit_action_with_limit(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    action: LayerContextAction,
    close_holes_limit_mm: Option<f32>,
) -> Result<LayerContextApply, CoreError> {
    if action == LayerContextAction::CloseHoles {
        return apply_visible_close_holes(scene, edit_mode, close_holes_limit_mm);
    }
    let Some(command) = edit_command_for_layer_action(action) else {
        return Ok(LayerContextApply::default());
    };
    if !matches!(
        action,
        LayerContextAction::DeleteSelectedFaces
            | LayerContextAction::CropToSelectedFaces
            | LayerContextAction::CutSelectionToNewLayer
            | LayerContextAction::SeparateSelectedComponents
    ) {
        return Ok(LayerContextApply::default());
    }

    let plan = edit_mode.visible_selection_plan(scene);
    let Some(focus_layer_id) = plan.first().map(|selection| selection.layer_id) else {
        return Ok(LayerContextApply::default());
    };
    let mut planned = Vec::with_capacity(plan.len());
    for selection in &plan {
        let Some(source) = scene.meshes().iter().find(|entry| {
            entry.id() == selection.layer_id
                && entry.visible
                && entry.mesh.topology_id() == selection.topology_id
                && entry.mesh.triangle_count() == selection.selection.len()
        }) else {
            return Ok(LayerContextApply::default());
        };
        if selection.selection.selected_count() == 0
            || selection.selection.selected_count() == source.mesh.triangle_count()
        {
            return Ok(LayerContextApply::default());
        }

        planned.push(match action {
            LayerContextAction::DeleteSelectedFaces | LayerContextAction::CropToSelectedFaces => {
                PlannedEdit::Replace {
                    layer_id: source.id(),
                    mesh: selected_face_edit_result(&source.mesh, &selection.selection, action)?
                        .mesh,
                }
            }
            LayerContextAction::CutSelectionToNewLayer => {
                let (remainder, extracted) = cut_selection_meshes(source, &selection.selection)?;
                PlannedEdit::Cut {
                    layer_id: source.id(),
                    remainder,
                    extracted,
                }
            }
            LayerContextAction::SeparateSelectedComponents => {
                let components =
                    selected_connected_components_in_mesh(&source.mesh, &selection.selection)?;
                if components.len() > MAX_SEPARATE_COMPONENTS {
                    return Ok(LayerContextApply::default());
                }
                let split = split_selection_into_meshes(&source.mesh, &components)?;
                PlannedEdit::Separate {
                    layer_id: source.id(),
                    remainder: split.remainder,
                    components: split.components,
                }
            }
            _ => return Ok(LayerContextApply::default()),
        });
    }

    let Some(token) = edit_mode.begin_scene_edit(scene, focus_layer_id, command) else {
        return Ok(LayerContextApply::default());
    };
    if !edit_mode.last_edit_undoable() {
        let _ = edit_mode.finish_layer_edit_noop(token);
        return Ok(LayerContextApply::default());
    }

    let mut draft = scene.clone();
    for edit in planned {
        if !apply_planned_edit(&mut draft, edit) {
            // This can only indicate an internal stale-plan mismatch. The
            // caller's scene is still untouched, so discard the token cleanly.
            let _ = edit_mode.finish_layer_edit_noop(token);
            return Ok(LayerContextApply::default());
        }
    }

    let _ = edit_mode.finish_scene_edit_success(token, &draft);
    *scene = draft;
    Ok(structural_scene_apply())
}

fn apply_visible_close_holes(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    close_holes_limit_mm: Option<f32>,
) -> Result<LayerContextApply, CoreError> {
    let selection_plan = edit_mode.visible_selection_plan(scene);
    let targets = if selection_plan.is_empty() {
        scene
            .meshes()
            .iter()
            .filter(|entry| {
                entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
            })
            .map(|entry| (entry.id(), None))
            .collect::<Vec<(SceneMeshId, Option<FaceSelection>)>>()
    } else {
        selection_plan
            .into_iter()
            .map(|selection| (selection.layer_id, Some(selection.selection)))
            .collect()
    };
    let Some(focus_layer_id) = targets.first().map(|(layer_id, _)| *layer_id) else {
        return Ok(LayerContextApply::default());
    };

    let mut planned = Vec::with_capacity(targets.len());
    for (layer_id, selection) in targets {
        let Some(source) = scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == layer_id && entry.visible)
        else {
            return Ok(LayerContextApply::default());
        };
        if source.mesh.is_point_cloud() || source.mesh.triangle_count() == 0 {
            return Ok(LayerContextApply::default());
        }
        if let Some(selection) = selection.as_ref() {
            if selection.len() != source.mesh.triangle_count() {
                return Ok(LayerContextApply::default());
            }
        }
        let repaired = close_holes_in_mesh(&source.mesh, selection.as_ref(), close_holes_limit_mm)?;
        if repaired.report.filled_holes > 0 {
            planned.push(PlannedEdit::Replace {
                layer_id,
                mesh: repaired.mesh,
            });
        }
    }
    if planned.is_empty() {
        return Ok(LayerContextApply::default());
    }

    let Some(token) = edit_mode.begin_scene_edit(
        scene,
        focus_layer_id,
        super::super::EditModeCommand::CloseHoles,
    ) else {
        return Ok(LayerContextApply::default());
    };
    if !edit_mode.last_edit_undoable() {
        let _ = edit_mode.finish_layer_edit_noop(token);
        return Ok(LayerContextApply::default());
    }

    let mut draft = scene.clone();
    for edit in planned {
        if !apply_planned_edit(&mut draft, edit) {
            let _ = edit_mode.finish_layer_edit_noop(token);
            return Ok(LayerContextApply::default());
        }
    }
    let _ = edit_mode.finish_scene_edit_success(token, &draft);
    *scene = draft;
    Ok(structural_scene_apply())
}

fn apply_planned_edit(scene: &mut Scene, edit: PlannedEdit) -> bool {
    let layer_id = match &edit {
        PlannedEdit::Replace { layer_id, .. }
        | PlannedEdit::Cut { layer_id, .. }
        | PlannedEdit::Separate { layer_id, .. } => *layer_id,
    };
    let Some(index) = scene
        .meshes()
        .iter()
        .position(|entry| entry.id() == layer_id)
    else {
        return false;
    };
    let source = scene.meshes()[index].clone();
    match edit {
        PlannedEdit::Replace { mesh, .. } => scene.meshes_mut()[index].mesh = mesh,
        PlannedEdit::Cut {
            remainder,
            extracted,
            ..
        } => {
            scene.meshes_mut()[index].mesh = remainder;
            scene.insert(
                index + 1,
                clone_layer_with_mesh(&source, extracted)
                    .with_tint(crate::layer_actions::next_layer_tint(source.tint)),
            );
        }
        PlannedEdit::Separate {
            remainder,
            components,
            ..
        } => {
            scene.meshes_mut()[index].mesh = remainder;
            let mut part_tint = source.tint;
            for (offset, mesh) in components.into_iter().enumerate() {
                part_tint = crate::layer_actions::next_layer_tint(part_tint);
                scene.insert(
                    index + 1 + offset,
                    clone_layer_with_mesh(&source, mesh).with_tint(part_tint),
                );
            }
        }
    }
    true
}

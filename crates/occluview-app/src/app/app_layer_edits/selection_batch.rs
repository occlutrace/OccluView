//! Atomic operations over the canonical visible multi-layer selection plan.

use super::super::{
    EditModeCommand, EditModeController, LayerContextAction, LayerContextApply, Scene,
};
use super::selection_ops::selected_face_edit_result;
use super::structural::{
    clone_layer_with_mesh, cut_selection_meshes, split_selection_into_meshes,
    structural_scene_apply, MAX_SEPARATE_COMPONENTS,
};
use super::whole_mesh::{
    close_holes_changes_content, close_holes_in_mesh, edit_command_for_layer_action,
};
use occluview_core::{CoreError, Mesh, SceneMeshId};
use occluview_mesh_edit::{selected_connected_components_in_mesh, FaceSelection, MeshEditReport};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct SelectionBatchOutcome {
    pub(crate) apply: LayerContextApply,
    pub(crate) refusal: Option<SelectionBatchRefusal>,
    pub(crate) holes: Vec<(SceneMeshId, MeshEditReport)>,
    /// Layers whose mesh changed and that are still in the scene.
    pub(crate) changed_layers: Vec<SceneMeshId>,
    /// Layers a whole-mesh deletion took out of the scene.
    pub(crate) removed_layers: usize,
}

#[derive(Debug)]
pub(crate) enum SelectionBatchRefusal {
    NoSelection,
    StaleSelection,
    WholeSelection(SceneMeshId),
    TooManyComponents(SceneMeshId, usize),
    HistoryBudget,
    Busy,
}

impl SelectionBatchOutcome {
    fn refused(reason: SelectionBatchRefusal) -> Self {
        Self {
            apply: LayerContextApply::default(),
            refusal: Some(reason),
            holes: Vec::new(),
            changed_layers: Vec::new(),
            removed_layers: 0,
        }
    }
    fn completed(
        apply: LayerContextApply,
        holes: Vec<(SceneMeshId, MeshEditReport)>,
        changed_layers: Vec<SceneMeshId>,
    ) -> Self {
        Self {
            apply,
            refusal: None,
            holes,
            changed_layers,
            removed_layers: 0,
        }
    }
}

enum PlannedEdit {
    Replace {
        layer_id: SceneMeshId,
        mesh: Mesh,
    },
    /// Every face of the layer was marked for deletion, so the layer goes.
    Remove {
        layer_id: SceneMeshId,
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

impl PlannedEdit {
    fn layer_id(&self) -> SceneMeshId {
        match self {
            Self::Replace { layer_id, .. }
            | Self::Remove { layer_id }
            | Self::Cut { layer_id, .. }
            | Self::Separate { layer_id, .. } => *layer_id,
        }
    }

    /// Whether the layer is still in the scene after the edit.
    fn keeps_layer(&self) -> bool {
        !matches!(self, Self::Remove { .. })
    }
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
) -> Result<SelectionBatchOutcome, CoreError> {
    apply_visible_selected_face_mesh_edit_action_with_limit(scene, edit_mode, action, None)
}

/// Apply a Mesh Editor operation across the scene. Close Holes follows the
/// same explicit-selection contract dental CAD software uses: only marked
/// faces on visible layers enter the repair plan. An empty selection is a
/// no-op; hidden layers never enter the plan.
pub(crate) fn apply_visible_selected_face_mesh_edit_action_with_limit(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    action: LayerContextAction,
    close_holes_limit_mm: Option<f32>,
) -> Result<SelectionBatchOutcome, CoreError> {
    if edit_mode.is_busy() {
        return Ok(SelectionBatchOutcome::refused(SelectionBatchRefusal::Busy));
    }
    if action == LayerContextAction::CloseHoles {
        return apply_visible_close_holes(scene, edit_mode, close_holes_limit_mm);
    }
    let Some(command) = edit_command_for_layer_action(action) else {
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::StaleSelection,
        ));
    };
    if !matches!(
        action,
        LayerContextAction::DeleteSelectedFaces
            | LayerContextAction::CropToSelectedFaces
            | LayerContextAction::CutSelectionToNewLayer
            | LayerContextAction::SeparateSelectedComponents
    ) {
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::StaleSelection,
        ));
    }

    let plan = edit_mode.visible_selection_plan(scene);
    let Some(focus_layer_id) = plan.first().map(|selection| selection.layer_id) else {
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::NoSelection,
        ));
    };
    let mut planned = Vec::with_capacity(plan.len());
    for selection in &plan {
        let Some(source) = scene.meshes().iter().find(|entry| {
            entry.id() == selection.layer_id
                && entry.visible
                && entry.mesh.topology_id() == selection.topology_id
                && entry.mesh.triangle_count() == selection.selection.len()
        }) else {
            return Ok(SelectionBatchOutcome::refused(
                SelectionBatchRefusal::StaleSelection,
            ));
        };
        match plan_layer_edit(source, &selection.selection, action)? {
            Ok(edit) => planned.push(edit),
            Err(refusal) => return Ok(SelectionBatchOutcome::refused(refusal)),
        }
    }

    // Deleting the last layer would close the scene, which is not what a
    // face deletion is for; that stays with the layer's own Remove.
    let removed = planned.iter().filter(|edit| !edit.keeps_layer()).count();
    if removed == scene.meshes().len() {
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::WholeSelection(focus_layer_id),
        ));
    }

    Ok(commit_selection_plan(
        scene,
        edit_mode,
        focus_layer_id,
        command,
        planned,
    ))
}

/// What `action` does to one layer whose marked faces are `selection`, or why
/// it does nothing to it. The mesh work happens here, before any history
/// snapshot is taken.
fn plan_layer_edit(
    source: &occluview_core::SceneMesh,
    selection: &FaceSelection,
    action: LayerContextAction,
) -> Result<Result<PlannedEdit, SelectionBatchRefusal>, CoreError> {
    let layer_id = source.id();
    if selection.selected_count() == 0 {
        return Ok(Err(SelectionBatchRefusal::NoSelection));
    }
    if selection.selected_count() == source.mesh.triangle_count() {
        // A whole object marked and deleted is the object deleted: an Object
        // pick on a scan that is one piece marks all of it, and leaving a
        // layer with no triangles behind would only be a refusal with extra
        // steps. The other three have nothing to do with a whole mesh.
        return Ok(if action == LayerContextAction::DeleteSelectedFaces {
            Ok(PlannedEdit::Remove { layer_id })
        } else {
            Err(SelectionBatchRefusal::WholeSelection(layer_id))
        });
    }
    Ok(Ok(match action {
        LayerContextAction::DeleteSelectedFaces | LayerContextAction::CropToSelectedFaces => {
            PlannedEdit::Replace {
                layer_id,
                mesh: selected_face_edit_result(&source.mesh, selection, action)?.mesh,
            }
        }
        LayerContextAction::CutSelectionToNewLayer => {
            let (remainder, extracted) = cut_selection_meshes(source, selection)?;
            PlannedEdit::Cut {
                layer_id,
                remainder,
                extracted,
            }
        }
        LayerContextAction::SeparateSelectedComponents => {
            let components = selected_connected_components_in_mesh(&source.mesh, selection)?;
            if components.len() > MAX_SEPARATE_COMPONENTS {
                return Ok(Err(SelectionBatchRefusal::TooManyComponents(
                    layer_id,
                    components.len(),
                )));
            }
            let split = split_selection_into_meshes(&source.mesh, &components)?;
            PlannedEdit::Separate {
                layer_id,
                remainder: split.remainder,
                components: split.components,
            }
        }
        _ => return Ok(Err(SelectionBatchRefusal::StaleSelection)),
    }))
}

fn commit_selection_plan(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    focus_layer_id: SceneMeshId,
    command: EditModeCommand,
    planned: Vec<PlannedEdit>,
) -> SelectionBatchOutcome {
    // History reaches a scene step through a layer that is in the scene on
    // both sides of it, so a step that removes its own focus is filed under a
    // layer that stays.
    let stays = |id: SceneMeshId| {
        planned
            .iter()
            .all(|edit| edit.keeps_layer() || edit.layer_id() != id)
    };
    let focus_layer_id = if stays(focus_layer_id) {
        Some(focus_layer_id)
    } else {
        scene
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .find(|id| stays(*id))
    };
    let Some(token) =
        focus_layer_id.and_then(|focus| edit_mode.begin_scene_edit(scene, focus, command))
    else {
        return SelectionBatchOutcome::refused(SelectionBatchRefusal::Busy);
    };
    if !edit_mode.last_edit_undoable() {
        let _ = edit_mode.finish_layer_edit_noop(token);
        return SelectionBatchOutcome::refused(SelectionBatchRefusal::HistoryBudget);
    }

    let changed_layers = planned
        .iter()
        .filter(|edit| edit.keeps_layer())
        .map(PlannedEdit::layer_id)
        .collect();
    let removed_layers = planned.iter().filter(|edit| !edit.keeps_layer()).count();
    let mut draft = scene.clone();
    for edit in planned {
        if !apply_planned_edit(&mut draft, edit) {
            // This can only indicate an internal stale-plan mismatch. The
            // caller's scene is still untouched, so discard the token cleanly.
            let _ = edit_mode.finish_layer_edit_noop(token);
            return SelectionBatchOutcome::refused(SelectionBatchRefusal::StaleSelection);
        }
    }

    let _ = edit_mode.finish_scene_edit_success(token, &draft);
    *scene = draft;
    SelectionBatchOutcome {
        removed_layers,
        ..SelectionBatchOutcome::completed(structural_scene_apply(), Vec::new(), changed_layers)
    }
}

fn apply_visible_close_holes(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    close_holes_limit_mm: Option<f32>,
) -> Result<SelectionBatchOutcome, CoreError> {
    let selection_plan = edit_mode.visible_selection_plan(scene);
    let targets = selection_plan
        .into_iter()
        .map(|selection| (selection.layer_id, selection.selection))
        .collect::<Vec<(SceneMeshId, FaceSelection)>>();
    let Some(focus_layer_id) = targets.first().map(|(layer_id, _)| *layer_id) else {
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::NoSelection,
        ));
    };

    let mut planned = Vec::with_capacity(targets.len());
    let mut holes = Vec::with_capacity(targets.len());
    for (layer_id, selection) in targets {
        let Some(source) = scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == layer_id && entry.visible)
        else {
            return Ok(SelectionBatchOutcome::refused(
                SelectionBatchRefusal::StaleSelection,
            ));
        };
        if source.mesh.is_point_cloud() || source.mesh.triangle_count() == 0 {
            return Ok(SelectionBatchOutcome::refused(
                SelectionBatchRefusal::StaleSelection,
            ));
        }
        if selection.len() != source.mesh.triangle_count() {
            return Ok(SelectionBatchOutcome::refused(
                SelectionBatchRefusal::StaleSelection,
            ));
        }
        let repaired = close_holes_in_mesh(&source.mesh, &selection, close_holes_limit_mm)?;
        holes.push((layer_id, repaired.report.clone()));
        if close_holes_changes_content(&repaired.report) {
            planned.push(PlannedEdit::Replace {
                layer_id,
                mesh: repaired.mesh,
            });
        }
    }
    if planned.is_empty() {
        return Ok(SelectionBatchOutcome::completed(
            LayerContextApply::default(),
            holes,
            Vec::new(),
        ));
    }

    let Some(token) =
        edit_mode.begin_scene_edit(scene, focus_layer_id, EditModeCommand::CloseHoles)
    else {
        return Ok(SelectionBatchOutcome::refused(SelectionBatchRefusal::Busy));
    };
    if !edit_mode.last_edit_undoable() {
        let _ = edit_mode.finish_layer_edit_noop(token);
        return Ok(SelectionBatchOutcome::refused(
            SelectionBatchRefusal::HistoryBudget,
        ));
    }

    let changed_layers = planned.iter().map(PlannedEdit::layer_id).collect();
    let mut draft = scene.clone();
    for edit in planned {
        if !apply_planned_edit(&mut draft, edit) {
            let _ = edit_mode.finish_layer_edit_noop(token);
            return Ok(SelectionBatchOutcome::refused(
                SelectionBatchRefusal::StaleSelection,
            ));
        }
    }
    let _ = edit_mode.finish_scene_edit_success(token, &draft);
    *scene = draft;
    Ok(SelectionBatchOutcome::completed(
        structural_scene_apply(),
        holes,
        changed_layers,
    ))
}

fn apply_planned_edit(scene: &mut Scene, edit: PlannedEdit) -> bool {
    let layer_id = edit.layer_id();
    let Some(index) = scene
        .meshes()
        .iter()
        .position(|entry| entry.id() == layer_id)
    else {
        return false;
    };
    let source = scene.meshes()[index].clone();
    match edit {
        PlannedEdit::Replace { mesh, .. } => scene.meshes_mut()[index].mesh = Arc::new(mesh),
        PlannedEdit::Remove { .. } => {
            scene.remove(index);
        }
        PlannedEdit::Cut {
            remainder,
            extracted,
            ..
        } => {
            scene.meshes_mut()[index].mesh = Arc::new(remainder);
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
            scene.meshes_mut()[index].mesh = Arc::new(remainder);
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

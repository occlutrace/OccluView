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
pub(super) mod whole_mesh;

pub(super) use undo_redo::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
};

use super::{
    layer_actions, layers_overlay, LayerContextAction, LayerContextApply, LayerContextRequest,
    OccluViewApp, PathBuf, Scene,
};
use super::{state_document::DocumentState, state_ui::UiState, AppErrorAction, AppErrorDialog};
use crate::edit_mode::{EditModeController, EditSessionToken};
use occluview_core::{CoreError, SceneMesh, SceneMeshId};
use repair::apply_layer_repair_action_with_status;
use selection_ops::apply_visible_selection_action_with_status;

#[cfg(test)]
pub(crate) use selection_batch::apply_visible_selected_face_mesh_edit_action;
pub(crate) use selection_batch::apply_visible_selected_face_mesh_edit_action_with_limit;
use undo_redo::apply_layer_mesh_undo_action_with_status;
use whole_mesh::apply_layer_mesh_edit_action_with_status;

#[derive(Clone, Copy)]
struct SelectedFaceEditContext {
    index: usize,
    layer_id: SceneMeshId,
    token: EditSessionToken,
}

pub(super) fn apply_layer_context_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    if app.tools.bridge_split_active() {
        app.ui.status_message = Some(app.ui.locale.tr("bridge-busy"));
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
        // The operator marks faces with the Marquee or the Lasso, and those
        // marks live on every visible layer they crossed. Running the action
        // against only the layer the menu was opened on would leave the other
        // marked layers untouched, so the action follows the same
        // visible-selection plan the Mesh Editor's own buttons use.
        return apply_visible_selection_action_with_status(app, scene, paths, request);
    }

    if matches!(
        request.action,
        LayerContextAction::CloseHoles | LayerContextAction::InvertNormals
    ) {
        return apply_layer_mesh_edit_action_with_status(app, scene, paths, request);
    }

    if request.action == LayerContextAction::RepairMesh {
        return apply_layer_repair_action_with_status(app, scene, paths, request);
    }

    // A contact reading is not a mesh edit: it measures the scene and paints
    // the measurement. It is routed here because this is where a layer context
    // action lands, and it must run before the edit-mode guard below decides
    // anything about the mesh.
    if matches!(
        request.action,
        LayerContextAction::Contacts | LayerContextAction::HideContacts
    ) {
        app.apply_contact_context_action(scene, request);
        return LayerContextApply::default();
    }

    if request.action == LayerContextAction::ExportLayer {
        app.save_layer_export_dialog(scene, paths, request);
        return LayerContextApply::default();
    }

    if request.action != LayerContextAction::Remove {
        return layer_actions::apply_layer_context_action(scene, request);
    }

    let Some((_, removed_label)) = resolve_layer(scene, paths, &request, &app.ui.locale) else {
        return LayerContextApply::default();
    };
    let apply = layer_actions::apply_layer_context_action(scene, request);
    if apply.scene_changed {
        app.ui.status_message = Some(
            app.ui
                .locale
                .tr_with("layer-removed", &[("label", &removed_label)]),
        );
    }
    apply
}

fn begin_face_selection_with_status(
    app: &mut OccluViewApp,
    scene: &Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) {
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request, &app.ui.locale) else {
        return;
    };
    let switching_target = app.document.edit_mode.selected_layer_id() != Some(entry.id());
    if app.document.edit_mode.begin_face_selection(entry, scene) {
        // Open on the Edit Mesh tab so the session starts in selection/repair.
        app.tools.editor_tab = crate::mesh_editor_overlay::EditorTab::EditMesh;
        if switching_target {
            // A lasso's screen points belong to its previous mesh. Do not let
            // a layer-row context action carry that outline into a new target.
            app.document.mesh_selection_drag = None;
        }
        app.render.invalidation.selection_changed();
        // Start the sculpt preparation while the operator is still choosing a
        // mesh-editor action. This removes the one-time weld/adjacency wait
        // from the first sculpt stroke without blocking the editor UI.
        app.prepare_armed_sculpt_session();
        app.ui.status_message = Some(
            app.ui
                .locale
                .tr_with("layer-face-selection", &[("label", &layer_label)]),
        );
    } else {
        app.ui.status_message = Some(
            app.ui
                .locale
                .tr_with("select-faces-cannot", &[("layer", &layer_label)]),
        );
    }
}

/// Resolve a context-request's layer index+id against the live scene and
/// build its display label — the lookup-check-label sequence shared by the
/// layer-edit executors.
pub(super) fn resolve_layer<'s>(
    scene: &'s Scene,
    paths: &[PathBuf],
    request: &LayerContextRequest,
    locale: &crate::i18n::LocaleManager,
) -> Option<(&'s SceneMesh, String)> {
    let entry = scene.meshes().get(request.index)?;
    if entry.id() != request.layer_id {
        return None;
    }
    Some((
        entry,
        layers_overlay::layer_label(paths, entry, request.index, locale),
    ))
}

/// Append the "not undoable" note when the last edit's pre-op snapshot was
/// skipped (oversized) — the suffix shared by the mesh-edit status lines.
///
/// Canonical English: "{status} (not undoable: snapshot too large)".
/// One whole message (`edit-locked-status` with `$status` data) so no
/// language freezes English word order around the note.
pub(super) fn with_undoable_note(
    edit_mode: &EditModeController,
    locale: &crate::i18n::LocaleManager,
    status: String,
) -> String {
    if edit_mode.last_edit_undoable() {
        status
    } else {
        locale.tr_with("edit-locked-status", &[("status", &status)])
    }
}

/// What one token-based layer mesh-edit resolved to: the kernel verdict
/// translated into document/undo/notification effects by
/// [`commit_layer_edit`].
pub(super) enum LayerEditResolution {
    /// Applied; `changed` selects the undoable commit (unsaved-tracked,
    /// undoability-noted status) from the content no-op (snapshot discarded,
    /// status as written).
    Applied { changed: bool, status: String },
    /// Refused by the kernel; rendered into the shared failure dialog.
    Failed {
        error: CoreError,
        layer_label: String,
    },
}

/// The busy-session refusal shared by every token-based executor: no
/// snapshot is taken, so there is nothing to finish.
pub(super) fn refuse_busy_layer_edit(ui: &mut UiState) -> LayerContextApply {
    let busy = ui.locale.tr("repair-edit-busy");
    ui.status_message = Some(busy);
    LayerContextApply::default()
}

/// Apply one resolved layer edit: finish the edit session, track unsaved
/// state, and notify — the document/undo/notification combination every
/// token-based executor must otherwise remember. Status gains the
/// undoability note from the edit state; failures open the copyable error
/// dialog. Takes the affected owners, not the whole app, so the effects
/// stay explicit and the boundary stays headless-testable.
pub(super) fn commit_layer_edit(
    document: &mut DocumentState,
    ui: &mut UiState,
    token: EditSessionToken,
    layer_id: SceneMeshId,
    resolution: LayerEditResolution,
) {
    match resolution {
        LayerEditResolution::Applied {
            changed: true,
            status,
        } => {
            document.mark_mesh_edits_unsaved(layer_id);
            let _ = document.edit_mode.finish_layer_edit_success(token);
            ui.status_message = Some(with_undoable_note(&document.edit_mode, &ui.locale, status));
        }
        LayerEditResolution::Applied {
            changed: false,
            status,
        } => {
            let _ = document.edit_mode.finish_layer_edit_noop(token);
            ui.status_message = Some(status);
        }
        LayerEditResolution::Failed { error, layer_label } => {
            let summary = ui.locale.tr_with(
                "repair-edit-failed-summary",
                &[("detail", &error.to_string())],
            );
            let _ = document
                .edit_mode
                .finish_layer_edit_error(token, error.to_string());
            ui.status_message = Some(summary.clone());
            let title = ui.locale.tr("repair-edit-failed-title");
            ui.app_error = Some(AppErrorDialog {
                title,
                summary,
                details: format!("Layer edit failed\n\nLayer:\n{layer_label}\n\nError:\n{error:#}"),
                action: AppErrorAction::None,
            });
        }
    }
}

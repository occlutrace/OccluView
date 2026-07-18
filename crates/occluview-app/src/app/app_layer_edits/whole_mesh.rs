//! Layer mesh ops (Close Holes / Keep Largest / Invert Normals): dispatch with
//! honest content no-ops, plus the operator status lines.

use super::super::{
    AppErrorDialog, EditModeCommand, LayerContextAction, LayerContextApply, LayerContextRequest,
    OccluViewApp, PathBuf, Scene,
};
use super::structural::structural_scene_apply;
use super::{resolve_layer, with_undoable_note};
use occluview_core::{
    fill_selected_holes_in_mesh, invert_mesh_orientation, smooth_selected_faces_in_mesh, CoreError,
    CoreMeshEditResult, FaceSelection, Mesh, MeshEditOptions, MeshEditReport,
};

/// Generous edge ceiling for the interactive Close Holes action. With the mm
/// perimeter slider doing the real limiting, the edge count is only a safety
/// valve, so it must not spuriously refuse a legitimate hole on a densely
/// triangulated scan. Bounded under the kernel ear-clip's u16 rim limit.
const CLOSE_HOLES_EDGE_CEILING: usize = 20_000;

pub(super) fn apply_layer_mesh_edit_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request) else {
        return LayerContextApply::default();
    };
    let Some(command) = edit_command_for_layer_action(request.action) else {
        return LayerContextApply::default();
    };

    let selection = if request.action == LayerContextAction::CloseHoles {
        app.edit_mode
            .selected_faces_for_layer(request.layer_id)
            .filter(|selection| selection.selected_count() > 0)
    } else if request.action == LayerContextAction::InvertNormals {
        app.edit_mode
            .selected_faces_for_layer(request.layer_id)
            .filter(|selection| selection.selected_count() > 0)
    } else if request.action == LayerContextAction::SmoothSelection {
        app.edit_mode
            .selected_faces_for_layer(request.layer_id)
            .filter(|selection| selection.selected_count() > 0)
    } else {
        None
    };
    if matches!(
        request.action,
        LayerContextAction::CloseHoles | LayerContextAction::SmoothSelection
    ) && selection.is_none()
    {
        app.status_message = Some("Select mesh faces first".to_string());
        return LayerContextApply::default();
    }

    let Some(token) = app.edit_mode.begin_layer_edit(entry, command) else {
        app.status_message = Some("Layer edit already in progress".to_string());
        return LayerContextApply::default();
    };

    // Close Holes is always explicitly selection-scoped in the interactive
    // app. Whole-mesh repair remains an internal/CLI operation instead.
    let close_holes_limit_mm = None;

    match apply_layer_mesh_edit_action_with_limit(
        scene,
        request,
        selection.as_ref(),
        close_holes_limit_mm,
    ) {
        Ok((apply, report)) => {
            if apply.scene_changed {
                app.mark_mesh_edits_unsaved(request.layer_id);
                let _ = app.edit_mode.finish_layer_edit_success(token);
                let status = close_holes_aware_status(
                    &layer_label,
                    request.action,
                    report.as_ref(),
                    close_holes_limit_mm,
                    true,
                );
                app.status_message = Some(with_undoable_note(app, status));
            } else {
                let _ = app.edit_mode.finish_layer_edit_noop(token);
                app.status_message = Some(close_holes_aware_status(
                    &layer_label,
                    request.action,
                    report.as_ref(),
                    close_holes_limit_mm,
                    false,
                ));
            }
            apply
        }
        Err(error) => {
            let summary = format!("Could not edit layer: {error}");
            let _ = app
                .edit_mode
                .finish_layer_edit_error(token, error.to_string());
            app.status_message = Some(summary.clone());
            app.app_error = Some(AppErrorDialog {
                title: "Could not edit layer".to_string(),
                summary,
                details: format!("Layer edit failed\n\nLayer:\n{layer_label}\n\nError:\n{error:#}"),
            });
            LayerContextApply::default()
        }
    }
}

pub(super) fn edit_command_for_layer_action(action: LayerContextAction) -> Option<EditModeCommand> {
    match action {
        LayerContextAction::CloseHoles => Some(EditModeCommand::CloseHoles),
        LayerContextAction::SmoothSelection => Some(EditModeCommand::SmoothSelection),
        LayerContextAction::InvertNormals => Some(EditModeCommand::InvertNormals),
        LayerContextAction::DeleteSelectedFaces => Some(EditModeCommand::DeleteSelectedFaces),
        LayerContextAction::CropToSelectedFaces => Some(EditModeCommand::CropToSelectedFaces),
        LayerContextAction::CutSelectionToNewLayer => Some(EditModeCommand::CutSelectionToNewLayer),
        LayerContextAction::SeparateSelectedComponents => {
            Some(EditModeCommand::SeparateSelectedComponents)
        }
        _ => None,
    }
}

/// Selection-only convenience wrapper (no mm budget). Used by the layer-edit
/// tests; production always routes through
/// [`apply_layer_mesh_edit_action_with_limit`].
#[cfg(test)]
pub(super) fn apply_layer_mesh_edit_action(
    scene: &mut Scene,
    request: LayerContextRequest,
    selection: Option<&FaceSelection>,
) -> Result<(LayerContextApply, Option<MeshEditReport>), CoreError> {
    apply_layer_mesh_edit_action_with_limit(scene, request, selection, None)
}

/// The layer action executor plus the Close Holes mm budget.
/// `close_holes_limit_mm` is `None` for every other action (and for callers
/// that never opt in), leaving their behaviour unchanged.
pub(super) fn apply_layer_mesh_edit_action_with_limit(
    scene: &mut Scene,
    request: LayerContextRequest,
    selection: Option<&FaceSelection>,
    close_holes_limit_mm: Option<f32>,
) -> Result<(LayerContextApply, Option<MeshEditReport>), CoreError> {
    let LayerContextRequest {
        index,
        layer_id,
        action,
    } = request;
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return Ok((LayerContextApply::default(), None));
    };
    if entry.id() != layer_id {
        return Ok((LayerContextApply::default(), None));
    }

    let edited = match action {
        LayerContextAction::CloseHoles => {
            let Some(selection) = selection else {
                return Ok((LayerContextApply::default(), None));
            };
            close_holes_in_mesh(&entry.mesh, selection, close_holes_limit_mm)?
        }
        LayerContextAction::InvertNormals => invert_mesh_orientation(&entry.mesh, selection)?,
        LayerContextAction::SmoothSelection => {
            let Some(selection) = selection else {
                return Ok((LayerContextApply::default(), None));
            };
            smooth_selected_faces_in_mesh(&entry.mesh, selection)?
        }
        _ => return Ok((LayerContextApply::default(), None)),
    };

    // Content no-op (nothing filled / nothing dropped / nothing moved): leave
    // the mesh alone so the caller reports an honest status instead of a
    // phantom edit. The report still rides along so a no-op can say so.
    let content_changed = match action {
        LayerContextAction::CloseHoles => edited.report.filled_holes > 0,
        LayerContextAction::SmoothSelection => edited.report.moved_vertices > 0,
        _ => true,
    };
    if !content_changed {
        return Ok((LayerContextApply::default(), Some(edited.report)));
    }

    entry.mesh = edited.mesh;
    Ok((structural_scene_apply(), Some(edited.report)))
}

/// Run the canonical Close Holes kernel used by both the layer menu and the
/// scene-wide Mesh Editor action. Keeping the options here prevents the two
/// entry points from quietly drifting into different repair behaviour.
pub(super) fn close_holes_in_mesh(
    mesh: &Mesh,
    selection: &FaceSelection,
    close_holes_limit_mm: Option<f32>,
) -> Result<CoreMeshEditResult, CoreError> {
    fill_selected_holes_in_mesh(mesh, selection, close_holes_options(close_holes_limit_mm))
}

/// Interactive Close Holes options. With the mm slider set (`Some`), the mm
/// perimeter is the real gate and the edge count is only a safety ceiling;
/// without it (kernel/tests default) fall back to the plain edge cap so
/// behaviour is unchanged for callers that never opt in.
fn close_holes_options(close_holes_limit_mm: Option<f32>) -> MeshEditOptions {
    match close_holes_limit_mm {
        Some(limit_mm) => MeshEditOptions {
            compact_vertices: true,
            max_boundary_loop: CLOSE_HOLES_EDGE_CEILING,
            max_rim_perimeter_mm: Some(limit_mm),
            // Heal the cut line first: a digitally extracted tooth leaves a
            // jagged rim (needle/lone triangles, near-coincident seam verts) —
            // clean it so the socket closes instead of reporting dozens of
            // "damaged" nick rims. The exocad behavior.
            heal_boundary_rims: true,
            ..MeshEditOptions::default()
        },
        None => MeshEditOptions {
            compact_vertices: true,
            heal_boundary_rims: true,
            ..MeshEditOptions::default()
        },
    }
}

/// Route the status line: Close Holes gets the mm-aware phrasing, every other
/// layer action keeps the shared status helpers untouched.
fn close_holes_aware_status(
    layer_label: &str,
    action: LayerContextAction,
    report: Option<&MeshEditReport>,
    close_holes_limit_mm: Option<f32>,
    changed: bool,
) -> String {
    if action == LayerContextAction::CloseHoles {
        return close_holes_status(layer_label, report, close_holes_limit_mm, changed);
    }
    if changed {
        layer_edit_status(layer_label, action, report)
    } else {
        layer_edit_noop_status(layer_label, action)
    }
}

/// Honest selection-scoped Close Holes status. Partial success is reported as it
/// happens (some rims close while others are skipped), skips name the mm budget
/// so the operator knows why a rim stayed open, and it must NOT claim "no holes"
/// when loops were found but refused.
fn close_holes_status(
    layer_label: &str,
    report: Option<&MeshEditReport>,
    close_holes_limit_mm: Option<f32>,
    changed: bool,
) -> String {
    let (filled, border, oversize, damaged, healed) = report.map_or((0, 0, 0, 0, 0), |report| {
        (
            report.filled_holes,
            report.skipped_border_rims,
            report.skipped_oversize_rims,
            report.skipped_damaged_rims,
            report.healed_rims,
        )
    });
    let mut segments: Vec<String> = Vec::new();
    if healed > 0 {
        // Pre-cleaning healed the jagged cut line (dropped needle/lone
        // triangles, welded seam vertices) before capping — the operator sees
        // why the socket closed cleanly instead of leaving nick rims.
        let noun = if healed == 1 { "nick" } else { "nicks" };
        segments.push(format!("{healed} {noun} healed"));
    }
    if border > 0 {
        segments.push(if border == 1 {
            "scan border kept open".to_string()
        } else {
            format!("{border} border rims kept open")
        });
    }
    if oversize > 0 {
        let noun = if oversize == 1 { "hole" } else { "holes" };
        segments.push(match close_holes_limit_mm {
            Some(limit_mm) => format!("{oversize} {noun} over the {limit_mm:.0} mm limit"),
            None => format!("{oversize} {noun} too large"),
        });
    }
    if damaged > 0 {
        let noun = if damaged == 1 { "rim" } else { "rims" };
        segments.push(format!("{damaged} damaged {noun} skipped"));
    }

    if !changed {
        return if segments.is_empty() {
            format!("No holes to close: {layer_label}")
        } else {
            format!("{}, none closed: {layer_label}", segments.join(", "))
        };
    }
    let closed = if filled == 1 {
        "Closed 1 hole".to_string()
    } else {
        format!("Closed {filled} holes")
    };
    if segments.is_empty() {
        format!("{closed}: {layer_label}")
    } else {
        format!("{closed} ({}): {layer_label}", segments.join(", "))
    }
}

/// Status for a whole-mesh op (other than Close Holes) that changed nothing.
fn layer_edit_noop_status(layer_label: &str, _action: LayerContextAction) -> String {
    format!("No changes: {layer_label}")
}

pub(super) fn layer_edit_status(
    layer_label: &str,
    action: LayerContextAction,
    _report: Option<&MeshEditReport>,
) -> String {
    let action_label = match action {
        LayerContextAction::InvertNormals => "Inverted normals",
        LayerContextAction::DeleteSelectedFaces => "Deleted selected faces",
        LayerContextAction::CropToSelectedFaces => "Cropped to selection",
        LayerContextAction::CutSelectionToNewLayer => "Cut selection to new layer",
        LayerContextAction::SeparateSelectedComponents => "Separated selected components",
        LayerContextAction::SmoothSelection => "Smoothed selection",
        _ => "Edited layer",
    };
    format!("{action_label}: {layer_label}")
}

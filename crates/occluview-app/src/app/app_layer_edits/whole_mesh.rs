//! Layer mesh ops (Close Holes / Keep Largest / Invert Normals): dispatch with
//! content no-ops that leave the mesh untouched, plus the operator status lines.

use super::super::{
    EditModeCommand, LayerContextAction, LayerContextApply, LayerContextRequest, OccluViewApp,
    PathBuf, Scene,
};
use super::resolve_layer;
use super::structural::structural_scene_apply;
use occluview_core::{
    fill_selected_holes_in_mesh, invert_mesh_orientation, CoreError, CoreMeshEditResult,
    FaceSelection, Mesh, MeshEditOptions, MeshEditReport,
};
use std::sync::Arc;

/// Generous edge ceiling for the interactive Close Holes action. With the mm
/// perimeter slider doing the real limiting, the edge count is only a safety
/// valve, so it must not spuriously refuse a legitimate hole on a densely
/// triangulated scan.
///
/// The kernel owns the number, and a private copy here defeats it: the
/// selection gate takes `max(options.max_boundary_loop, kernel constant)`, so
/// lowering the kernel value against a stale local copy changes nothing in the
/// shipped product.
use occluview_core::CLOSE_HOLES_EDGE_CEILING;

pub(super) fn apply_layer_mesh_edit_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request, &app.ui.locale) else {
        return LayerContextApply::default();
    };
    let Some(command) = edit_command_for_layer_action(request.action) else {
        return LayerContextApply::default();
    };

    let selection = if matches!(
        request.action,
        LayerContextAction::CloseHoles | LayerContextAction::InvertNormals
    ) {
        app.document
            .edit_mode
            .selected_faces_for_layer(request.layer_id)
            .filter(|selection| selection.selected_count() > 0)
    } else {
        None
    };
    if request.action == LayerContextAction::CloseHoles && selection.is_none() {
        app.ui.status_message = Some(app.ui.locale.tr("edit-select-faces-first"));
        return LayerContextApply::default();
    }

    let Some(token) = app.document.edit_mode.begin_layer_edit(entry, command) else {
        return super::refuse_busy_layer_edit(&mut app.ui);
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
                let status = close_holes_aware_status(
                    &layer_label,
                    request.action,
                    report.as_ref(),
                    close_holes_limit_mm,
                    true,
                    &app.ui.locale,
                );
                super::commit_layer_edit(
                    &mut app.document,
                    &mut app.ui,
                    token,
                    request.layer_id,
                    super::LayerEditResolution::Applied {
                        changed: true,
                        status,
                    },
                );
            } else {
                let status = close_holes_aware_status(
                    &layer_label,
                    request.action,
                    report.as_ref(),
                    close_holes_limit_mm,
                    false,
                    &app.ui.locale,
                );
                super::commit_layer_edit(
                    &mut app.document,
                    &mut app.ui,
                    token,
                    request.layer_id,
                    super::LayerEditResolution::Applied {
                        changed: false,
                        status,
                    },
                );
            }
            apply
        }
        Err(error) => {
            super::commit_layer_edit(
                &mut app.document,
                &mut app.ui,
                token,
                request.layer_id,
                super::LayerEditResolution::Failed { error, layer_label },
            );
            LayerContextApply::default()
        }
    }
}

pub(super) fn edit_command_for_layer_action(action: LayerContextAction) -> Option<EditModeCommand> {
    match action {
        LayerContextAction::CloseHoles => Some(EditModeCommand::CloseHoles),
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
        _ => return Ok((LayerContextApply::default(), None)),
    };

    // Content no-op (nothing filled / nothing dropped / nothing moved): leave
    // the mesh alone so the caller reports a no-op status instead of recording
    // an edit. The report still rides along so a no-op can say so.
    let content_changed = match action {
        LayerContextAction::CloseHoles => edited.report.filled_holes > 0,
        _ => true,
    };
    if !content_changed {
        return Ok((LayerContextApply::default(), Some(edited.report)));
    }

    entry.mesh = Arc::new(edited.mesh);
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
            // "damaged" nick rims.
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
///
/// Six cohesive dispatch inputs (label, action, report, limit, outcome,
/// locale); a struct would only be built to be destructured again.
#[expect(clippy::too_many_arguments)]
fn close_holes_aware_status(
    layer_label: &str,
    action: LayerContextAction,
    report: Option<&MeshEditReport>,
    close_holes_limit_mm: Option<f32>,
    changed: bool,
    locale: &crate::i18n::LocaleManager,
) -> String {
    if action == LayerContextAction::CloseHoles {
        return close_holes_status(layer_label, report, close_holes_limit_mm, changed, locale);
    }
    if changed {
        layer_edit_status(layer_label, action, report, locale)
    } else {
        layer_edit_noop_status(layer_label, locale)
    }
}

/// Selection-scoped Close Holes status. Partial success is reported as it
/// happens (some rims close while others are skipped), skips name the mm budget
/// so the operator knows why a rim stayed open, and it must not claim "no holes"
/// when loops were found but refused.
fn close_holes_status(
    layer_label: &str,
    report: Option<&MeshEditReport>,
    close_holes_limit_mm: Option<f32>,
    changed: bool,
    locale: &crate::i18n::LocaleManager,
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
        segments.push(locale.tr_plural("holes-seg-healed", &[], &[("n", healed)]));
    }
    if border > 0 {
        segments.push(locale.tr("holes-seg-border"));
    }
    if oversize > 0 {
        segments.push(match close_holes_limit_mm {
            Some(limit_mm) => locale.tr_with(
                "holes-seg-oversize-limit",
                &[
                    ("n", &oversize.to_string()),
                    ("limit", &format!("{limit_mm:.0}")),
                ],
            ),
            None => locale.tr_plural("holes-seg-oversize", &[], &[("n", oversize)]),
        });
    }
    if damaged > 0 {
        segments.push(locale.tr_plural("holes-seg-damaged", &[], &[("n", damaged)]));
    }

    if !changed {
        return if segments.is_empty() {
            locale.tr_with("holes-nothing", &[("layer", layer_label)])
        } else {
            locale.tr_with(
                "holes-partial",
                &[("segments", &segments.join(", ")), ("layer", layer_label)],
            )
        };
    }
    let closed = locale.tr_plural("holes-closed", &[], &[("filled", filled)]);
    if segments.is_empty() {
        locale.tr_with(
            "holes-closed-detail",
            &[("closed", &closed), ("layer", layer_label)],
        )
    } else {
        locale.tr_with(
            "holes-closed-segments",
            &[
                ("closed", &closed),
                ("segments", &segments.join(", ")),
                ("layer", layer_label),
            ],
        )
    }
}

/// Status for a whole-mesh op (other than Close Holes) that changed nothing.
fn layer_edit_noop_status(layer_label: &str, locale: &crate::i18n::LocaleManager) -> String {
    locale.tr_with("edit-no-changes", &[("layer", layer_label)])
}

pub(super) fn layer_edit_status(
    layer_label: &str,
    action: LayerContextAction,
    _report: Option<&MeshEditReport>,
    locale: &crate::i18n::LocaleManager,
) -> String {
    // Action labels render through the `batchedit-*` catalog keys.
    let action_key = match action {
        LayerContextAction::InvertNormals => "batchedit-invert",
        LayerContextAction::DeleteSelectedFaces => "batchedit-delete",
        LayerContextAction::CropToSelectedFaces => "batchedit-crop",
        LayerContextAction::CutSelectionToNewLayer => "batchedit-cut",
        LayerContextAction::SeparateSelectedComponents => "batchedit-separate",
        _ => "batchedit-edited",
    };
    locale.tr_with(
        "edit-applied-status",
        &[("action", &locale.tr(action_key)), ("layer", layer_label)],
    )
}

// Batch action labels render through the `batch-*` catalog keys.
pub(crate) fn batch_action_label(
    action: LayerContextAction,
    locale: &crate::i18n::LocaleManager,
) -> String {
    let key = match action {
        LayerContextAction::CloseHoles => "batch-close-holes",
        LayerContextAction::DeleteSelectedFaces => "batch-delete",
        LayerContextAction::CropToSelectedFaces => "batch-crop",
        LayerContextAction::CutSelectionToNewLayer => "batch-cut",
        LayerContextAction::SeparateSelectedComponents => "batch-separate",
        _ => "batch-edited",
    };
    locale.tr(key)
}

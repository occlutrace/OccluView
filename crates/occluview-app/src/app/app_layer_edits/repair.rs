//! One-click Repair mesh executor: runs the full occlu-mesh-edit repair
//! pipeline (weld / slivers / duplicates / non-manifold / orientation /
//! debris / pinholes) on a whole layer as ONE undo step, with honest no-op
//! semantics and a per-pass status line that reports only what happened.

use super::super::{
    EditModeCommand, LayerContextApply, LayerContextRequest, OccluViewApp, PathBuf, Scene,
};
use super::resolve_layer;
use super::structural::structural_scene_apply;
use occluview_core::{repair_mesh_in_mesh, CoreError, RepairOptions, RepairReport};
use std::sync::Arc;

/// What one repair run did to the requested layer.
pub(super) enum LayerRepairOutcome {
    /// The request no longer matches the live scene; nothing was touched.
    Stale,
    /// The pipeline found nothing to fix; the mesh is untouched.
    Clean(RepairReport),
    /// The layer's mesh was replaced with the repaired result.
    Repaired(RepairReport),
}

pub(super) fn apply_layer_repair_action_with_status(
    app: &mut OccluViewApp,
    scene: &mut Scene,
    paths: &[PathBuf],
    request: LayerContextRequest,
) -> LayerContextApply {
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request, &app.ui.locale) else {
        return LayerContextApply::default();
    };
    let Some(token) = app
        .document
        .edit_mode
        .begin_layer_edit(entry, EditModeCommand::RepairMesh)
    else {
        return super::refuse_busy_layer_edit(&mut app.ui);
    };

    // Repair is a whole-mesh operation by design (like Keep Largest Island):
    // any live face selection is ignored, the pipeline decides what is damage.
    match apply_layer_repair_action(scene, request) {
        Ok(LayerRepairOutcome::Repaired(report)) => {
            let status = repaired_status(&layer_label, &report, &app.ui.locale);
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
            // The toast above is the glance; the card is the detail — one human
            // line per non-zero pass, kept open until the operator dismisses it.
            app.ui.repair_report.present(&layer_label, report);
            structural_scene_apply()
        }
        Ok(LayerRepairOutcome::Clean(report)) => {
            // Honest no-op: mesh untouched, snapshot discarded, session not
            // dirtied — but the operator still hears about open rims left.
            let status = clean_status(&layer_label, &report, &app.ui.locale);
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
            // Positive confirmation, matching the convention dental CAD
            // software uses: a clean scan still gets a card ("Nothing to
            // repair — mesh is clean"), not silence.
            app.ui.repair_report.present(&layer_label, report);
            LayerContextApply::default()
        }
        Ok(LayerRepairOutcome::Stale) => {
            let _ = app.document.edit_mode.finish_layer_edit_noop(token);
            LayerContextApply::default()
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

/// Run the repair pipeline against the live scene entry. The kernel's
/// `changed_content` verdict decides between a real edit and a content no-op,
/// so a clean mesh is never replaced (and never re-uploaded to the GPU).
pub(super) fn apply_layer_repair_action(
    scene: &mut Scene,
    request: LayerContextRequest,
) -> Result<LayerRepairOutcome, CoreError> {
    let Some(entry) = scene.meshes_mut().get_mut(request.index) else {
        return Ok(LayerRepairOutcome::Stale);
    };
    if entry.id() != request.layer_id {
        return Ok(LayerRepairOutcome::Stale);
    }

    let result = repair_mesh_in_mesh(&entry.mesh, RepairOptions::default())?;
    if !result.report.changed_content() {
        return Ok(LayerRepairOutcome::Clean(result.report));
    }
    entry.mesh = Arc::new(result.mesh);
    Ok(LayerRepairOutcome::Repaired(result.report))
}

/// "Repaired {layer}: ..." listing ONLY the non-zero pass counts, plus the
/// skipped-rim warning tail when the fill pass refused non-simple rims.
pub(super) fn repaired_status(
    layer_label: &str,
    report: &RepairReport,
    locale: &crate::i18n::LocaleManager,
) -> String {
    let mut parts = repair_parts(report, locale);
    let skipped = report.warnings.len();
    if skipped > 0 {
        parts.push(locale.tr_plural("repair-toast-skipped", &[], &[("count", skipped)]));
    }
    locale.tr_with(
        "repair-toast-done",
        &[("layer", layer_label), ("parts", &parts.join(", "))],
    )
}

/// Status for a mesh the pipeline had nothing to fix on. Open rims larger
/// than the pinhole cap are the scan's natural boundary — informational, but
/// the operator deserves to hear they exist.
pub(super) fn clean_status(
    layer_label: &str,
    report: &RepairReport,
    locale: &crate::i18n::LocaleManager,
) -> String {
    let rims = report.open_rims_left;
    if rims > 0 {
        locale.tr_plural(
            "repair-toast-clean-rims",
            &[("layer", layer_label)],
            &[("count", rims)],
        )
    } else {
        locale.tr_with("repair-toast-clean", &[("layer", layer_label)])
    }
}

/// One phrase per non-zero pass count, in pipeline order. Every counter that
/// can set `changed_content` is covered (debris triangles ride along with
/// their components), so a repaired status never comes out empty.
fn repair_parts(report: &RepairReport, locale: &crate::i18n::LocaleManager) -> Vec<String> {
    let entries: [(usize, &str); 10] = [
        (report.welded_vertices, "repair-toast-welded"),
        (report.removed_degenerate_triangles, "repair-toast-slivers"),
        (
            report.removed_duplicate_triangles,
            "repair-toast-duplicate-faces",
        ),
        (report.split_nonmanifold_edges, "repair-toast-nonmanifold"),
        (report.split_bowtie_vertices, "repair-toast-bowtie"),
        (report.reoriented_triangles, "repair-toast-reoriented"),
        (report.flipped_components, "repair-toast-flipped"),
        (report.removed_debris_components, "repair-toast-debris"),
        (report.filled_holes, "repair-toast-pinholes"),
        (report.removed_unreferenced_vertices, "repair-toast-unused"),
    ];
    entries
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|&(count, key)| locale.tr_plural(key, &[], &[("count", count)]))
        .collect()
}

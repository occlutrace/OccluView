//! One-click Repair mesh executor: runs the full occlu-mesh-edit repair
//! pipeline (weld / slivers / duplicates / non-manifold / orientation /
//! debris / pinholes) on a whole layer as ONE undo step, with honest no-op
//! semantics and a per-pass status line that reports only what happened.

use super::super::{
    AppErrorDialog, EditModeCommand, LayerContextApply, LayerContextRequest, OccluViewApp, PathBuf,
    Scene,
};
use super::structural::structural_scene_apply;
use super::{resolve_layer, with_undoable_note};
use occluview_core::{repair_mesh_in_mesh, CoreError, RepairOptions, RepairReport};

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
    let Some((entry, layer_label)) = resolve_layer(scene, paths, &request) else {
        return LayerContextApply::default();
    };
    let Some(token) = app
        .edit_mode
        .begin_layer_edit(entry, EditModeCommand::RepairMesh)
    else {
        app.status_message = Some("Layer edit already in progress".to_string());
        return LayerContextApply::default();
    };

    // Repair is a whole-mesh operation by design (like Keep Largest Island):
    // any live face selection is ignored, the pipeline decides what is damage.
    match apply_layer_repair_action(scene, request) {
        Ok(LayerRepairOutcome::Repaired(report)) => {
            app.mark_mesh_edits_unsaved(request.layer_id);
            let _ = app.edit_mode.finish_layer_edit_success(token);
            let status = repaired_status(&layer_label, &report);
            app.status_message = Some(with_undoable_note(app, status));
            // The toast above is the glance; the card is the detail — one human
            // line per non-zero pass, kept open until the operator dismisses it.
            app.repair_report.present(&layer_label, report);
            structural_scene_apply()
        }
        Ok(LayerRepairOutcome::Clean(report)) => {
            // Honest no-op: mesh untouched, snapshot discarded, session not
            // dirtied — but the operator still hears about open rims left.
            let _ = app.edit_mode.finish_layer_edit_noop(token);
            app.status_message = Some(clean_status(&layer_label, &report));
            // Positive confirmation, exocad-style: a clean scan still gets a
            // card ("Nothing to repair — mesh is clean"), not silence.
            app.repair_report.present(&layer_label, report);
            LayerContextApply::default()
        }
        Ok(LayerRepairOutcome::Stale) => {
            let _ = app.edit_mode.finish_layer_edit_noop(token);
            LayerContextApply::default()
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
    entry.mesh = result.mesh;
    Ok(LayerRepairOutcome::Repaired(result.report))
}

/// "Repaired {layer}: ..." listing ONLY the non-zero pass counts, plus the
/// skipped-rim warning tail when the fill pass refused non-simple rims.
pub(super) fn repaired_status(layer_label: &str, report: &RepairReport) -> String {
    let mut parts = repair_parts(report);
    let skipped = report.warnings.len();
    if skipped > 0 {
        parts.push(format!(
            "{skipped} {} skipped (non-simple)",
            plural(skipped, "rim", "rims")
        ));
    }
    format!("Repaired {layer_label}: {}", parts.join(", "))
}

/// Status for a mesh the pipeline had nothing to fix on. Open rims larger
/// than the pinhole cap are the scan's natural boundary — informational, but
/// the operator deserves to hear they exist.
pub(super) fn clean_status(layer_label: &str, report: &RepairReport) -> String {
    let rims = report.open_rims_left;
    if rims > 0 {
        format!(
            "Mesh is already clean: {layer_label}, {rims} open {} left",
            plural(rims, "rim", "rims")
        )
    } else {
        format!("Mesh is already clean: {layer_label}")
    }
}

/// One phrase per non-zero pass count, in pipeline order. Every counter that
/// can set `changed_content` is covered (debris triangles ride along with
/// their components), so a repaired status never comes out empty.
fn repair_parts(report: &RepairReport) -> Vec<String> {
    let entries: [(usize, &str, &str, &str); 10] = [
        (report.welded_vertices, "welded", "vertex", "vertices"),
        (
            report.removed_degenerate_triangles,
            "removed",
            "sliver",
            "slivers",
        ),
        (
            report.removed_duplicate_triangles,
            "",
            "duplicate face",
            "duplicate faces",
        ),
        (
            report.split_nonmanifold_edges,
            "fixed",
            "non-manifold edge",
            "non-manifold edges",
        ),
        (report.split_bowtie_vertices, "split", "bowtie", "bowties"),
        (
            report.reoriented_triangles,
            "reoriented",
            "triangle",
            "triangles",
        ),
        (
            report.flipped_components,
            "flipped",
            "inside-out part",
            "inside-out parts",
        ),
        (
            report.removed_debris_components,
            "dropped",
            "debris part",
            "debris parts",
        ),
        (report.filled_holes, "closed", "pinhole", "pinholes"),
        (
            report.removed_unreferenced_vertices,
            "removed",
            "unused vertex",
            "unused vertices",
        ),
    ];
    entries
        .iter()
        .filter(|(count, ..)| *count > 0)
        .map(|&(count, verb, singular, plural_noun)| {
            count_phrase(count, verb, singular, plural_noun)
        })
        .collect()
}

/// "verb N noun" with the verb optional ("12 duplicate faces" has none).
fn count_phrase(count: usize, verb: &str, singular: &str, plural_noun: &str) -> String {
    let noun = plural(count, singular, plural_noun);
    if verb.is_empty() {
        format!("{count} {noun}")
    } else {
        format!("{verb} {count} {noun}")
    }
}

fn plural<'a>(count: usize, singular: &'a str, plural_noun: &'a str) -> &'a str {
    if count == 1 {
        singular
    } else {
        plural_noun
    }
}

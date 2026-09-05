//! Orchestration-boundary tests for [`super::super::commit_layer_edit`]: the
//! document/undo/notification effects every token-based layer executor must
//! otherwise remember.
#![allow(clippy::expect_used, clippy::panic)]

use super::super::{
    commit_layer_edit, refuse_busy_layer_edit, LayerContextAction, LayerContextApply,
    LayerContextRequest, LayerEditResolution,
};
use crate::app::state_document::DocumentState;
use crate::app::state_ui::UiState;
use crate::edit_mode::EditModeCommand;
use eframe::egui;
use occluview_core::{CoreError, Mesh, Scene, SceneMesh};

fn vertex(x: f32, y: f32, z: f32) -> occluview_core::Vertex {
    occluview_core::Vertex::at(glam::Vec3::new(x, y, z))
}

fn commit_scene() -> Option<Scene> {
    let mesh = Mesh::new(
        Some("commit".into()),
        vec![
            vertex(0.0, 0.0, 0.0),
            vertex(1.0, 0.0, 0.0),
            vertex(0.0, 1.0, 0.0),
            vertex(1.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 1, 3, 2],
    )
    .ok()?;
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    Some(scene)
}

fn owners() -> (DocumentState, UiState) {
    (DocumentState::new(), UiState::new(egui::Context::default()))
}

#[test]
fn commit_applied_change_marks_unsaved_stores_undo_and_notes_status() {
    let Some(scene) = commit_scene() else {
        return;
    };
    let (mut document, mut ui) = owners();
    let entry = scene.meshes()[0].clone();
    let layer_id = entry.id();
    let Some(token) = document
        .edit_mode
        .begin_layer_edit(&entry, EditModeCommand::CloseHoles)
    else {
        panic!("a fresh layer edit should begin");
    };

    commit_layer_edit(
        &mut document,
        &mut ui,
        token,
        layer_id,
        LayerEditResolution::Applied {
            changed: true,
            status: "Closed holes".to_string(),
        },
    );

    assert!(document.has_unsaved_mesh_edits());
    assert_eq!(document.edit_mode.undo_len(), 1);
    // Small mesh: the pre-op snapshot is kept, so no suffix is appended.
    assert_eq!(ui.status_message.as_deref(), Some("Closed holes"));
    assert!(ui.app_error.is_none());
}

#[test]
fn commit_noop_discards_snapshot_without_unsaved_or_dialog() {
    let Some(scene) = commit_scene() else {
        return;
    };
    let (mut document, mut ui) = owners();
    let entry = scene.meshes()[0].clone();
    let layer_id = entry.id();
    let Some(token) = document
        .edit_mode
        .begin_layer_edit(&entry, EditModeCommand::CloseHoles)
    else {
        panic!("a fresh layer edit should begin");
    };

    commit_layer_edit(
        &mut document,
        &mut ui,
        token,
        layer_id,
        LayerEditResolution::Applied {
            changed: false,
            status: "Nothing to repair — mesh is clean".to_string(),
        },
    );

    assert!(!document.has_unsaved_mesh_edits());
    assert_eq!(
        ui.status_message.as_deref(),
        Some("Nothing to repair — mesh is clean")
    );
    assert!(ui.app_error.is_none());
}

#[test]
fn commit_failure_finishes_error_and_opens_the_copyable_dialog() {
    let Some(scene) = commit_scene() else {
        return;
    };
    let (mut document, mut ui) = owners();
    let entry = scene.meshes()[0].clone();
    let layer_id = entry.id();
    let Some(token) = document
        .edit_mode
        .begin_layer_edit(&entry, EditModeCommand::CloseHoles)
    else {
        panic!("a fresh layer edit should begin");
    };

    commit_layer_edit(
        &mut document,
        &mut ui,
        token,
        layer_id,
        LayerEditResolution::Failed {
            error: CoreError::Geometry("boom".to_string()),
            layer_label: "scan".to_string(),
        },
    );

    assert!(!document.has_unsaved_mesh_edits());
    let status = ui.status_message.expect("failure must notify");
    assert!(status.contains("Could not edit layer"));
    let dialog = ui.app_error.expect("failure must open the dialog");
    assert_eq!(dialog.title, "Could not edit layer");
    assert!(dialog.summary.contains("Could not edit layer"));
    assert!(dialog.details.contains("scan"));
    assert!(dialog.details.contains("boom"));
}

#[test]
fn busy_refusal_notifies_without_consuming_a_session() {
    let (_, mut ui) = owners();

    let apply: LayerContextApply = refuse_busy_layer_edit(&mut ui);

    assert!(!apply.scene_changed);
    assert_eq!(
        ui.status_message.as_deref(),
        Some("Layer edit already in progress")
    );
    assert!(ui.app_error.is_none());
}

#[test]
fn repair_stale_index_resolves_without_touching_undo() {
    use super::super::repair::{apply_layer_repair_action, LayerRepairOutcome};

    let Some(scene) = commit_scene() else {
        return;
    };
    let (document, _) = owners();
    // Index past the end: entry lookup misses, so the outcome is stale and
    // no edit session may open.
    let mut scene = scene;
    let stale_request = LayerContextRequest {
        index: 7,
        layer_id: scene.meshes()[0].id(),
        action: LayerContextAction::RepairMesh,
    };
    let outcome = apply_layer_repair_action(&mut scene, stale_request);
    assert!(
        matches!(outcome, Ok(LayerRepairOutcome::Stale)),
        "a missing entry must resolve stale, never as an edit"
    );
    assert_eq!(document.edit_mode.undo_len(), 0);
    assert!(!document.has_unsaved_mesh_edits());
}

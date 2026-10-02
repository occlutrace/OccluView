#![allow(clippy::float_cmp)]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::app::workspace::commands::{LayerIds, SplitSide, TransferDestination};
use crate::app::workspace::history::{HistoryDirection, HistoryStepKind};
use crate::app::workspace::id::{PaneId, SceneKey};
use crate::app::workspace::layout::WorkspaceLayout;
use crate::app::workspace::state::SceneSession;
use crate::app::OccluViewApp;
use crate::edit_mode::{BusyFinish, EditModeCommand};
use eframe::egui;
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId};
use std::path::PathBuf;
use std::sync::Arc;

fn app_with_layer() -> (OccluViewApp, SceneKey, SceneMeshId, Arc<Mesh>) {
    let mut app = OccluViewApp::new_for_tests(egui::Context::default());
    let source_key = app.workspace.scenes[0].key;
    let mesh = Arc::new(Mesh::empty());
    let layer = SceneMesh::new(Arc::clone(&mesh));
    let layer_id = layer.id();
    let mut scene = Scene::new();
    scene.add(layer);
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    (app, source_key, layer_id, mesh)
}

fn reserved_destination(app: &mut OccluViewApp) -> (SceneKey, PaneId) {
    (
        app.workspace.ids.allocate_scene().unwrap(),
        app.workspace.ids.allocate_pane_id().unwrap(),
    )
}

fn move_to_new_scene(
    app: &mut OccluViewApp,
    source: SceneKey,
    layer: SceneMeshId,
) -> (SceneKey, PaneId) {
    let (destination, pane) = reserved_destination(app);
    app.transfer_layers(
        source,
        TransferDestination::CreateBeside {
            scene: destination,
            pane,
            side: SplitSide::Right,
        },
        LayerIds::one(layer),
    )
    .unwrap();
    (destination, pane)
}

#[test]
#[allow(clippy::too_many_lines)]
fn save_after_transfer_keeps_layer_state_through_undo_redo() {
    let (mut app, source, layer_id, mesh) = app_with_layer();
    let (reserved_key, pane) = reserved_destination(&mut app);
    let source_document = &mut app.workspace.scenes[0].document;
    source_document.current_paths = vec![PathBuf::from("source-scan.stl")];
    source_document.unsaved_edit_layer_ids.insert(layer_id);
    source_document.hidden_layer_stack.push(layer_id);
    source_document
        .translucent_layer_restore
        .insert(layer_id, 0.35);
    app.ui.popup_open_at_frame_start = true;

    app.transfer_layers(
        source,
        TransferDestination::CreateBeside {
            scene: reserved_key,
            pane,
            side: SplitSide::Right,
        },
        LayerIds::one(layer_id),
    )
    .unwrap();

    assert_eq!(app.workspace.scenes.len(), 2);
    assert_eq!(app.workspace.scenes[1].key, reserved_key);
    let moved = app.workspace.scenes[1]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone();
    assert_eq!(moved.id(), layer_id);
    assert!(Arc::ptr_eq(&moved.mesh, &mesh));
    assert_eq!(
        app.workspace.scenes[1].document.current_paths,
        [PathBuf::from("source-scan.stl")]
    );
    assert!(app.workspace.scenes[1]
        .document
        .unsaved_edit_layer_ids
        .contains(&layer_id));
    assert_eq!(
        app.workspace.scenes[1].document.hidden_layer_stack,
        [layer_id]
    );
    assert_eq!(
        app.workspace.scenes[1]
            .document
            .translucent_layer_restore
            .get(&layer_id),
        Some(&0.35)
    );

    let moved_document = &mut app.workspace.scenes[1].document;
    moved_document.unsaved_edit_layer_ids.remove(&layer_id);
    moved_document.hidden_layer_stack.clear();
    moved_document
        .translucent_layer_restore
        .insert(layer_id, 0.8);
    let moved_layer = moved_document
        .live_scene_mut()
        .unwrap()
        .meshes_mut()
        .first_mut()
        .unwrap();
    moved_layer.visible = false;
    moved_layer.opacity = 0.45;

    app.navigate_workspace_history(reserved_key, HistoryDirection::Undo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 1);
    let restored = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone();
    assert_eq!(restored.id(), layer_id);
    assert!(Arc::ptr_eq(&restored.mesh, &mesh));
    assert_eq!(
        app.workspace.scenes[0].document.current_paths,
        [PathBuf::from("source-scan.stl")]
    );
    assert!(!app.workspace.scenes[0]
        .document
        .unsaved_edit_layer_ids
        .contains(&layer_id));
    assert!(app.workspace.scenes[0]
        .document
        .hidden_layer_stack
        .is_empty());
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .translucent_layer_restore
            .get(&layer_id),
        Some(&0.8)
    );
    assert!(!restored.visible);
    assert_eq!(restored.opacity, 0.45);

    let restored_document = &mut app.workspace.scenes[0].document;
    restored_document.current_paths = vec![PathBuf::from("changed-after-undo.stl")];
    restored_document.unsaved_edit_layer_ids.remove(&layer_id);
    restored_document.hidden_layer_stack.clear();
    restored_document
        .translucent_layer_restore
        .insert(layer_id, 0.9);
    let restored_layer = restored_document
        .live_scene_mut()
        .unwrap()
        .meshes_mut()
        .first_mut()
        .unwrap();
    restored_layer.visible = true;
    restored_layer.opacity = 0.65;

    app.navigate_workspace_history(source, HistoryDirection::Redo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 2);
    let recreated = app.workspace.scenes[1].key;
    assert_eq!(recreated.id, reserved_key.id);
    assert_ne!(recreated.epoch, reserved_key.epoch);
    let redone = app.workspace.scenes[1]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone();
    assert_eq!(redone.id(), layer_id);
    assert!(Arc::ptr_eq(&redone.mesh, &mesh));
    assert_eq!(
        app.workspace.scenes[1].document.current_paths,
        [PathBuf::from("changed-after-undo.stl")]
    );
    assert!(!app.workspace.scenes[1]
        .document
        .unsaved_edit_layer_ids
        .contains(&layer_id));
    assert!(app.workspace.scenes[1]
        .document
        .hidden_layer_stack
        .is_empty());
    assert_eq!(
        app.workspace.scenes[1]
            .document
            .translucent_layer_restore
            .get(&layer_id),
        Some(&0.9)
    );
    assert!(redone.visible);
    assert_eq!(redone.opacity, 0.65);
}

#[test]
fn rejected_preflights_do_not_leave_an_empty_auto_created_scene() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let original_layout = app.workspace.layout;
    let (destination, pane) = reserved_destination(&mut app);

    app.workspace.scenes[0].document.unsaved_sculpt_stroke = true;
    assert!(app
        .transfer_layers(
            source,
            TransferDestination::CreateBeside {
                scene: destination,
                pane,
                side: SplitSide::Left,
            },
            LayerIds::one(layer_id),
        )
        .is_err());
    assert_eq!(app.workspace.scenes.len(), 1);
    assert_eq!(app.workspace.layout, original_layout);

    app.workspace.scenes[0].document.unsaved_sculpt_stroke = false;
    let unknown_layer = SceneMesh::new(Mesh::empty()).id();
    assert!(app
        .transfer_layers(
            source,
            TransferDestination::CreateBeside {
                scene: destination,
                pane,
                side: SplitSide::Left,
            },
            LayerIds::one(unknown_layer),
        )
        .is_err());
    assert_eq!(app.workspace.scenes.len(), 1);
    assert_eq!(app.workspace.layout, original_layout);
    assert!(app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .iter()
        .any(|layer| layer.id() == layer_id));
}

#[test]
fn transfer_undo_keeps_previously_committed_scene_history() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let layer = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone();
    let token = app.workspace.scenes[0]
        .document
        .edit_mode
        .begin_layer_edit(&layer, EditModeCommand::InvertNormals)
        .unwrap();
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .edit_mode
            .finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    app.workspace.scenes[0]
        .document
        .edit_mode
        .finish_edit_session();

    let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
    assert_eq!(
        app.workspace
            .history
            .borrow()
            .top_step(Some(source), HistoryDirection::Undo)
            .unwrap()
            .kind,
        HistoryStepKind::Transfer
    );

    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    let previous = app
        .workspace
        .history
        .borrow()
        .top_step(Some(source), HistoryDirection::Undo)
        .unwrap();
    assert_eq!(previous.kind, HistoryStepKind::LayerEdit { layer_id });
}

#[test]
fn undo_preserves_a_touched_auto_created_destination_for_redo() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
    let dest = app
        .workspace
        .scenes
        .iter_mut()
        .find(|scene| scene.key == destination)
        .unwrap();
    dest.name = "Edited destination".to_owned();
    dest.preserve_on_transfer_undo = true;

    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    let retained = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap();
    assert_eq!(retained.name, "Edited destination");
    assert!(retained.preserve_on_transfer_undo);
    assert!(retained
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .is_empty());

    app.navigate_workspace_history(source, HistoryDirection::Redo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 2);
    let retained = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap();
    assert_eq!(retained.name, "Edited destination");
    assert_eq!(
        retained.document.scene.as_ref().unwrap().meshes()[0].id(),
        layer_id
    );
}

#[test]
fn auto_created_destination_with_later_transfer_redo_keeps_both_commands_navigable() {
    let (mut app, source, layer_a, _) = app_with_layer();
    let layer_b = SceneMesh::new(Arc::new(Mesh::empty()));
    let layer_b_id = layer_b.id();
    let source_document = &mut app.workspace.scenes[0].document;
    source_document.current_paths = vec![PathBuf::from("scan-a.stl"), PathBuf::from("scan-b.stl")];
    source_document.live_scene_mut().unwrap().add(layer_b);

    let (destination, _) = move_to_new_scene(&mut app, source, layer_a);
    app.transfer_layers(
        source,
        TransferDestination::Existing(destination),
        LayerIds::one(layer_b_id),
    )
    .unwrap();

    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();

    assert_eq!(app.workspace.scenes.len(), 2);
    let retained = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap();
    assert!(retained
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .is_empty());

    app.navigate_workspace_history(source, HistoryDirection::Redo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 2);
    assert_eq!(
        app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .key,
        destination
    );
    assert!(app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .iter()
        .any(|layer| layer.id() == layer_b_id));

    app.navigate_workspace_history(destination, HistoryDirection::Redo)
        .unwrap();
    let destination_scene = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .scene
        .as_ref()
        .unwrap()
        .clone();
    assert!(destination_scene
        .meshes()
        .iter()
        .any(|layer| layer.id() == layer_a));
    assert!(destination_scene
        .meshes()
        .iter()
        .any(|layer| layer.id() == layer_b_id));
}

#[test]
// Keep the complete transfer/edit/undo/redo scenario readable in one test.
#[allow(clippy::too_many_lines)]
fn undone_local_edit_keeps_auto_created_destination_for_transfer_redo() {
    let (mut app, source, layer_id, original_mesh) = app_with_layer();
    let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
    let layer = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone();
    let token = app
        .workspace
        .scenes
        .iter_mut()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .edit_mode
        .begin_layer_edit(&layer, EditModeCommand::InvertNormals)
        .unwrap();
    let changed_mesh = Arc::new(Mesh::empty());
    app.workspace
        .scenes
        .iter_mut()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .live_scene_mut()
        .unwrap()
        .meshes_mut()[0]
        .mesh = Arc::clone(&changed_mesh);
    assert_eq!(
        app.workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .edit_mode
            .finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    app.workspace
        .scenes
        .iter_mut()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .edit_mode
        .finish_edit_session();

    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    assert!(Arc::ptr_eq(
        &app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .mesh,
        &original_mesh
    ));
    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 2);
    assert!(app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key == destination)
        .unwrap()
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .is_empty());

    app.navigate_workspace_history(source, HistoryDirection::Redo)
        .unwrap();
    assert_eq!(app.workspace.scenes.len(), 2);
    assert_eq!(
        app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .key,
        destination
    );
    assert_eq!(
        app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .id(),
        layer_id
    );
    assert!(Arc::ptr_eq(
        &app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .mesh,
        &original_mesh
    ));
    app.navigate_workspace_history(destination, HistoryDirection::Redo)
        .unwrap();
    assert!(Arc::ptr_eq(
        &app.workspace
            .scenes
            .iter()
            .find(|scene| scene.key == destination)
            .unwrap()
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .mesh,
        &changed_mesh
    ));
}

#[test]
fn edit_checkpoint_in_either_participant_blocks_linked_undo() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
    let checkpoint = app
        .workspace
        .history
        .borrow_mut()
        .begin_edit_checkpoint(Some(destination), Scene::new(), 1)
        .unwrap();

    assert!(app
        .navigate_workspace_history(source, HistoryDirection::Undo)
        .is_err());
    assert_eq!(app.workspace.scenes.len(), 2);
    assert_eq!(
        app.workspace
            .history
            .borrow()
            .top_step(Some(source), HistoryDirection::Undo)
            .unwrap()
            .kind,
        HistoryStepKind::Transfer
    );
    assert!(app
        .workspace
        .history
        .borrow_mut()
        .cancel_edit_checkpoint(checkpoint)
        .is_some());
}

#[test]
fn pending_load_in_either_participant_blocks_linked_undo() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let (destination, _) = move_to_new_scene(&mut app, source, layer_id);
    app.loader.enqueue(crate::scene_loading::SceneLoadRequest {
        scene_key: destination,
        paths: vec![PathBuf::from("scan.stl")],
        source: "test",
        mode: crate::scene_loading::SceneLoadMode::Append,
        content_revision_at_request: 0,
        dirty_at_request: false,
        requested_at: std::time::Instant::now(),
    });

    assert!(app
        .navigate_workspace_history(source, HistoryDirection::Undo)
        .is_err());
    assert_eq!(app.workspace.scenes.len(), 2);
    assert_eq!(
        app.workspace
            .history
            .borrow()
            .top_step(Some(source), HistoryDirection::Undo)
            .unwrap()
            .kind,
        HistoryStepKind::Transfer
    );
}

#[test]
fn transfer_drops_derived_layer_overlay_and_clears_only_removed_scene_rulers() {
    let (mut app, source, layer_id, _) = app_with_layer();
    let colors = Arc::new(Vec::new());
    let analyzed_layer = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()[0]
        .clone()
        .with_overlay(occluview_core::OverlayKind::Measured, Some(colors));
    let mut source_scene = Scene::new();
    source_scene.add(analyzed_layer);
    app.workspace.scenes[0].document.scene = Some(Arc::new(source_scene));
    app.workspace.scenes[0]
        .tools
        .measure
        .place_ruler_point(Vec3::ZERO);
    app.workspace.scenes[0]
        .tools
        .measure
        .place_ruler_point(Vec3::X);

    let destination = app.workspace.ids.allocate_scene().unwrap();
    let pane = app.workspace.ids.allocate_pane_id().unwrap();
    let mut destination_session = SceneSession::new(
        destination,
        pane,
        "Destination".to_owned(),
        None,
        &app.workspace.history,
    );
    let mut destination_scene = Scene::new();
    destination_scene.add(SceneMesh::new(Mesh::empty()));
    destination_session.document.scene = Some(Arc::new(destination_scene));
    destination_session
        .tools
        .measure
        .place_ruler_point(Vec3::ZERO);
    destination_session.tools.measure.place_ruler_point(Vec3::Y);
    app.workspace.scenes.push(destination_session);
    app.workspace.layout = WorkspaceLayout::SideBySide {
        left: app.workspace.scenes[0].pane,
        right: pane,
        ratio: 0.5,
    };

    app.ui.popup_open_at_frame_start = true;
    app.transfer_layers(
        source,
        TransferDestination::Existing(destination),
        LayerIds::one(layer_id),
    )
    .unwrap();
    assert_eq!(app.workspace.scenes[0].tools.measure.ruler_count(), 0);
    assert_eq!(app.workspace.scenes[1].tools.measure.ruler_count(), 1);
    assert_eq!(
        app.workspace.scenes[1]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[1]
            .overlay_kind(),
        None
    );

    app.workspace.scenes[0]
        .tools
        .measure
        .place_ruler_point(Vec3::ZERO);
    app.workspace.scenes[0]
        .tools
        .measure
        .place_ruler_point(Vec3::Z);
    app.navigate_workspace_history(destination, HistoryDirection::Undo)
        .unwrap();
    assert_eq!(app.workspace.scenes[0].tools.measure.ruler_count(), 1);
    assert_eq!(app.workspace.scenes[1].tools.measure.ruler_count(), 0);
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()[0]
            .overlay_kind(),
        None
    );
}

#![allow(clippy::expect_used)]
//! Controller tests: state machine, sessions, selection, undo/redo, sync.

use super::*;
use crate::app::workspace::history::{
    HistoryDirection, HistorySnapshot, HistoryStepKind, WorkspaceHistory,
};
use occluview_core::test_support::{simple_triangle_mesh, two_triangle_mesh};
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, ScenePickHit, Vertex};

/// A two-object soup mesh: object A (soup triangles 0,1) near the origin and
/// object B (soup triangles 2,3) far along +x. Each object is a quad emitted as
/// two edge-connected triangles with private, unshared vertices — the
/// multi-object STL soup the Object mode must resolve to whole objects.
fn two_object_soup_mesh(name: &str) -> Option<Mesh> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for base_x in [0.0_f32, 10.0] {
        let quad = [
            glam::Vec3::new(base_x, 0.0, 0.0),
            glam::Vec3::new(base_x + 1.0, 0.0, 0.0),
            glam::Vec3::new(base_x + 1.0, 1.0, 0.0),
            glam::Vec3::new(base_x, 1.0, 0.0),
        ];
        for corner in [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]] {
            indices.push(u32::try_from(vertices.len()).unwrap_or(0));
            vertices.push(Vertex::at(corner));
        }
    }
    Mesh::new(Some(name.to_string()), vertices, indices).ok()
}

fn pick_hit(layer_index: usize, layer_id: SceneMeshId, triangle_index: usize) -> ScenePickHit {
    ScenePickHit {
        layer_index,
        layer_id,
        triangle_index,
        point: glam::Vec3::ZERO,
        distance: 1.0,
    }
}

#[test]
fn edit_mode_tracks_dirty_state_and_discard_without_scene_indices() {
    let layer = LayerKey::new(42);
    let mut state = EditModeState::default();

    assert_eq!(layer.get(), 42);
    state.start(layer);
    assert_eq!(state.active_layer(), Some(layer));
    assert!(!state.is_dirty());

    state.mark_dirty();
    assert!(state.is_dirty());
    assert!(matches!(state, EditModeState::ActiveDirty { layer: active } if active == layer));

    state.confirm_discard();
    assert!(matches!(state, EditModeState::Inactive));
}

#[test]
fn busy_completion_ignores_stale_tokens_and_restores_dirty_state() {
    let layer = LayerKey::new(7);
    let token = EditSessionToken::new(100);
    let stale_token = EditSessionToken::new(101);
    let mut edit_state = EditModeState::default();

    assert_eq!(token.get(), 100);
    edit_state.start(layer);
    edit_state.mark_dirty();
    assert!(edit_state.begin_busy(EditModeCommand::InvertNormals, token));

    let stale_result = edit_state.finish_busy_success(stale_token, true);
    assert_eq!(stale_result, BusyFinish::Stale);
    assert!(matches!(
        edit_state,
        EditModeState::Busy {
            layer: active,
            token: active_token,
            ..
        } if active == layer && active_token == token
    ));

    let result = edit_state.finish_busy_success(token, false);
    assert_eq!(result, BusyFinish::Applied);
    assert!(matches!(edit_state, EditModeState::ActiveDirty { layer: active } if active == layer));
}

#[test]
fn busy_failure_enters_recoverable_error_for_matching_token() {
    let layer = LayerKey::new(3);
    let token = EditSessionToken::new(200);
    let mut state = EditModeState::default();

    state.start(layer);
    assert!(state.begin_busy(EditModeCommand::InvertNormals, token));
    assert_eq!(
        state.finish_busy_error(token, "failed".to_string()),
        BusyFinish::Applied
    );

    assert!(matches!(
        state,
        EditModeState::Error {
            layer: Some(active),
            recoverable: true,
            ..
        } if active == layer
    ));
    assert!(state.clear_error());
    assert!(matches!(state, EditModeState::ActiveDirty { layer: active } if active == layer));
}

#[test]
fn edit_mode_commands_cover_planned_operations() {
    let commands = [
        EditModeCommand::BridgeSplit,
        EditModeCommand::InvertNormals,
        EditModeCommand::DeleteSelectedFaces,
        EditModeCommand::CropToSelectedFaces,
        EditModeCommand::CutSelectionToNewLayer,
        EditModeCommand::SeparateSelectedComponents,
    ];

    assert_eq!(commands.len(), 6);
}

fn journal_kind() -> HistoryStepKind {
    HistoryStepKind::LayerEdit {
        layer_id: SceneMesh::new(Mesh::empty()).id(),
    }
}

fn commit_journal_edit(history: &mut WorkspaceHistory, value: &'static str, bytes: usize) {
    let command = history
        .push_pending_edit(None, journal_kind(), value, bytes)
        .expect("snapshot fits");
    assert!(history.commit_pending_edit(command));
}

fn navigate_journal(
    history: &mut WorkspaceHistory,
    direction: HistoryDirection,
    current: &'static str,
) -> Option<&'static str> {
    let command = history.top_step(None, direction)?.command_id;
    history.navigate_edit(
        None,
        direction,
        command,
        HistorySnapshot {
            value: current,
            bytes: 4,
        },
    )
}

#[test]
fn workspace_journal_bounds_count_and_bytes_across_undo_redo() {
    let mut history = WorkspaceHistory::new(2, 12);
    for value in ["a", "b", "c"] {
        commit_journal_edit(&mut history, value, 4);
    }
    assert_eq!(history.undo_len(None), 2);
    assert_eq!(history.used_bytes(), 8);
    assert_eq!(
        navigate_journal(&mut history, HistoryDirection::Undo, "current"),
        Some("c")
    );
    assert_eq!(history.used_bytes(), 8);
    assert_eq!(
        navigate_journal(&mut history, HistoryDirection::Redo, "after-undo"),
        Some("current")
    );
    commit_journal_edit(&mut history, "new", 4);
    assert!(history.top_step(None, HistoryDirection::Redo).is_none());
    assert!(history
        .push_pending_edit(None, journal_kind(), "too-large", 13)
        .is_none());
    assert_eq!(history.undo_len(None), 2);
    assert_eq!(history.used_bytes(), 8);
    history.clear_scope(None);
    assert!(history.top_step(None, HistoryDirection::Undo).is_none());
    assert!(history.top_step(None, HistoryDirection::Redo).is_none());
    assert_eq!(history.used_bytes(), 0);
}

#[test]
fn workspace_journal_noop_preserves_redo_history() {
    let mut history = WorkspaceHistory::new(8, 4096);
    commit_journal_edit(&mut history, "before-edit", 4);
    assert_eq!(
        navigate_journal(&mut history, HistoryDirection::Undo, "after-edit"),
        Some("before-edit")
    );
    let pending = history
        .push_pending_edit(None, journal_kind(), "before-noop", 4)
        .expect("snapshot fits");
    assert!(history.discard_pending(pending));
    assert_eq!(
        navigate_journal(&mut history, HistoryDirection::Redo, "current"),
        Some("after-edit")
    );
}

#[test]
fn workspace_journal_committed_edit_clears_redo() {
    let mut history = WorkspaceHistory::new(8, 4096);
    commit_journal_edit(&mut history, "before-edit", 4);
    assert_eq!(
        navigate_journal(&mut history, HistoryDirection::Undo, "after-edit"),
        Some("before-edit")
    );
    assert!(history.top_step(None, HistoryDirection::Redo).is_some());
    commit_journal_edit(&mut history, "before-real-edit", 4);
    assert!(history.top_step(None, HistoryDirection::Redo).is_none());
}

#[test]
fn controller_records_layer_edit_undo_and_dirty_state() {
    let layer = SceneMesh::new(Mesh::empty());
    let mut controller = EditModeController::new(4, 1_000_000);

    let token = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals);
    assert!(token.is_some());
    assert_eq!(
        controller.active_layer(),
        Some(LayerKey::from_scene_mesh_id(layer.id()))
    );
    assert_eq!(
        controller.undo_len(),
        0,
        "uncommitted work is not an undo step"
    );

    let Some(token) = token else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert!(controller.is_dirty());
    assert_eq!(
        controller.undo_len(),
        1,
        "successful work publishes its snapshot"
    );
}

#[test]
fn controller_undo_restores_last_snapshot_for_matching_layer() {
    let Some(before_mesh) = simple_triangle_mesh(Some("before")) else {
        panic!("required test setup or expected result was missing");
    };
    let Some(after_mesh) = simple_triangle_mesh(Some("after")) else {
        panic!("required test setup or expected result was missing");
    };
    let layer = SceneMesh::new(before_mesh);
    let mut current = layer.clone();
    current.mesh = std::sync::Arc::new(after_mesh);
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert_eq!(controller.undo_layer_id(), Some(layer.id()));

    let restored = controller.undo_last_layer_edit(&current);
    assert!(restored.is_some());
    let Some(restored) = restored else {
        panic!("required test setup or expected result was missing");
    };

    assert_eq!(restored.id(), layer.id());
    assert_eq!(restored.mesh.name(), Some("before"));
    assert_eq!(restored.mesh.indices(), layer.mesh.indices());
    assert_eq!(controller.undo_layer_id(), None);
}

#[test]
fn controller_undo_rejects_stale_layer_without_popping_history() {
    let Some(before_mesh) = simple_triangle_mesh(Some("before")) else {
        panic!("required test setup or expected result was missing");
    };
    let Some(other_mesh) = simple_triangle_mesh(Some("other")) else {
        panic!("required test setup or expected result was missing");
    };
    let layer = SceneMesh::new(before_mesh);
    let other_layer = SceneMesh::new(other_mesh);
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );

    assert!(controller.undo_last_layer_edit(&other_layer).is_none());
    assert_eq!(controller.undo_layer_id(), Some(layer.id()));
}

#[test]
fn controller_undo_is_lifo_across_layer_edits() {
    let Some(first_mesh) = simple_triangle_mesh(Some("first")) else {
        panic!("required test setup or expected result was missing");
    };
    let Some(second_mesh) = simple_triangle_mesh(Some("second")) else {
        panic!("required test setup or expected result was missing");
    };
    let first = SceneMesh::new(first_mesh);
    let second = SceneMesh::new(second_mesh);
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(first_token) = controller.begin_layer_edit(&first, EditModeCommand::InvertNormals)
    else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_layer_edit_success(first_token),
        BusyFinish::Applied
    );

    let Some(second_token) = controller.begin_layer_edit(&second, EditModeCommand::InvertNormals)
    else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_layer_edit_success(second_token),
        BusyFinish::Applied
    );

    assert_eq!(controller.undo_layer_id(), Some(second.id()));
    assert_eq!(
        controller
            .undo_last_layer_edit(&second)
            .map(|layer| layer.id()),
        Some(second.id())
    );
    assert_eq!(controller.undo_layer_id(), Some(first.id()));
    assert_eq!(
        controller
            .undo_last_layer_edit(&first)
            .map(|layer| layer.id()),
        Some(first.id())
    );
}

#[test]
fn controller_restores_scene_snapshot_when_layer_set_is_unchanged() {
    let Some(first_mesh) = simple_triangle_mesh(Some("first")) else {
        panic!("required test setup or expected result was missing");
    };
    let Some(second_mesh) = simple_triangle_mesh(Some("second")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let first_index = scene.add(SceneMesh::new(first_mesh));
    scene.add(SceneMesh::new(second_mesh));
    let layer_id = scene.meshes()[first_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) =
        controller.begin_scene_edit(&scene, layer_id, EditModeCommand::CutSelectionToNewLayer)
    else {
        panic!("required test setup or expected result was missing");
    };
    // Stamp the post-op fingerprint to the current (unchanged) id-set.
    assert_eq!(
        controller.finish_scene_edit_success(token, &scene),
        BusyFinish::Applied
    );

    // Presentation-only change (same layer id-set): the restore is still safe,
    // and it discards the presentation change back to the snapshot.
    let mut mutated = scene.clone();
    mutated.meshes_mut()[0].visible = false;

    let step = controller.undo_last_scene_edit(&mutated, layer_id);
    assert!(
        matches!(step, StructuralHistoryStep::Restored(_)),
        "an unchanged layer set should restore the scene snapshot"
    );
    let StructuralHistoryStep::Restored(restored) = step else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(restored.meshes().len(), 2);
    assert!(restored.meshes()[0].visible);
    assert_eq!(restored.meshes()[0].id(), layer_id);
}

#[test]
fn controller_refuses_scene_snapshot_when_layer_was_removed_since() {
    // Structural-history guard: a whole-scene restore is refused when the live
    // scene lost a layer since the structural step, rather than resurrecting it.
    let Some(first_mesh) = simple_triangle_mesh(Some("first")) else {
        panic!("required test setup or expected result was missing");
    };
    let Some(second_mesh) = simple_triangle_mesh(Some("second")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let first_index = scene.add(SceneMesh::new(first_mesh));
    scene.add(SceneMesh::new(second_mesh));
    let layer_id = scene.meshes()[first_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) =
        controller.begin_scene_edit(&scene, layer_id, EditModeCommand::CutSelectionToNewLayer)
    else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        controller.finish_scene_edit_success(token, &scene),
        BusyFinish::Applied
    );

    let mut mutated = scene.clone();
    mutated.remove(1);

    assert!(
        matches!(
            controller.undo_last_scene_edit(&mutated, layer_id),
            StructuralHistoryStep::SceneChanged
        ),
        "removing a layer since the snapshot must block the whole-scene restore"
    );
    // The snapshot is left intact for the caller to report, not popped.
    assert_eq!(controller.undo_layer_id(), Some(layer_id));
}

#[test]
fn controller_refuses_scene_snapshot_when_layer_was_appended_since() {
    // Data-loss case: a layer appended after a structural op must not be
    // deleted by an undo that restores the pre-append whole-scene snapshot.
    let Some(mesh) = simple_triangle_mesh(Some("source")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) =
        controller.begin_scene_edit(&scene, layer_id, EditModeCommand::CutSelectionToNewLayer)
    else {
        panic!("required test setup or expected result was missing");
    };
    // Simulate the structural product being inserted before the op finishes.
    let Some(product) = simple_triangle_mesh(Some("product")) else {
        panic!("required test setup or expected result was missing");
    };
    scene.add(SceneMesh::new(product));
    assert_eq!(
        controller.finish_scene_edit_success(token, &scene),
        BusyFinish::Applied
    );

    // The operator now appends a fresh layer (a separate load) after the op.
    let Some(appended) = simple_triangle_mesh(Some("appended")) else {
        panic!("required test setup or expected result was missing");
    };
    scene.add(SceneMesh::new(appended));

    assert!(
        matches!(
            controller.undo_last_scene_edit(&scene, layer_id),
            StructuralHistoryStep::SceneChanged
        ),
        "an appended layer must block the whole-scene restore that would drop it"
    );
    assert_eq!(controller.undo_layer_id(), Some(layer_id));
}

#[test]
fn controller_records_failed_layer_edit_as_recoverable_error() {
    let layer = SceneMesh::new(Mesh::empty());
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        panic!("required test setup or expected result was missing");
    };

    assert_eq!(
        controller.finish_layer_edit_error(token, "failed".to_string()),
        BusyFinish::Applied
    );
    assert!(matches!(
        controller.state(),
        EditModeState::Error {
            layer: Some(active),
            recoverable: true,
            ..
        } if *active == LayerKey::from_scene_mesh_id(layer.id())
    ));
}

#[test]
fn controller_records_face_selection_from_scene_pick_hit() {
    let Some(mesh) = two_triangle_mesh(Some("selectable")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    let selected = controller.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id,
            triangle_index: 1,
            point: glam::Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    );

    assert!(selected);
    assert_eq!(controller.selected_layer_id(), Some(layer_id));
    assert_eq!(controller.selected_face_count(), 1);
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[false, true]);
}

#[test]
fn controller_rejects_stale_face_selection_hit_without_clearing_current_selection() {
    let Some(mesh) = two_triangle_mesh(Some("selectable")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id,
            triangle_index: 0,
            point: glam::Vec3::new(0.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
    let stale_selected = controller.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id: stale_layer_id,
            triangle_index: 1,
            point: glam::Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    );

    assert!(!stale_selected);
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, false]);
}

#[test]
fn controller_can_accumulate_invert_and_clear_selection_without_leaving_edit_mode() {
    let Some(mesh) = two_triangle_mesh(Some("selectable")) else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&scene.meshes()[layer_index], &scene));
    assert_eq!(controller.selected_layer_id(), Some(layer_id));
    assert_eq!(controller.selected_face_count(), 0);

    assert!(controller.select_face_hit_with_mode(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id,
            triangle_index: 0,
            point: glam::Vec3::new(0.25, 0.25, 0.0),
            distance: 10.0,
        },
        false,
    ));
    // Plain clicks accumulate (dental CAD convention — no replace).
    assert!(controller.select_face_hit_with_mode(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id,
            triangle_index: 1,
            point: glam::Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
        false,
    ));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true]);

    // Shift-click un-marks the clicked face only.
    assert!(controller.select_face_hit_with_mode(
        &scene,
        ScenePickHit {
            layer_index,
            layer_id,
            triangle_index: 0,
            point: glam::Vec3::new(0.25, 0.25, 0.0),
            distance: 10.0,
        },
        true,
    ));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[false, true]);

    assert!(controller.clear_face_selection());
    assert_eq!(controller.selected_layer_id(), Some(layer_id));
    assert_eq!(controller.selected_face_count(), 0);
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[false, false]);

    assert!(controller.invert_face_selection());
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true]);

    assert!(controller.clear_face_selection());
    assert!(controller.select_all_faces());
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true]);
}

#[test]
fn controller_clear_resets_state_and_history() {
    let layer = SceneMesh::new(Mesh::empty());
    let mut controller = EditModeController::new(4, 1_000_000);
    let token = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals);
    assert!(token.is_some());

    controller.clear();

    assert!(matches!(controller.state(), EditModeState::Inactive));
    assert_eq!(controller.undo_len(), 0);
    assert_eq!(controller.selected_layer_id(), None);
}

#[test]
fn object_mode_defaults_off_and_is_mutually_exclusive_with_lasso() {
    let mut controller = EditModeController::new(4, 1_000_000);
    // A fresh controller is in neither gesture (marquee resting state).
    assert!(!controller.object_mode());
    assert!(!controller.lasso_armed());

    // Arming Object turns it on and leaves the lasso off.
    assert!(controller.set_object_mode(true));
    assert!(controller.object_mode());
    assert!(!controller.lasso_armed());

    // Arming the lasso disarms Object (mutually exclusive gestures).
    assert!(controller.set_lasso_armed(true));
    assert!(controller.lasso_armed());
    assert!(!controller.object_mode());

    // Re-arming Object disarms the lasso again.
    assert!(controller.set_object_mode(true));
    assert!(controller.object_mode());
    assert!(!controller.lasso_armed());

    // Disarming Object returns to marquee (neither gesture) and is idempotent.
    assert!(controller.set_object_mode(false));
    assert!(!controller.object_mode());
    assert!(!controller.lasso_armed());
    assert!(!controller.set_object_mode(false));
}

#[test]
fn object_click_selects_whole_component_accumulating_with_shift_unmark() {
    let Some(mesh) = two_object_soup_mesh("multi-object") else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&scene.meshes()[layer_index], &scene));
    assert!(controller.set_object_mode(true));

    // Click a facet of object A (triangle 0): its whole object (0,1) is marked,
    // object B (2,3) is untouched — not confetti, not the neighbour.
    assert!(controller.select_component_hit(&scene, pick_hit(layer_index, layer_id, 0), false));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true, false, false]);

    // Clicking object B accumulates (dental CAD convention — plain click adds).
    assert!(controller.select_component_hit(&scene, pick_hit(layer_index, layer_id, 3), false));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true, true, true]);

    // Shift-click on object A un-marks the whole object A.
    assert!(controller.select_component_hit(&scene, pick_hit(layer_index, layer_id, 1), true));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[false, false, true, true]);

    // Idempotent: clicking object A twice stays selected, never toggles off.
    assert!(controller.select_component_hit(&scene, pick_hit(layer_index, layer_id, 0), false));
    assert!(controller.select_component_hit(&scene, pick_hit(layer_index, layer_id, 0), false));
    let Some(selection) = controller.selected_faces_for_layer(layer_id) else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(selection.as_slice(), &[true, true, true, true]);
}

#[test]
fn object_click_on_a_different_layer_is_a_noop() {
    let Some(mesh) = two_object_soup_mesh("multi-object") else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&scene.meshes()[layer_index], &scene));
    assert!(controller.set_object_mode(true));

    // A hit carrying a foreign layer id (a click that landed on another layer)
    // is rejected and leaves the empty selection untouched.
    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
    assert!(!controller.select_component_hit(
        &scene,
        pick_hit(layer_index, stale_layer_id, 0),
        false,
    ));
    assert_eq!(controller.selected_face_count(), 0);
}

#[test]
fn object_mode_resets_when_the_session_ends() {
    let Some(mesh) = two_object_soup_mesh("reset") else {
        panic!("required test setup or expected result was missing");
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    // Arm Object mid-session, then Done: the gesture resets so no stale Object
    // state leaks, and the next session opens on the lasso as always.
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.set_object_mode(true));
    assert!(controller.object_mode());
    controller.finish_edit_session();
    assert!(!controller.object_mode());
    assert!(!controller.lasso_armed());

    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(!controller.object_mode());
    assert!(controller.lasso_armed());

    // Cancel resets the gesture too.
    assert!(controller.set_object_mode(true));
    assert!(controller.cancel_edit_session().is_some());
    assert!(!controller.object_mode());
    assert!(!controller.lasso_armed());
}

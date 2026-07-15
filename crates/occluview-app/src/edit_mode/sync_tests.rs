//! Scene-sync regression tests kept separate from the core controller suite.

use occluview_core::{Scene, SceneMesh, ScenePickHit};

use super::session_tests::{triangle_mesh, two_triangle_mesh};
use super::*;

#[test]
fn controller_sync_to_scene_clears_stale_or_hidden_selection() {
    let Some(mesh) = two_triangle_mesh("sync") else {
        return;
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
    assert_eq!(controller.selected_layer_id(), Some(layer_id));

    scene.meshes_mut()[0].visible = false;
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
}

#[test]
fn controller_sync_to_scene_clears_selection_when_topology_changes() {
    let Some(mesh) = two_triangle_mesh("sync") else {
        return;
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
            triangle_index: 1,
            point: glam::Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let Some(single_triangle) = triangle_mesh("single") else {
        return;
    };
    scene.meshes_mut()[0].mesh = single_triangle;
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
}

#[test]
fn controller_sync_to_scene_discards_active_state_and_undo_for_removed_layer() {
    let Some(first_mesh) = triangle_mesh("first") else {
        return;
    };
    let Some(second_mesh) = triangle_mesh("second") else {
        return;
    };
    let mut scene = Scene::new();
    let first_index = scene.add(SceneMesh::new(first_mesh));
    scene.add(SceneMesh::new(second_mesh));
    let layer = scene.meshes()[first_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert_eq!(controller.undo_layer_id(), Some(layer.id()));

    scene.remove(first_index);
    controller.sync_to_scene(&scene);

    assert!(matches!(controller.state(), EditModeState::Inactive));
    assert_eq!(controller.undo_len(), 0);
    assert_eq!(controller.undo_layer_id(), None);
}

//! Canonical multi-layer selection behavior.

use super::session_tests::{triangle_mesh, two_triangle_mesh};
use super::*;
use occluview_core::{CameraProjection, Scene, SceneMesh, SceneMeshId, ScenePickHit};

fn hit(layer_index: usize, layer_id: SceneMeshId, triangle_index: usize) -> ScenePickHit {
    ScenePickHit {
        layer_index,
        layer_id,
        triangle_index,
        point: glam::Vec3::ZERO,
        distance: 1.0,
    }
}

fn camera() -> occluview_core::Camera {
    occluview_core::Camera {
        target: glam::Vec3::ZERO,
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        orientation: None,
        projection: CameraProjection::Orthographic,
        orthographic_height: 100.0,
        fovy: 45.0_f32.to_radians(),
        near: 0.1,
        far: 10_000.0,
    }
}

fn full_viewport_request(polygon_px: &[egui::Pos2]) -> ScreenPolygonSelectionRequest<'_> {
    ScreenPolygonSelectionRequest {
        viewport_rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0)),
        polygon_px,
        unmark: false,
        through_mesh: true,
    }
}

#[test]
fn hidden_selection_is_retained_but_visible_plan_excludes_it() {
    let Some(mesh) = triangle_mesh("hidden") else {
        return;
    };
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    let id = scene.meshes()[index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_face_hit(&scene, hit(index, id, 0)));
    assert_eq!(controller.total_selected_face_count(), 1);
    assert_eq!(controller.total_selected_layer_count(), 1);

    scene.meshes_mut()[index].visible = false;
    controller.sync_to_scene(&scene);
    assert_eq!(controller.visible_selected_face_count(&scene), 0);
    assert_eq!(controller.visible_selected_layer_count(&scene), 0);
    assert!(controller.visible_selection_plan(&scene).is_empty());

    scene.meshes_mut()[index].visible = true;
    controller.sync_to_scene(&scene);
    let plan = controller.visible_selection_plan(&scene);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].layer_id, id);
    assert_eq!(plan[0].selection.selected_count(), 1);
}

#[test]
fn lasso_accumulates_selection_across_two_visible_layers() {
    let Some(mesh_a) = triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = triangle_mesh("B") else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh_a));
    scene.add(SceneMesh::new(mesh_b));
    let polygon = [
        egui::pos2(0.0, 0.0),
        egui::pos2(400.0, 0.0),
        egui::pos2(400.0, 400.0),
        egui::pos2(0.0, 400.0),
    ];
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_faces_in_screen_polygon(
        &scene,
        &camera(),
        full_viewport_request(&polygon),
    ));

    let plan = controller.visible_selection_plan(&scene);
    assert_eq!(plan.len(), 2);
    assert_eq!(controller.total_selected_face_count(), 2);
    assert_eq!(controller.total_selected_layer_count(), 2);
}

#[test]
fn face_clicks_accumulate_by_hit_layer_without_switching_state() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let id_a = scene.meshes()[index_a].id();
    let id_b = scene.meshes()[index_b].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_face_hit(&scene, hit(index_a, id_a, 0)));
    assert!(controller.select_face_hit(&scene, hit(index_b, id_b, 1)));

    let plan = controller.visible_selection_plan(&scene);
    assert_eq!(plan.len(), 2);
    assert_eq!(controller.total_selected_face_count(), 2);
    assert_eq!(controller.total_selected_layer_count(), 2);
    assert!(plan.iter().any(|entry| entry.layer_id == id_a));
    assert!(plan.iter().any(|entry| entry.layer_id == id_b));
}

#[test]
fn object_picks_accumulate_components_across_two_visible_layers() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let id_a = scene.meshes()[index_a].id();
    let id_b = scene.meshes()[index_b].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_component_hit(&scene, hit(index_a, id_a, 0), false));
    assert!(controller.select_component_hit(&scene, hit(index_b, id_b, 1), false));

    let plan = controller.visible_selection_plan(&scene);
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].layer_id, id_a);
    assert_eq!(plan[0].selection.selected_count(), 1);
    assert_eq!(plan[1].layer_id, id_b);
    assert_eq!(plan[1].selection.selected_count(), 1);
}

#[test]
fn topology_sync_invalidates_only_the_affected_layer() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let Some(rebuilt_a) = triangle_mesh("A-rebuilt") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let id_a = scene.meshes()[index_a].id();
    let id_b = scene.meshes()[index_b].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_face_hit(&scene, hit(index_a, id_a, 0)));
    assert!(controller.select_face_hit(&scene, hit(index_b, id_b, 1)));
    scene.meshes_mut()[index_a].mesh = rebuilt_a;
    controller.sync_to_scene(&scene);

    let plan = controller.visible_selection_plan(&scene);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].layer_id, id_b);
    assert_eq!(controller.total_selected_face_count(), 1);
}

#[test]
fn visible_bulk_operations_ignore_hidden_layers() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    scene.meshes_mut()[index_b].visible = false;
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.select_all_visible_selections(&scene));
    assert_eq!(controller.visible_selected_face_count(&scene), 2);
    assert_eq!(controller.visible_selected_layer_count(&scene), 1);
    assert_eq!(controller.visible_selections(&scene).count(), 1);
    assert!(controller.invert_visible_selections(&scene));
    assert_eq!(controller.visible_selected_face_count(&scene), 0);
    assert!(!controller.clear_visible_selections(&scene));
}

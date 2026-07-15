//! Multi-layer edit-session selection, suspension, and baseline tests.

use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, ScenePickHit, Vertex};

use super::session_tests::{triangle_mesh, two_triangle_mesh};
use super::*;

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
fn switching_targets_restores_cached_marks_per_layer() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let layer_a = scene.meshes()[index_a].clone();
    let layer_b = scene.meshes()[index_b].clone();
    let id_a = layer_a.id();
    let id_b = layer_b.id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    assert!(controller.select_face_hit(&scene, pick_hit(index_a, id_a, 0)));

    assert!(controller.begin_face_selection(&layer_b, &scene));
    assert_eq!(controller.selected_layer_id(), Some(id_b));
    assert_eq!(controller.selected_face_count(), 0);
    assert!(controller.select_face_hit(&scene, pick_hit(index_b, id_b, 1)));

    assert!(controller.begin_face_selection(&layer_a, &scene));
    assert_eq!(controller.selected_layer_id(), Some(id_a));
    assert_eq!(controller.selected_face_count(), 1);
    let Some(selection_a) = controller.selected_faces_for_layer(id_a) else {
        return;
    };
    assert_eq!(selection_a.as_slice(), &[true, false]);

    assert!(controller.begin_face_selection(&layer_b, &scene));
    assert_eq!(controller.selected_layer_id(), Some(id_b));
    assert_eq!(controller.selected_face_count(), 1);
    let Some(selection_b) = controller.selected_faces_for_layer(id_b) else {
        return;
    };
    assert_eq!(selection_b.as_slice(), &[false, true]);
}

#[test]
fn cached_marks_reset_when_same_layer_topology_changes_with_same_triangle_count() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let Some(rebuilt_a) = Mesh::new(
        Some("A-rebuilt".to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 0.0, 1.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 1.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 1.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok() else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let layer_a = scene.meshes()[index_a].clone();
    let layer_b = scene.meshes()[index_b].clone();
    let id_a = layer_a.id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    assert!(controller.select_face_hit(&scene, pick_hit(index_a, id_a, 0)));
    assert!(controller.begin_face_selection(&layer_b, &scene));

    let original_topology = scene.meshes()[index_a].mesh.topology_id();
    scene.meshes_mut()[index_a].mesh = rebuilt_a;
    assert_eq!(scene.meshes()[index_a].mesh.triangle_count(), 2);
    assert_ne!(
        scene.meshes()[index_a].mesh.topology_id(),
        original_topology
    );

    assert!(controller.begin_face_selection(&scene.meshes()[index_a], &scene));
    assert_eq!(controller.selected_layer_id(), Some(id_a));
    assert_eq!(controller.selected_face_count(), 0);
    let Some(selection) = controller.selected_faces_for_layer(id_a) else {
        return;
    };
    assert_eq!(selection.as_slice(), &[false, false]);
}

#[test]
fn switching_targets_keeps_original_session_baseline_for_cancel() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let Some(edited_a) = triangle_mesh("A-edited") else {
        return;
    };
    let Some(edited_b) = triangle_mesh("B-edited") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let original = scene.clone();
    let layer_a = scene.meshes()[index_a].clone();
    let layer_b = scene.meshes()[index_b].clone();
    let mut controller = EditModeController::new(8, 10_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    let Some(token) = controller.begin_layer_edit(&layer_a, EditModeCommand::DeleteSelectedFaces)
    else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    scene.meshes_mut()[index_a].mesh = edited_a;
    controller.sync_to_scene(&scene);

    assert!(controller.begin_face_selection(&scene.meshes()[index_b], &scene));
    let Some(token) = controller.begin_layer_edit(&layer_b, EditModeCommand::CloseHoles) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    scene.meshes_mut()[index_b].mesh = edited_b;
    controller.sync_to_scene(&scene);

    let Some(restored) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(restored.meshes().len(), original.meshes().len());
    for (restored_entry, original_entry) in restored.meshes().iter().zip(original.meshes()) {
        assert_eq!(restored_entry.id(), original_entry.id());
        assert_eq!(
            restored_entry.mesh.topology_id(),
            original_entry.mesh.topology_id()
        );
        assert_eq!(restored_entry.mesh.indices(), original_entry.mesh.indices());
        assert_eq!(
            restored_entry.mesh.triangle_count(),
            original_entry.mesh.triangle_count()
        );
    }
}

#[test]
fn hidden_or_removed_active_target_never_retargets_silently() {
    let Some(mesh) = two_triangle_mesh("hidden") else {
        return;
    };
    let Some(other_mesh) = two_triangle_mesh("other") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let other_index = scene.add(SceneMesh::new(other_mesh));
    let layer = scene.meshes()[layer_index].clone();
    let other = scene.meshes()[other_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.begin_face_selection(&other, &scene));
    scene.meshes_mut()[other_index].visible = false;
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
    assert!(controller.cancel_edit_session().is_some());

    let mut scene = Scene::new();
    let Some(mesh) = two_triangle_mesh("hidden") else {
        return;
    };
    let Some(other_mesh) = two_triangle_mesh("other") else {
        return;
    };
    let layer_index = scene.add(SceneMesh::new(mesh));
    let other_index = scene.add(SceneMesh::new(other_mesh));
    let layer = scene.meshes()[layer_index].clone();
    let other = scene.meshes()[other_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.begin_face_selection(&other, &scene));
    scene.remove(other_index);
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
    assert!(controller.cancel_edit_session().is_some());
    assert!(controller.begin_face_selection(&scene.meshes()[layer_index], &scene));
    assert_eq!(controller.selected_layer_id(), Some(layer.id()));
}

#[test]
fn hidden_layers_cannot_become_the_active_editor_target() {
    let Some(visible_mesh) = two_triangle_mesh("visible") else {
        return;
    };
    let Some(hidden_mesh) = two_triangle_mesh("hidden") else {
        return;
    };
    let mut scene = Scene::new();
    let visible_index = scene.add(SceneMesh::new(visible_mesh));
    let hidden_index = scene.add(SceneMesh::new(hidden_mesh));
    let visible = scene.meshes()[visible_index].clone();
    let hidden_id = scene.meshes()[hidden_index].id();
    scene.meshes_mut()[hidden_index].visible = false;
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&visible, &scene));
    assert!(!controller.begin_face_selection(&scene.meshes()[hidden_index], &scene));
    assert_eq!(controller.selected_layer_id(), Some(visible.id()));
    assert_ne!(controller.selected_layer_id(), Some(hidden_id));
}

#[test]
fn cancel_restores_dirty_session_after_its_active_layer_is_removed_then_switched() {
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let Some(mesh_b) = two_triangle_mesh("B") else {
        return;
    };
    let Some(edited_a) = triangle_mesh("A-edited") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let index_b = scene.add(SceneMesh::new(mesh_b));
    let original = scene.clone();
    let layer_a = scene.meshes()[index_a].clone();
    let layer_b_id = scene.meshes()[index_b].id();
    let mut controller = EditModeController::new(8, 10_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    let Some(token) = controller.begin_layer_edit(&layer_a, EditModeCommand::DeleteSelectedFaces)
    else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    scene.meshes_mut()[index_a].mesh = edited_a;
    controller.sync_to_scene(&scene);
    assert!(controller.is_dirty());

    scene.remove(index_a);
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
    assert!(controller.has_active_session());

    assert!(controller.begin_face_selection(&scene.meshes()[0], &scene));
    assert_eq!(controller.selected_layer_id(), Some(layer_b_id));

    let Some(restored) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(restored.meshes().len(), original.meshes().len());
    for (restored_entry, original_entry) in restored.meshes().iter().zip(original.meshes()) {
        assert_eq!(restored_entry.id(), original_entry.id());
        assert_eq!(
            restored_entry.mesh.topology_id(),
            original_entry.mesh.topology_id()
        );
    }
}

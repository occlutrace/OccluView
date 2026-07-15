//! Layer-edit orchestration tests, moved verbatim from the single-file module.

use super::super::{
    EditModeCommand, LayerContextAction, LayerContextApply, LayerContextRequest, Scene,
};
use super::apply_visible_selected_face_mesh_edit_action;
use super::repair::{apply_layer_repair_action, LayerRepairOutcome};
use super::selection_ops::apply_selected_face_mesh_edit_action;
use super::undo_redo::apply_layer_mesh_undo_action;
use super::whole_mesh::apply_layer_mesh_edit_action;
use crate::edit_mode::EditModeController;
use glam::{Affine3A, Vec3};
use occluview_core::{Mesh, SceneMesh, ScenePickHit, Vertex};

fn v(x: f32, y: f32, z: f32) -> Vertex {
    Vertex::at(Vec3::new(x, y, z))
}

fn scene_with_islands() -> Option<Scene> {
    let mesh = Mesh::new(
        Some("scan".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(1.0, 1.0, 0.0),
            v(4.0, 0.0, 0.0),
            v(5.0, 0.0, 0.0),
            v(4.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 1, 3, 2, 4, 5, 6],
    );
    assert!(mesh.is_ok(), "valid test mesh should construct");
    let Ok(mesh) = mesh else {
        return None;
    };
    let mut scene = Scene::new();
    scene.add(
        SceneMesh::new(mesh)
            .with_transform(Affine3A::from_translation(Vec3::new(3.0, 2.0, 1.0)))
            .with_tint([0.2, 0.3, 0.4, 1.0])
            .with_opacity(0.42)
            .with_wireframe(true),
    );
    Some(scene)
}

fn scene_with_two_triangles() -> Option<Scene> {
    let mesh = Mesh::new(
        Some("scan".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(2.0, 0.0, 0.0),
            v(3.0, 0.0, 0.0),
            v(2.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok()?;
    let mut scene = Scene::new();
    scene.add(
        SceneMesh::new(mesh)
            .with_transform(Affine3A::from_translation(Vec3::new(3.0, 2.0, 1.0)))
            .with_tint([0.2, 0.3, 0.4, 1.0])
            .with_opacity(0.42)
            .with_wireframe(true),
    );
    Some(scene)
}

fn assert_same_color(left: [f32; 4], right: [f32; 4]) {
    assert!(
        left.into_iter()
            .zip(right)
            .all(|(lhs, rhs)| lhs.to_bits() == rhs.to_bits()),
        "colors differ: left={left:?}, right={right:?}"
    );
}

fn request(scene: &Scene, index: usize, action: LayerContextAction) -> LayerContextRequest {
    LayerContextRequest {
        index,
        layer_id: scene.meshes()[index].id(),
        action,
    }
}

fn clean_tetrahedron() -> Option<Mesh> {
    Mesh::new(
        Some("tetra".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
        ],
        vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
    )
    .ok()
}

fn batch_scene_with_two_layers() -> Option<Scene> {
    let mesh_a = Mesh::new(
        Some("A".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(2.0, 0.0, 0.0),
            v(3.0, 0.0, 0.0),
            v(2.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok()?;
    let mesh_b = Mesh::new(
        Some("B".into()),
        vec![
            v(0.0, 0.0, 1.0),
            v(1.0, 0.0, 1.0),
            v(0.0, 1.0, 1.0),
            v(2.0, 0.0, 1.0),
            v(3.0, 0.0, 1.0),
            v(2.0, 1.0, 1.0),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok()?;
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh_a));
    scene.add(SceneMesh::new(mesh_b));
    Some(scene)
}

fn select_batch_faces(scene: &Scene, edit_mode: &mut EditModeController, triangle: usize) {
    for (index, entry) in scene.meshes().iter().enumerate() {
        assert!(edit_mode.select_face_hit(
            scene,
            ScenePickHit {
                layer_index: index,
                layer_id: entry.id(),
                triangle_index: triangle,
                point: Vec3::ZERO,
                distance: 1.0,
            },
        ));
    }
}

fn batch_scene_signature(scene: &Scene) -> Vec<(u64, usize, Vec<u32>, bool)> {
    scene
        .meshes()
        .iter()
        .map(|entry| {
            (
                entry.id().get(),
                entry.mesh.triangle_count(),
                entry.mesh.indices().to_vec(),
                entry.visible,
            )
        })
        .collect()
}

fn apply_batch(
    scene: &mut Scene,
    edit_mode: &mut EditModeController,
    action: LayerContextAction,
) -> Option<LayerContextApply> {
    let result = apply_visible_selected_face_mesh_edit_action(scene, edit_mode, action);
    assert!(result.is_ok(), "visible batch edit failed: {result:?}");
    result.ok()
}

#[test]
fn visible_selection_batch_deletes_or_crops_every_visible_layer() {
    for action in [
        LayerContextAction::DeleteSelectedFaces,
        LayerContextAction::CropToSelectedFaces,
    ] {
        let Some(mut scene) = batch_scene_with_two_layers() else {
            return;
        };
        let mut edit_mode = EditModeController::new(4, 1_000_000);
        select_batch_faces(&scene, &mut edit_mode, 0);

        let Some(apply) = apply_batch(&mut scene, &mut edit_mode, action) else {
            return;
        };

        assert!(apply.scene_changed);
        assert_eq!(scene.meshes().len(), 2);
        assert_eq!(
            scene
                .meshes()
                .iter()
                .map(|entry| entry.mesh.triangle_count())
                .collect::<Vec<_>>(),
            vec![1; 2]
        );
    }
}

#[test]
fn visible_selection_batch_skips_hidden_layer_byte_for_byte() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);
    scene.meshes_mut()[1].visible = false;
    let hidden_before = format!("{:?}", scene.meshes()[1]);

    let result = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::DeleteSelectedFaces,
    );
    assert!(result.is_some());

    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    assert_eq!(format!("{:?}", scene.meshes()[1]), hidden_before);
}

#[test]
fn visible_selection_batch_refuses_whole_selection_without_touching_other_layers() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id: scene.meshes()[0].id(),
            triangle_index: 0,
            point: Vec3::ZERO,
            distance: 1.0,
        },
    ));
    assert!(edit_mode.begin_face_selection(&scene.meshes()[1].clone(), &scene));
    assert!(edit_mode.select_all_faces());
    let before = batch_scene_signature(&scene);

    let Some(apply) = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::DeleteSelectedFaces,
    ) else {
        return;
    };

    assert!(!apply.scene_changed);
    assert_eq!(batch_scene_signature(&scene), before);
    assert_eq!(edit_mode.undo_layer_id(), None);
}

#[test]
fn visible_selection_batch_rolls_back_when_a_later_layer_fails_preflight() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);
    assert!(edit_mode.begin_face_selection(&scene.meshes()[1].clone(), &scene));
    assert!(edit_mode.select_all_faces());
    let before = batch_scene_signature(&scene);

    let Some(apply) = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::SeparateSelectedComponents,
    ) else {
        return;
    };

    assert!(!apply.scene_changed);
    assert_eq!(batch_scene_signature(&scene), before);
    assert_eq!(edit_mode.undo_layer_id(), None);
}

#[test]
fn visible_selection_batch_has_one_undo_for_all_layers() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let before = scene.clone();
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);

    let result = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::DeleteSelectedFaces,
    );
    assert!(result.is_some());
    assert_eq!(edit_mode.undo_layer_id(), Some(before.meshes()[0].id()));

    let restored = edit_mode.undo_last_scene_edit(&scene, before.meshes()[0].id());
    assert!(
        matches!(
            restored,
            crate::edit_mode::StructuralHistoryStep::Restored(_)
        ),
        "one scene undo should restore the complete batch"
    );
    let crate::edit_mode::StructuralHistoryStep::Restored(restored) = restored else {
        return;
    };
    assert_eq!(
        batch_scene_signature(&restored),
        batch_scene_signature(&before)
    );
}

#[test]
fn visible_selection_batch_undo_redo_restores_the_complete_scene() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let original = scene.clone();
    let focus_layer_id = scene.meshes()[0].id();
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);
    let applied = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::CutSelectionToNewLayer,
    );
    assert!(applied.is_some());
    let edited_signature = batch_scene_signature(&scene);

    let undo = edit_mode.undo_last_scene_edit(&scene, focus_layer_id);
    assert!(
        matches!(undo, crate::edit_mode::StructuralHistoryStep::Restored(_)),
        "batch undo should restore the original scene"
    );
    let crate::edit_mode::StructuralHistoryStep::Restored(undone) = undo else {
        return;
    };
    assert_eq!(
        batch_scene_signature(&undone),
        batch_scene_signature(&original)
    );

    let redo = edit_mode.redo_last_scene_edit(&undone, focus_layer_id);
    assert!(
        matches!(redo, crate::edit_mode::StructuralHistoryStep::Restored(_)),
        "batch redo should restore the complete edited scene"
    );
    let crate::edit_mode::StructuralHistoryStep::Restored(redone) = redo else {
        return;
    };
    assert_eq!(batch_scene_signature(&redone), edited_signature);
}

#[test]
fn visible_selection_batch_rejects_oversized_snapshot_before_mutation() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let before = batch_scene_signature(&scene);
    let mut edit_mode = EditModeController::new(4, 0);
    select_batch_faces(&scene, &mut edit_mode, 0);

    let Some(apply) = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::DeleteSelectedFaces,
    ) else {
        return;
    };

    assert!(!apply.scene_changed);
    assert_eq!(batch_scene_signature(&scene), before);
    assert!(!edit_mode.last_edit_undoable());
    assert_eq!(edit_mode.undo_layer_id(), None);
}

#[test]
fn visible_selection_batch_cut_and_separate_keep_deterministic_source_order() {
    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);
    let cut = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::CutSelectionToNewLayer,
    );
    assert!(cut.is_some());
    assert_eq!(
        scene
            .meshes()
            .iter()
            .map(|entry| entry.mesh.name().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["A", "A", "B", "B"]
    );

    let Some(mut scene) = batch_scene_with_two_layers() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    select_batch_faces(&scene, &mut edit_mode, 0);
    let separate = apply_batch(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::SeparateSelectedComponents,
    );
    assert!(separate.is_some());
    assert_eq!(
        scene
            .meshes()
            .iter()
            .map(|entry| entry.mesh.name().unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["A", "A", "B", "B"]
    );
}

mod holes;
mod operations;
#[path = "tests/repair_tests.rs"]
mod repair_tests;

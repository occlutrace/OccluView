use super::{
    apply_preview_to_scene, BridgeSplitSceneApplyError, BridgeSplitSceneResult, BridgeSplitTarget,
};
use crate::edit_mode::{BusyFinish, EditModeCommand, EditModeController, StructuralHistoryStep};
use glam::{Affine3A, Vec3};
use occluview_core::{BridgeSplitReport, CoreBridgeSplitResult, Mesh, Scene, SceneMesh, Vertex};

fn mesh(name: &str, z: f32) -> Option<Mesh> {
    Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, z)),
            Vertex::at(Vec3::new(1.0, 0.0, z)),
            Vertex::at(Vec3::new(0.0, 1.0, z)),
        ],
        vec![0, 1, 2],
    )
    .ok()
}

fn split_result() -> Option<CoreBridgeSplitResult> {
    Some(CoreBridgeSplitResult {
        part_a: mesh("Bridge - Part A", 0.0)?,
        part_b: mesh("Bridge - Part B", 1.0)?,
        report: BridgeSplitReport::default(),
    })
}

#[test]
fn scene_apply_replaces_source_and_inserts_presentation_matched_part_b() {
    let Some(other) = mesh("Other", -1.0) else {
        return;
    };
    let Some(bridge) = mesh("Bridge", 0.0) else {
        return;
    };
    let Some(tail) = mesh("Tail", 2.0) else {
        return;
    };
    let Some(result) = split_result() else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(other));
    let source_index = scene.add(
        SceneMesh::new(bridge)
            .with_transform(Affine3A::from_translation(Vec3::new(3.0, 4.0, 5.0)))
            .with_tint([0.2, 0.4, 0.6, 1.0])
            .with_opacity(0.45)
            .with_wireframe(true),
    );
    scene.meshes_mut()[source_index].show_orientation = true;
    scene.add(SceneMesh::new(tail));
    let source = scene.meshes()[source_index].clone();
    let target = BridgeSplitTarget::capture(&source);

    let outcome: Result<BridgeSplitSceneResult, BridgeSplitSceneApplyError> =
        apply_preview_to_scene(&scene, target, &result);
    assert!(outcome.is_ok(), "valid preview should apply");
    let Ok(applied) = outcome else {
        return;
    };

    assert_eq!(applied.source_layer_id, source.id());
    assert_eq!(applied.scene.meshes().len(), 4);
    let part_a = &applied.scene.meshes()[source_index];
    let part_b = &applied.scene.meshes()[source_index + 1];
    assert_eq!(part_a.id(), source.id());
    assert_ne!(part_b.id(), source.id());
    assert_eq!(part_b.id(), applied.part_b_layer_id);
    assert_eq!(part_a.mesh.name(), Some("Bridge - Part A"));
    assert_eq!(part_b.mesh.name(), Some("Bridge - Part B"));
    for part in [part_a, part_b] {
        assert_eq!(part.transform, source.transform);
        assert!(part
            .tint
            .into_iter()
            .zip(source.tint)
            .all(|(left, right)| left.to_bits() == right.to_bits()));
        assert_eq!(part.opacity.to_bits(), source.opacity.to_bits());
        assert_eq!(part.visible, source.visible);
        assert_eq!(part.wireframe, source.wireframe);
        assert_eq!(part.show_orientation, source.show_orientation);
    }
    assert_eq!(
        applied.scene.meshes()[source_index + 2].mesh.name(),
        Some("Tail")
    );
}

#[test]
fn scene_apply_rejects_a_stale_or_hidden_target() {
    let Some(bridge) = mesh("Bridge", 0.0) else {
        return;
    };
    let Some(result) = split_result() else {
        return;
    };
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(bridge));
    let target = BridgeSplitTarget::capture(&scene.meshes()[index]);

    scene.meshes_mut()[index].transform = Affine3A::from_translation(Vec3::X);
    assert!(matches!(
        apply_preview_to_scene(&scene, target, &result),
        Err(BridgeSplitSceneApplyError::TargetChanged)
    ));

    scene.meshes_mut()[index].transform = Affine3A::IDENTITY;
    scene.meshes_mut()[index].visible = false;
    assert!(matches!(
        apply_preview_to_scene(&scene, target, &result),
        Err(BridgeSplitSceneApplyError::TargetUnavailable)
    ));
}

#[test]
fn bridge_split_undo_restores_the_exact_pre_split_scene_without_part_b() {
    let Some(bridge) = mesh("Bridge", 0.0) else {
        return;
    };
    let Some(tail) = mesh("Tail", 2.0) else {
        return;
    };
    let Some(result) = split_result() else {
        return;
    };
    let mut scene = Scene::new();
    let source_index = scene.add(SceneMesh::new(bridge));
    scene.add(SceneMesh::new(tail));
    let original = scene.clone();
    let source = scene.meshes()[source_index].clone();
    let target = BridgeSplitTarget::capture(&source);
    let mut edit_mode = EditModeController::new(8, 10_000_000);

    let Some(token) = edit_mode.begin_scene_edit(&scene, source.id(), EditModeCommand::BridgeSplit)
    else {
        return;
    };
    let Ok(applied) = apply_preview_to_scene(&scene, target, &result) else {
        return;
    };
    assert_eq!(
        edit_mode.finish_scene_edit_success(token, &applied.scene),
        BusyFinish::Applied
    );

    let step = edit_mode.undo_last_scene_edit(&applied.scene, source.id());
    assert!(matches!(step, StructuralHistoryStep::Restored(_)));
    let StructuralHistoryStep::Restored(restored) = step else {
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

#[test]
fn both_split_parts_are_immediately_valid_mesh_editor_targets() {
    let Some(bridge) = mesh("Bridge", 0.0) else {
        return;
    };
    let Some(result) = split_result() else {
        return;
    };
    let mut scene = Scene::new();
    let source_index = scene.add(SceneMesh::new(bridge));
    let target = BridgeSplitTarget::capture(&scene.meshes()[source_index]);
    let Ok(applied) = apply_preview_to_scene(&scene, target, &result) else {
        return;
    };
    let part_a = applied.scene.meshes()[source_index].clone();
    let part_b = applied.scene.meshes()[source_index + 1].clone();
    let mut editor = EditModeController::new(8, 10_000_000);

    assert!(editor.begin_face_selection(&part_a, &applied.scene));
    assert_eq!(editor.selected_layer_id(), Some(part_a.id()));
    assert!(editor.begin_face_selection(&part_b, &applied.scene));
    assert_eq!(editor.selected_layer_id(), Some(part_b.id()));
    assert!(editor.begin_face_selection(&part_a, &applied.scene));
    assert_eq!(editor.selected_layer_id(), Some(part_a.id()));
}

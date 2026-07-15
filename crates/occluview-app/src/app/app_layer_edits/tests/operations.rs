use super::*;

#[test]
fn mesh_edit_layer_action_replaces_geometry_and_preserves_layer_material() {
    let Some(mut scene) = scene_with_islands() else {
        return;
    };
    let before_transform = scene.meshes()[0].transform;
    let before_tint = scene.meshes()[0].tint;
    let before_opacity = scene.meshes()[0].opacity;
    let before_wireframe = scene.meshes()[0].wireframe;
    let before_indices = scene.meshes()[0].mesh.indices().to_vec();

    let action = request(&scene, 0, LayerContextAction::InvertNormals);
    let apply = apply_layer_mesh_edit_action(&mut scene, action, None);
    assert!(apply.is_ok(), "invert-normals action should succeed");
    let Ok((apply, _report)) = apply else {
        return;
    };

    assert!(apply.scene_changed);
    assert!(apply.structural_scene_change);
    // Geometry is replaced (winding flipped) while the triangle count and every
    // display attribute survive unchanged.
    assert_ne!(scene.meshes()[0].mesh.indices(), before_indices.as_slice());
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 3);
    assert_eq!(scene.meshes()[0].transform, before_transform);
    assert_same_color(scene.meshes()[0].tint, before_tint);
    assert_eq!(
        scene.meshes()[0].opacity.to_bits(),
        before_opacity.to_bits()
    );
    assert_eq!(scene.meshes()[0].wireframe, before_wireframe);
}

#[test]
fn mesh_edit_layer_action_rejects_point_cloud_without_mutating_scene() {
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(Mesh::point_cloud(
        Some("cloud".into()),
        vec![v(0.0, 0.0, 0.0)],
    )));

    let action = request(&scene, 0, LayerContextAction::InvertNormals);
    let err = apply_layer_mesh_edit_action(&mut scene, action, None);
    assert!(err.is_err(), "point cloud edit should be rejected");
    let Err(err) = err else {
        return;
    };

    assert!(err.to_string().contains("point cloud"));
    assert!(scene.meshes()[0].mesh.is_point_cloud());
}

#[test]
fn mesh_edit_layer_action_ignores_stale_layer_identity_without_mutating_scene() {
    let Some(mut scene) = scene_with_islands() else {
        return;
    };
    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
    let before_triangle_count = scene.meshes()[0].mesh.triangle_count();

    let apply = apply_layer_mesh_edit_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id: stale_layer_id,
            action: LayerContextAction::InvertNormals,
        },
        None,
    );

    assert!(apply.is_ok());
    let Ok((apply, _report)) = apply else {
        return;
    };
    assert!(!apply.scene_changed);
    assert_eq!(
        scene.meshes()[0].mesh.triangle_count(),
        before_triangle_count
    );
}

#[test]
fn undo_layer_mesh_edit_restores_geometry_and_keeps_current_display_state() {
    let Some(mut scene) = scene_with_islands() else {
        return;
    };
    let request = request(&scene, 0, LayerContextAction::InvertNormals);
    let before_indices = scene.meshes()[0].mesh.indices().to_vec();
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let Some(token) =
        edit_mode.begin_layer_edit(&scene.meshes()[0], EditModeCommand::InvertNormals)
    else {
        return;
    };

    let apply = apply_layer_mesh_edit_action(&mut scene, request, None);
    assert!(apply.is_ok(), "mesh edit should succeed");
    let Ok((apply, _report)) = apply else {
        return;
    };
    assert!(apply.scene_changed);
    assert_eq!(
        edit_mode.finish_layer_edit_success(token),
        crate::edit_mode::BusyFinish::Applied
    );
    assert_ne!(scene.meshes()[0].mesh.indices(), before_indices.as_slice());

    let current_transform = Affine3A::from_translation(Vec3::new(9.0, 8.0, 7.0));
    let current_tint = [0.9, 0.1, 0.2, 1.0];
    scene.meshes_mut()[0].transform = current_transform;
    scene.meshes_mut()[0].tint = current_tint;
    scene.meshes_mut()[0].opacity = 0.25;
    scene.meshes_mut()[0].visible = false;
    scene.meshes_mut()[0].wireframe = false;

    let apply = apply_layer_mesh_undo_action(
        &mut scene,
        LayerContextRequest {
            action: LayerContextAction::UndoLastMeshEdit,
            ..request
        },
        &mut edit_mode,
    );

    assert!(apply.scene_changed);
    assert!(apply.structural_scene_change);
    assert_eq!(scene.meshes()[0].mesh.indices(), before_indices.as_slice());
    assert_eq!(scene.meshes()[0].transform, current_transform);
    assert_same_color(scene.meshes()[0].tint, current_tint);
    assert_eq!(scene.meshes()[0].opacity.to_bits(), 0.25f32.to_bits());
    assert!(!scene.meshes()[0].visible);
    assert!(!scene.meshes()[0].wireframe);
}

#[test]
fn undo_layer_mesh_edit_ignores_stale_layer_identity_without_popping_history() {
    let Some(mut scene) = scene_with_islands() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let Some(token) =
        edit_mode.begin_layer_edit(&scene.meshes()[0], EditModeCommand::InvertNormals)
    else {
        return;
    };
    assert_eq!(
        edit_mode.finish_layer_edit_success(token),
        crate::edit_mode::BusyFinish::Applied
    );
    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
    let before_triangle_count = scene.meshes()[0].mesh.triangle_count();

    let apply = apply_layer_mesh_undo_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id: stale_layer_id,
            action: LayerContextAction::UndoLastMeshEdit,
        },
        &mut edit_mode,
    );

    assert!(!apply.scene_changed);
    assert_eq!(
        scene.meshes()[0].mesh.triangle_count(),
        before_triangle_count
    );
    assert_eq!(edit_mode.undo_layer_id(), Some(scene.meshes()[0].id()));
}

#[test]
fn selected_face_mesh_edit_deletes_selection_and_records_undo() {
    let Some(mut scene) = scene_with_two_triangles() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer_id = scene.meshes()[0].id();
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 1,
            point: Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let before_transform = scene.meshes()[0].transform;
    let before_tint = scene.meshes()[0].tint;
    let before_opacity = scene.meshes()[0].opacity;
    let before_wireframe = scene.meshes()[0].wireframe;
    let request = request(&scene, 0, LayerContextAction::DeleteSelectedFaces);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);

    assert!(apply.is_ok(), "delete selected faces should succeed");
    let Ok(apply) = apply else {
        return;
    };
    assert!(apply.scene_changed);
    assert!(apply.structural_scene_change);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[0].mesh.indices(), &[0, 1, 2]);
    assert_eq!(scene.meshes()[0].transform, before_transform);
    assert_same_color(scene.meshes()[0].tint, before_tint);
    assert_eq!(
        scene.meshes()[0].opacity.to_bits(),
        before_opacity.to_bits()
    );
    assert_eq!(scene.meshes()[0].wireframe, before_wireframe);
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
    // Product flow runs sync after the scene change: the stale mask (sized
    // for the pre-op topology) is cleared there, not by the op itself.
    edit_mode.sync_to_scene(&scene);
    assert_eq!(edit_mode.selected_layer_id(), None);
}

#[test]
fn selected_face_mesh_edit_refuses_whole_mesh_selection() {
    let Some(mut scene) = scene_with_two_triangles() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer = scene.meshes()[0].clone();
    assert!(edit_mode.begin_face_selection(&layer, &scene));
    assert!(edit_mode.select_all_faces());

    for action in [
        LayerContextAction::DeleteSelectedFaces,
        LayerContextAction::CropToSelectedFaces,
        LayerContextAction::CutSelectionToNewLayer,
        LayerContextAction::SeparateSelectedComponents,
    ] {
        let request = request(&scene, 0, action);
        let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);
        assert!(apply.is_ok(), "whole-mesh refusal must not error");
        let Ok(apply) = apply else {
            return;
        };
        // Refused honestly: nothing changed, no phantom undo, not dirty.
        assert!(!apply.scene_changed);
        assert_eq!(scene.meshes().len(), 1);
        assert_eq!(scene.meshes()[0].mesh.triangle_count(), 2);
        assert_eq!(edit_mode.undo_layer_id(), None);
        assert!(!edit_mode.is_dirty());
    }
}

#[test]
fn whole_mesh_layer_noop_discards_snapshot_and_stays_clean() {
    // Watertight tetrahedron: with no open rims, Close holes is a content no-op.
    let Some(mesh) = clean_tetrahedron() else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[0].clone();
    let mut edit_mode = EditModeController::new(4, 1_000_000);

    let request = request(&scene, 0, LayerContextAction::CloseHoles);
    let Some(token) = edit_mode.begin_layer_edit(&layer, EditModeCommand::CloseHoles) else {
        return;
    };
    let apply = apply_layer_mesh_edit_action(&mut scene, request, None);
    assert!(
        apply.is_ok(),
        "closing holes on a watertight mesh must not error"
    );
    let Ok((apply, _report)) = apply else {
        return;
    };

    // Content no-op: the mesh is untouched and the caller finishes the op
    // via the no-op path, which discards the pre-op snapshot.
    assert!(!apply.scene_changed);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4);
    let _ = edit_mode.finish_layer_edit_noop(token);
    assert_eq!(edit_mode.undo_layer_id(), None);
    assert!(!edit_mode.is_dirty());
}

#[test]
fn selected_face_mesh_edit_crops_to_selection_and_ignores_stale_layer() {
    let Some(mut scene) = scene_with_two_triangles() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer_id = scene.meshes()[0].id();
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 1,
            point: Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
    let stale = apply_selected_face_mesh_edit_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id: stale_layer_id,
            action: LayerContextAction::CropToSelectedFaces,
        },
        &mut edit_mode,
    );
    assert!(stale.is_ok(), "stale layer should be ignored without error");
    let Ok(stale) = stale else {
        return;
    };
    assert!(!stale.scene_changed);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 2);
    assert_eq!(edit_mode.selected_layer_id(), Some(layer_id));

    let request = request(&scene, 0, LayerContextAction::CropToSelectedFaces);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);

    assert!(apply.is_ok(), "crop selected faces should succeed");
    let Ok(apply) = apply else {
        return;
    };
    assert!(apply.scene_changed);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[0].mesh.indices(), &[0, 1, 2]);
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
    // Product flow runs sync after the scene change: the stale mask (sized
    // for the pre-op topology) is cleared there, not by the op itself.
    edit_mode.sync_to_scene(&scene);
    assert_eq!(edit_mode.selected_layer_id(), None);
}

#[test]
fn selected_face_mesh_cut_creates_new_layer_and_scene_undo_restores_structure() {
    let Some(mut scene) = scene_with_two_triangles() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer_id = scene.meshes()[0].id();
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 1,
            point: Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let before_transform = scene.meshes()[0].transform;
    let before_tint = scene.meshes()[0].tint;
    let before_opacity = scene.meshes()[0].opacity;
    let before_wireframe = scene.meshes()[0].wireframe;

    let request = request(&scene, 0, LayerContextAction::CutSelectionToNewLayer);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);

    assert!(apply.is_ok(), "cut selected faces should succeed");
    let Ok(apply) = apply else {
        return;
    };
    assert!(apply.scene_changed);
    assert!(apply.structural_scene_change);
    assert_eq!(scene.meshes().len(), 2);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[0].transform, before_transform);
    assert_eq!(scene.meshes()[1].transform, before_transform);
    assert_same_color(scene.meshes()[0].tint, before_tint);
    // The extracted layer is coincident with where it sat in the source; with
    // the source tint the cut would be invisible on screen, so it steps the
    // palette (exocad shows a divide by color).
    assert_same_color(
        scene.meshes()[1].tint,
        crate::layer_actions::next_layer_tint(before_tint),
    );
    assert_eq!(
        scene.meshes()[1].opacity.to_bits(),
        before_opacity.to_bits()
    );
    assert_eq!(scene.meshes()[1].wireframe, before_wireframe);
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
    // Product flow runs sync after the scene change: the stale mask (sized
    // for the pre-op topology) is cleared there, not by the op itself.
    edit_mode.sync_to_scene(&scene);
    assert_eq!(edit_mode.selected_layer_id(), None);

    let undo = apply_layer_mesh_undo_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id,
            action: LayerContextAction::UndoLastMeshEdit,
        },
        &mut edit_mode,
    );

    assert!(undo.scene_changed);
    assert!(undo.structural_scene_change);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 2);
    assert_eq!(scene.meshes()[0].id(), layer_id);
}

#[test]
fn structural_undo_is_refused_when_a_layer_is_appended_after_the_cut() {
    // Scenario 6: cut spawns a new layer, then the operator appends ANOTHER
    // layer (a separate load). Undoing the cut would restore the pre-cut
    // whole-scene snapshot and silently delete the appended layer, so it is
    // refused: the scene is left untouched (the wrapper reports the honest
    // "scene changed since" status).
    let Some(mut scene) = scene_with_two_triangles() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer_id = scene.meshes()[0].id();
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 1,
            point: Vec3::new(2.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));

    let request = request(&scene, 0, LayerContextAction::CutSelectionToNewLayer);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);
    assert!(apply.is_ok(), "cut should succeed");
    let Ok(apply) = apply else {
        return;
    };
    assert!(apply.structural_scene_change);
    assert_eq!(scene.meshes().len(), 2);
    edit_mode.sync_to_scene(&scene);

    // The operator appends a fresh layer after the cut.
    let appended = Mesh::new(
        Some("appended".into()),
        vec![v(9.0, 0.0, 0.0), v(10.0, 0.0, 0.0), v(9.0, 1.0, 0.0)],
        vec![0, 1, 2],
    );
    assert!(appended.is_ok(), "appended mesh should build");
    let Ok(appended) = appended else {
        return;
    };
    scene.add(SceneMesh::new(appended));
    edit_mode.sync_to_scene(&scene);
    let appended_id = scene.meshes()[2].id();

    let undo = apply_layer_mesh_undo_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id,
            action: LayerContextAction::UndoLastMeshEdit,
        },
        &mut edit_mode,
    );

    // Refused: the scene keeps all three layers, and the structural snapshot is
    // left on the stack (not popped) so the appended layer is never dropped.
    assert!(!undo.scene_changed);
    assert_eq!(scene.meshes().len(), 3);
    assert_eq!(scene.meshes()[2].id(), appended_id);
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
}

#[test]
fn selected_face_mesh_separate_splits_components_into_multiple_layers_and_undo_restores() {
    let Some(mut scene) = scene_with_islands() else {
        return;
    };
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let layer_id = scene.meshes()[0].id();
    assert!(edit_mode.select_face_hit(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 0,
            point: Vec3::new(0.25, 0.25, 0.0),
            distance: 10.0,
        },
    ));
    // Plain click accumulates into the highlight (exocad convention).
    assert!(edit_mode.select_face_hit_with_mode(
        &scene,
        ScenePickHit {
            layer_index: 0,
            layer_id,
            triangle_index: 2,
            point: Vec3::new(4.25, 0.25, 0.0),
            distance: 10.0,
        },
        false,
    ));

    let before_transform = scene.meshes()[0].transform;
    let before_tint = scene.meshes()[0].tint;

    let request = request(&scene, 0, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode);

    assert!(apply.is_ok(), "separate selected components should succeed");
    let Ok(apply) = apply else {
        return;
    };
    assert!(apply.scene_changed);
    assert!(apply.structural_scene_change);
    assert_eq!(scene.meshes().len(), 3);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[2].mesh.triangle_count(), 1);
    assert_eq!(scene.meshes()[1].transform, before_transform);
    assert_eq!(scene.meshes()[2].transform, before_transform);
    // Spawned parts step the palette so the divide is visible on screen.
    let first_part_tint = crate::layer_actions::next_layer_tint(before_tint);
    assert_same_color(scene.meshes()[1].tint, first_part_tint);
    assert_same_color(
        scene.meshes()[2].tint,
        crate::layer_actions::next_layer_tint(first_part_tint),
    );
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
    // Product flow runs sync after the scene change: the stale mask (sized
    // for the pre-op topology) is cleared there, not by the op itself.
    edit_mode.sync_to_scene(&scene);
    assert_eq!(edit_mode.selected_layer_id(), None);

    let undo = apply_layer_mesh_undo_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id,
            action: LayerContextAction::UndoLastMeshEdit,
        },
        &mut edit_mode,
    );

    assert!(undo.scene_changed);
    assert!(undo.structural_scene_change);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 3);
    assert_eq!(scene.meshes()[0].id(), layer_id);
}

#[test]
fn layer_edit_module_exposes_undo_orchestration_without_keyboard_input() {
    let source = include_str!("../undo_redo.rs").replace("\r\n", "\n");
    let production_source = source.as_str();

    assert!(
        production_source.contains("apply_last_mesh_edit_undo_with_status"),
        "viewport shortcut and layer context menu should share this undo path"
    );
    assert!(
        !production_source.contains("consume_key("),
        "layer edit module should not own keyboard input plumbing"
    );
}

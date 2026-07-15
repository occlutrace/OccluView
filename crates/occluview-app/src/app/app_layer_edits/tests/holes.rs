use super::*;

fn scene_with_a_tube() -> Option<Scene> {
    let mut vertices = Vec::new();
    for ring in 0..2 {
        let (radius, z) = if ring == 0 { (4.0, 0.0) } else { (0.5, 2.0) };
        for step in 0..8 {
            #[allow(clippy::cast_precision_loss)]
            let angle = std::f32::consts::TAU * f32::from(u8::try_from(step).unwrap_or(0)) / 8.0;
            vertices.push(v(radius * angle.cos(), radius * angle.sin(), z));
        }
    }
    let mut indices = Vec::new();
    for step in 0..8u32 {
        let next = (step + 1) % 8;
        let (a, b, c, d) = (step, next, 8 + step, 8 + next);
        indices.extend_from_slice(&[a, b, c, b, d, c]);
    }
    let mesh = Mesh::new(Some("tube".into()), vertices, indices).ok()?;
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    Some(scene)
}

fn tube_bottom_selection() -> FaceSelection {
    FaceSelection::new((0..16).map(|face| face % 2 == 0).collect())
}

fn boundary_edge_count(indices: &[u32]) -> usize {
    let mut edges = std::collections::BTreeMap::<(u32, u32), usize>::new();
    for triangle in indices.chunks_exact(3) {
        for [first, second] in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            let edge = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            *edges.entry(edge).or_default() += 1;
        }
    }
    edges.values().filter(|count| **count == 1).count()
}

#[test]
fn close_holes_button_requires_selection_and_scopes_to_selected_rim() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let request = request(&scene, 0, LayerContextAction::CloseHoles);
    let before = scene.meshes()[0].mesh.triangle_count();

    let no_selection = super::super::whole_mesh::apply_layer_mesh_edit_action_with_limit(
        &mut scene, request, None, None,
    );
    assert!(no_selection.is_ok());
    let Ok((no_selection_apply, no_selection_report)) = no_selection else {
        return;
    };
    assert!(
        !no_selection_apply.scene_changed,
        "Close Holes must not widen an empty selection to the whole mesh"
    );
    assert!(no_selection_report.is_none());
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), before);

    // Mark only the lower rim's owning faces. The opposite rim is left open,
    // exactly like an exocad selection that does not fully cover it.
    let selection = tube_bottom_selection();
    let selected = super::super::whole_mesh::apply_layer_mesh_edit_action_with_limit(
        &mut scene,
        request,
        Some(&selection),
        None,
    );
    assert!(selected.is_ok());
    let Ok((selected_apply, selected_report)) = selected else {
        return;
    };
    assert!(selected_apply.scene_changed);
    let Some(report) = selected_report else {
        return;
    };
    assert_eq!(report.filled_holes, 1);
    assert_eq!(boundary_edge_count(scene.meshes()[0].mesh.indices()), 8);
    assert!(scene.meshes()[0].mesh.triangle_count() > before);
}

#[test]
fn close_holes_without_selection_is_a_noop() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let request = request(&scene, 0, LayerContextAction::CloseHoles);
    let before = scene.meshes()[0].mesh.indices().to_vec();
    let result = apply_layer_mesh_edit_action(&mut scene, request, None);
    assert!(result.is_ok());
    let Ok((apply, report)) = result else {
        return;
    };
    assert!(!apply.scene_changed);
    assert!(report.is_none());
    assert_eq!(scene.meshes()[0].mesh.indices(), before.as_slice());
}

#[test]
fn mesh_editor_close_holes_requires_face_selection() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    scene.add(SceneMesh::new(scene.meshes()[0].mesh.clone()));
    let before = scene.clone();
    let mut edit_mode = EditModeController::new(4, 1_000_000);

    let result = apply_visible_selected_face_mesh_edit_action(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::CloseHoles,
    );
    assert!(result.is_ok(), "mesh-editor close holes failed: {result:?}");
    let Ok(apply) = result else { return };

    assert!(
        !apply.scene_changed,
        "empty selection must be an honest no-op"
    );
    assert_eq!(
        scene.meshes()[0].mesh.indices(),
        before.meshes()[0].mesh.indices()
    );
    assert_eq!(
        scene.meshes()[1].mesh.indices(),
        before.meshes()[1].mesh.indices()
    );
    assert_eq!(edit_mode.undo_layer_id(), None);
}

#[test]
fn mesh_editor_close_holes_scopes_to_selected_visible_layers() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let mut hidden = SceneMesh::new(scene.meshes()[0].mesh.clone());
    hidden.visible = false;
    scene.add(hidden);
    let hidden_before = format!("{:?}", scene.meshes()[1]);
    let visible_before = scene.meshes()[0].mesh.triangle_count();
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let visible = scene.meshes()[0].clone();
    assert!(edit_mode.begin_face_selection(&visible, &scene));
    for triangle_index in [0, 2, 4, 6] {
        assert!(edit_mode.select_face_hit(
            &scene,
            ScenePickHit {
                layer_index: 0,
                layer_id: visible.id(),
                triangle_index,
                point: Vec3::ZERO,
                distance: 1.0,
            },
        ));
    }

    let result = apply_visible_selected_face_mesh_edit_action(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::CloseHoles,
    );
    assert!(result.is_ok(), "mesh-editor close holes failed: {result:?}");
    let Ok(apply) = result else { return };

    assert!(apply.scene_changed);
    assert!(scene.meshes()[0].mesh.triangle_count() > visible_before);
    assert_eq!(boundary_edge_count(scene.meshes()[0].mesh.indices()), 8);
    assert_eq!(format!("{:?}", scene.meshes()[1]), hidden_before);
}

#[test]
fn non_face_edit_action_errors_instead_of_aborting() {
    use super::super::selection_ops::selected_face_edit_result;
    use occluview_core::{CoreError, FaceSelection};

    let Ok(mesh) = Mesh::new(
        Some("m".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    ) else {
        return;
    };
    let selection = FaceSelection::new(vec![true]);

    // ToggleVisibility is not a face edit. The adapter must degrade to an honest
    // error (which callers surface as a failed edit), never `unreachable!` —
    // release ships `panic = "abort"`, so that would be a hard process crash.
    let result = selected_face_edit_result(&mesh, &selection, LayerContextAction::ToggleVisibility);
    assert!(
        matches!(result, Err(CoreError::Geometry(_))),
        "non-face-edit action should return an error, got {result:?}"
    );
}

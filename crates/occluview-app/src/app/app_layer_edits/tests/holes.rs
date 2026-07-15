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

#[test]
fn close_holes_button_respects_the_mm_perimeter_budget() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let request = request(&scene, 0, LayerContextAction::CloseHoles);
    let before = scene.meshes()[0].mesh.triangle_count();

    // ~3.1 mm interior rim: a 2 mm restraint refuses it (border kept too).
    let tight = super::super::whole_mesh::apply_layer_mesh_edit_action_with_limit(
        &mut scene,
        request,
        None,
        Some(2.0),
    );
    assert!(tight.is_ok());
    let Ok((tight_apply, tight_report)) = tight else {
        return;
    };
    assert!(
        !tight_apply.scene_changed,
        "2 mm restraint must not fill a 3.1 mm rim"
    );
    let Some(report) = tight_report else { return };
    assert_eq!(report.filled_holes, 0);
    assert_eq!(report.skipped_oversize_rims, 1);
    assert_eq!(report.skipped_border_rims, 1);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), before);

    // A 10 mm restraint fills the interior hole; the border stays open.
    let generous = super::super::whole_mesh::apply_layer_mesh_edit_action_with_limit(
        &mut scene,
        request,
        None,
        Some(10.0),
    );
    assert!(generous.is_ok());
    let Ok((generous_apply, generous_report)) = generous else {
        return;
    };
    assert!(generous_apply.scene_changed);
    let Some(report) = generous_report else {
        return;
    };
    assert_eq!(report.filled_holes, 1);
    assert_eq!(report.skipped_border_rims, 1);
    assert!(scene.meshes()[0].mesh.triangle_count() > before);
}

#[test]
fn close_holes_without_a_limit_closes_interior_holes_and_keeps_the_border() {
    // The default (no mm restraint): every interior hole closes regardless of
    // size — the exocad behavior — while the scan border stays open.
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let request = request(&scene, 0, LayerContextAction::CloseHoles);
    let result = apply_layer_mesh_edit_action(&mut scene, request, None);
    assert!(result.is_ok());
    let Ok((apply, report)) = result else {
        return;
    };
    assert!(apply.scene_changed);
    let Some(report) = report else { return };
    assert_eq!(report.filled_holes, 1);
    assert_eq!(report.skipped_border_rims, 1);
    assert_eq!(report.skipped_damaged_rims, 0);
}

#[test]
fn mesh_editor_close_holes_repairs_every_visible_layer_without_a_selection() {
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
        apply.scene_changed,
        "both visible tube rims should be closed"
    );
    assert!(
        scene
            .meshes()
            .iter()
            .all(|entry| entry.mesh.triangle_count() > before.meshes()[0].mesh.triangle_count()),
        "every visible layer should receive the whole-mesh repair"
    );
    let restored = edit_mode.undo_last_scene_edit(&scene, before.meshes()[0].id());
    assert!(
        matches!(
            restored,
            crate::edit_mode::StructuralHistoryStep::Restored(_)
        ),
        "close holes should create one scene-wide undo step"
    );
    let crate::edit_mode::StructuralHistoryStep::Restored(restored) = restored else {
        return;
    };
    assert_eq!(
        restored
            .meshes()
            .iter()
            .map(|entry| entry.mesh.triangle_count())
            .collect::<Vec<_>>(),
        before
            .meshes()
            .iter()
            .map(|entry| entry.mesh.triangle_count())
            .collect::<Vec<_>>()
    );
}

#[test]
fn mesh_editor_close_holes_leaves_hidden_layers_byte_for_byte_untouched() {
    let Some(mut scene) = scene_with_a_tube() else {
        return;
    };
    let mut hidden = SceneMesh::new(scene.meshes()[0].mesh.clone());
    hidden.visible = false;
    scene.add(hidden);
    let hidden_before = format!("{:?}", scene.meshes()[1]);
    let visible_before = scene.meshes()[0].mesh.triangle_count();
    let mut edit_mode = EditModeController::new(4, 1_000_000);

    let result = apply_visible_selected_face_mesh_edit_action(
        &mut scene,
        &mut edit_mode,
        LayerContextAction::CloseHoles,
    );
    assert!(result.is_ok(), "mesh-editor close holes failed: {result:?}");
    let Ok(apply) = result else { return };

    assert!(apply.scene_changed);
    assert!(scene.meshes()[0].mesh.triangle_count() > visible_before);
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

use super::*;

fn english() -> crate::i18n::LocaleManager {
    crate::i18n::LocaleManager::for_tests()
}

/// The same tetrahedron with one face duplicated — one defect for the
/// duplicate-face pass, everything else already clean.
fn dirty_tetrahedron() -> Option<Mesh> {
    Mesh::new(
        Some("dirty tetra".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
        ],
        vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2, 0, 2, 1],
    )
    .ok()
}

#[test]
fn repair_layer_action_repairs_defects_and_records_single_undo() {
    let Some(mesh) = dirty_tetrahedron() else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[0].id();
    let mut edit_mode = EditModeController::new(4, 1_000_000);
    let Some(token) = edit_mode.begin_layer_edit(&scene.meshes()[0], EditModeCommand::RepairMesh)
    else {
        return;
    };

    let repair_request = request(&scene, 0, LayerContextAction::RepairMesh);
    let outcome = apply_layer_repair_action(&mut scene, repair_request);
    assert!(
        matches!(outcome, Ok(LayerRepairOutcome::Repaired(_))),
        "duplicate-face tetrahedron must come back repaired"
    );
    let Ok(LayerRepairOutcome::Repaired(report)) = outcome else {
        return;
    };

    assert_eq!(report.removed_duplicate_triangles, 1);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4);
    assert_eq!(
        edit_mode.finish_layer_edit_success(token),
        crate::edit_mode::BusyFinish::Applied
    );
    // The whole pipeline is one undo step.
    assert_eq!(edit_mode.undo_len(), 1);
    assert_eq!(edit_mode.undo_layer_id(), Some(layer_id));
}

#[test]
fn repair_layer_noop_pushes_no_undo_and_preserves_redo() {
    let (Some(dirty), Some(clean)) = (dirty_tetrahedron(), clean_tetrahedron()) else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(dirty));
    scene.add(SceneMesh::new(clean));
    let dirty_layer_id = scene.meshes()[0].id();
    let mut edit_mode = EditModeController::new(4, 1_000_000);

    // Build a redo entry: repair the defective layer, then undo it.
    let Some(token) = edit_mode.begin_layer_edit(&scene.meshes()[0], EditModeCommand::RepairMesh)
    else {
        return;
    };
    let repair_request = request(&scene, 0, LayerContextAction::RepairMesh);
    let outcome = apply_layer_repair_action(&mut scene, repair_request);
    assert!(matches!(outcome, Ok(LayerRepairOutcome::Repaired(_))));
    let _ = edit_mode.finish_layer_edit_success(token);
    let undo_request = request(&scene, 0, LayerContextAction::UndoLastMeshEdit);
    let undo = apply_layer_mesh_undo_action(&mut scene, undo_request, &mut edit_mode);
    assert!(undo.scene_changed);
    assert_eq!(edit_mode.undo_len(), 0);
    assert_eq!(edit_mode.redo_layer_id(), Some(dirty_layer_id));

    // Repairing the already-clean layer is a content no-op: no undo step is
    // recorded, and the redo history survives unchanged.
    let Some(token) = edit_mode.begin_layer_edit(&scene.meshes()[1], EditModeCommand::RepairMesh)
    else {
        return;
    };
    let noop_request = request(&scene, 1, LayerContextAction::RepairMesh);
    let outcome = apply_layer_repair_action(&mut scene, noop_request);
    assert!(
        matches!(outcome, Ok(LayerRepairOutcome::Clean(_))),
        "clean tetrahedron must come back untouched"
    );
    let Ok(LayerRepairOutcome::Clean(report)) = outcome else {
        return;
    };
    assert!(!report.changed_content());
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 4);
    let _ = edit_mode.finish_layer_edit_noop(token);
    assert_eq!(edit_mode.undo_len(), 0);
    assert_eq!(edit_mode.redo_layer_id(), Some(dirty_layer_id));
}

#[test]
fn repair_layer_action_ignores_stale_layer_identity_without_mutating_scene() {
    let Some(mesh) = dirty_tetrahedron() else {
        return;
    };
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    let stale_layer_id = SceneMesh::new(Mesh::empty()).id();

    let outcome = apply_layer_repair_action(
        &mut scene,
        LayerContextRequest {
            index: 0,
            layer_id: stale_layer_id,
            action: LayerContextAction::RepairMesh,
        },
    );

    assert!(matches!(outcome, Ok(LayerRepairOutcome::Stale)));
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 5);
}

#[test]
fn repair_status_lines_report_only_what_happened() {
    use occluview_core::{MeshEditWarning, RepairReport};

    // Already clean, no open rims.
    let clean = RepairReport::default();
    assert_eq!(
        super::super::repair::clean_status("scan.stl", &clean, &english()),
        "Mesh is already clean: \u{2068}scan.stl\u{2069}"
    );

    // Already clean, but the natural boundary stays open — say so.
    let with_rims = RepairReport {
        open_rims_left: 2,
        ..RepairReport::default()
    };
    // Embedded selects isolate twice (whole placeable + inner variable).
    assert_eq!(
        super::super::repair::clean_status("scan.stl", &with_rims, &english()),
        "Mesh is already clean: \u{2068}scan.stl\u{2069}, \u{2068}\u{2068}2\u{2069} open rims left\u{2069}"
    );
    let one_rim = RepairReport {
        open_rims_left: 1,
        ..RepairReport::default()
    };
    assert_eq!(
        super::super::repair::clean_status("scan.stl", &one_rim, &english()),
        "Mesh is already clean: \u{2068}scan.stl\u{2069}, \u{2068}\u{2068}1\u{2069} open rim left\u{2069}"
    );

    // Repaired: only non-zero counts appear, in pipeline order, with correct
    // singular/plural, plus the skipped-rims tail when warnings exist.
    // Interpolated numbers carry Fluent bidi isolation marks by design.
    let multi = RepairReport {
        welded_vertices: 1240,
        removed_degenerate_triangles: 86,
        removed_duplicate_triangles: 12,
        split_nonmanifold_edges: 3,
        split_bowtie_vertices: 2,
        reoriented_triangles: 154,
        removed_debris_components: 4,
        filled_holes: 12,
        warnings: vec![MeshEditWarning::DegenerateGeometry],
        ..RepairReport::default()
    };
    assert_eq!(
        super::super::repair::repaired_status("scan.stl", &multi, &english()),
        "Repaired \u{2068}scan.stl\u{2069}: \u{2068}welded \u{2068}1240\u{2069} vertices, removed \u{2068}86\u{2069} slivers, \u{2068}12\u{2069} duplicate faces, \
         fixed \u{2068}3\u{2069} non-manifold edges, split \u{2068}2\u{2069} bowties, reoriented \u{2068}154\u{2069} triangles, \
         dropped \u{2068}4\u{2069} debris parts, closed \u{2068}12\u{2069} pinholes, \u{2068}1\u{2069} rim skipped (non-simple)\u{2069}"
    );

    let single = RepairReport {
        welded_vertices: 1,
        filled_holes: 1,
        ..RepairReport::default()
    };
    assert_eq!(
        super::super::repair::repaired_status("scan.stl", &single, &english()),
        "Repaired \u{2068}scan.stl\u{2069}: \u{2068}welded \u{2068}1\u{2069} vertex, closed \u{2068}1\u{2069} pinhole\u{2069}"
    );
}

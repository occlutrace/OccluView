//! Session-lifecycle, sync re-arming, redo, and no-op controller tests.

use super::*;
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};

pub(super) fn triangle_mesh(name: &str) -> Option<Mesh> {
    Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .ok()
}

pub(super) fn two_triangle_mesh(name: &str) -> Option<Mesh> {
    Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(glam::Vec3::new(2.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(3.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(2.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .ok()
}

#[test]
fn begin_face_selection_captures_baseline_scene_once_per_session() {
    let Some(mesh) = two_triangle_mesh("baseline") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    // Captured baseline → Cancel returns it and ends the session.
    let Some(restored) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(restored.meshes().len(), 1);
    assert_eq!(
        restored.meshes()[0].mesh.triangle_count(),
        layer.mesh.triangle_count()
    );
    // Session closed: a second cancel has no baseline to restore.
    assert!(controller.cancel_edit_session().is_none());
}

#[test]
fn finish_edit_session_keeps_undo_history_for_stepwise_undo() {
    let Some(before_mesh) = triangle_mesh("before") else {
        return;
    };
    let Some(after_mesh) = triangle_mesh("after") else {
        return;
    };
    let layer = SceneMesh::new(before_mesh);
    let mut current = layer.clone();
    current.mesh = after_mesh;
    let mut scene = Scene::new();
    scene.add(layer.clone());
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert!(controller.is_dirty());
    assert_eq!(controller.undo_len(), 1);

    controller.finish_edit_session();

    // Done closes the session (no dirty, no baseline) but the undo stack
    // remains so Ctrl-Z can still revert individual mesh ops afterwards.
    assert!(!controller.is_dirty());
    assert_eq!(controller.undo_len(), 1);
    assert_eq!(controller.undo_layer_id(), Some(layer.id()));
    // Cancel is no longer available: nothing to revert to.
    assert!(controller.cancel_edit_session().is_none());
    // Undo still works against the current layer.
    let restored = controller.undo_last_layer_edit(&current);
    assert!(restored.is_some());
}

#[test]
fn cancel_edit_session_reverts_structural_edit_and_clears_history() {
    let Some(mesh) = two_triangle_mesh("session") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&scene.meshes()[layer_index], &scene));
    // A structural edit pushes a full-scene undo snapshot and mutates the
    // scene (simulate the cut/separate outcome: a layer is appended).
    let Some(token) =
        controller.begin_scene_edit(&scene, layer_id, EditModeCommand::CutSelectionToNewLayer)
    else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    scene.add(SceneMesh::new(Mesh::empty()));
    assert_eq!(scene.meshes().len(), 2);
    assert!(controller.is_dirty());

    let Some(restored) = controller.cancel_edit_session() else {
        return;
    };
    // Cancel reverts to the pre-session baseline (one layer), dropping the
    // structural addition regardless of the capped per-op snapshot stack.
    assert_eq!(restored.meshes().len(), 1);
    assert_eq!(restored.meshes()[0].id(), layer_id);
    assert!(!controller.is_dirty());
    assert_eq!(controller.undo_len(), 0);
    assert_eq!(controller.undo_layer_id(), None);
}

#[test]
fn is_busy_reflected_in_panel_state_machine() {
    let layer = SceneMesh::new(Mesh::empty());
    let mut scene = Scene::new();
    scene.add(layer.clone());
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(!controller.is_busy());
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    // While the busy token is outstanding the session is busy.
    assert!(controller.is_busy());
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert!(!controller.is_busy());
}

#[test]
fn sync_to_scene_rearms_empty_selection_between_session_ops() {
    let Some(mesh) = two_triangle_mesh("session") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let layer_id = layer.id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.select_all_faces());

    // First op: clears the selection and changes the layer topology.
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::DeleteSelectedFaces)
    else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    let Some(single_triangle) = triangle_mesh("after-op") else {
        return;
    };
    scene.meshes_mut()[layer_index].mesh = single_triangle;
    controller.sync_to_scene(&scene);

    // The session panel survives the op: an EMPTY selection sized to the
    // new topology is re-armed, ready for the next mark + op.
    assert_eq!(controller.selected_layer_id(), Some(layer_id));
    assert_eq!(controller.selected_face_count(), 0);
    assert!(controller.select_all_faces());
    assert_eq!(controller.selected_face_count(), 1);

    // Second op in the SAME session works without re-entering edit mode.
    let current = scene.meshes()[layer_index].clone();
    let Some(token) = controller.begin_layer_edit(&current, EditModeCommand::CloseHoles) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), Some(layer_id));
    assert!(controller.is_dirty());
    assert_eq!(controller.undo_len(), 2);
}

#[test]
fn redo_reapplies_undone_layer_edit_and_new_op_clears_redo() {
    let Some(before_mesh) = triangle_mesh("before") else {
        return;
    };
    let Some(after_mesh) = two_triangle_mesh("after") else {
        return;
    };
    let layer = SceneMesh::new(before_mesh);
    let mut edited = layer.clone();
    edited.mesh = after_mesh;
    let mut controller = EditModeController::new(4, 1_000_000);

    // Op: snapshot "before", scene now holds "edited".
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::CloseHoles) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );

    // Undo back to "before", then redo forward to "edited".
    let Some(undone) = controller.undo_last_layer_edit(&edited) else {
        return;
    };
    assert_eq!(undone.mesh.triangle_count(), 1);
    assert_eq!(controller.redo_layer_id(), Some(layer.id()));
    let Some(redone) = controller.redo_last_layer_edit(&undone) else {
        return;
    };
    assert_eq!(redone.mesh.triangle_count(), 2);
    assert!(controller.is_dirty());

    // A new op clears the redo chain.
    let Some(undone) = controller.undo_last_layer_edit(&redone) else {
        return;
    };
    let Some(token) = controller.begin_layer_edit(&undone, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    assert_eq!(controller.redo_layer_id(), None);
    assert!(controller.redo_last_layer_edit(&undone).is_none());
}

#[test]
fn noop_finish_discards_snapshot_and_keeps_marks() {
    let Some(mesh) = two_triangle_mesh("noop") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.select_all_faces());
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::CloseHoles) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_noop(token),
        BusyFinish::Applied
    );
    controller.sync_to_scene(&scene);

    // A content no-op leaves no phantom undo step, does not dirty the
    // session, and keeps the operator's marks (topology unchanged).
    assert_eq!(controller.undo_len(), 0);
    assert!(!controller.is_dirty());
    assert_eq!(controller.selected_layer_id(), Some(layer.id()));
    assert_eq!(controller.selected_face_count(), 2);
}

#[test]
fn noop_finish_clears_busy_state() {
    let layer = SceneMesh::new(Mesh::empty());
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(!controller.is_busy());
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    // A busy-token leak (early return without finishing) would leave this
    // true forever, permanently disabling the mesh-edit panel.
    assert!(controller.is_busy());
    assert_eq!(
        controller.finish_layer_edit_noop(token),
        BusyFinish::Applied
    );
    assert!(!controller.is_busy());
}

#[test]
fn sync_to_scene_does_not_rearm_selection_after_done_or_cancel() {
    let Some(mesh) = two_triangle_mesh("done") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    controller.finish_edit_session();
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);

    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.cancel_edit_session().is_some());
    controller.sync_to_scene(&scene);
    assert_eq!(controller.selected_layer_id(), None);
}

#[test]
fn mixed_structural_and_layer_history_unwinds_in_order_and_cancel_reverts_all() {
    let Some(mesh) = two_triangle_mesh("mixed") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let layer_id = layer.id();
    let mut controller = EditModeController::new(8, 10_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    // Non-structural op first (layer snapshot)...
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    // ...then a structural op (scene snapshot) that spawns a layer. As in the
    // real cut, the product is inserted BEFORE the op finishes, so the post-op
    // fingerprint covers it and the immediate undo is safe.
    let Some(token) =
        controller.begin_scene_edit(&scene, layer_id, EditModeCommand::CutSelectionToNewLayer)
    else {
        return;
    };
    scene.add(SceneMesh::new(Mesh::empty()));
    assert_eq!(
        controller.finish_scene_edit_success(token, &scene),
        BusyFinish::Applied
    );
    assert_eq!(controller.undo_len(), 2);

    // Undo unwinds LIFO: the structural snapshot first, then the layer one.
    let step = controller.undo_last_scene_edit(&scene, layer_id);
    assert!(
        matches!(step, StructuralHistoryStep::Restored(_)),
        "an unchanged layer set should restore the structural snapshot"
    );
    let StructuralHistoryStep::Restored(restored_scene) = step else {
        return;
    };
    assert_eq!(restored_scene.meshes().len(), 1);
    let current = restored_scene.meshes()[0].clone();
    assert!(controller.undo_last_layer_edit(&current).is_some());
    assert_eq!(controller.undo_len(), 0);

    // The session is still live and Cancel still reverts to the baseline.
    let Some(baseline) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(baseline.meshes().len(), 1);
    assert_eq!(baseline.meshes()[0].id(), layer_id);
}

#[test]
fn oversized_snapshot_applies_edit_without_phantom_undo() {
    let Some(mesh) = two_triangle_mesh("huge") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    // A byte cap below any real snapshot: the pre-op snapshot is skipped.
    let mut controller = EditModeController::new(8, 1);

    assert!(controller.begin_face_selection(&layer, &scene));
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    // The edit applied (dirty) but is not undoable — and nothing lies about it.
    assert!(controller.is_dirty());
    assert_eq!(controller.undo_layer_id(), None);

    // A no-op finish with a skipped snapshot must not discard someone else's
    // history either (there is none here; it must simply not panic/underflow).
    let Some(token) = controller.begin_layer_edit(&layer, EditModeCommand::CloseHoles) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_noop(token),
        BusyFinish::Applied
    );
    assert_eq!(controller.undo_layer_id(), None);
}

#[test]
fn append_preserves_active_session_and_undo_on_original_layer() {
    // Scenario 2: begin a session on A, then simulate appending layer B (a
    // second load). The session — selection, dirty state, undo history — is
    // keyed by SceneMeshId, so it survives the append and still applies to A.
    let Some(mesh_a) = two_triangle_mesh("A") else {
        return;
    };
    let mut scene = Scene::new();
    let index_a = scene.add(SceneMesh::new(mesh_a));
    let layer_a = scene.meshes()[index_a].clone();
    let id_a = layer_a.id();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    assert!(controller.select_all_faces());

    // A real edit on A records undo and dirties the session.
    let Some(token) = controller.begin_layer_edit(&layer_a, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );

    // Append B (append_scene keeps A's id and gives B a fresh unique id).
    let Some(mesh_b) = triangle_mesh("B") else {
        return;
    };
    scene.add(SceneMesh::new(mesh_b));
    controller.sync_to_scene(&scene);

    // The session survives: selection still targets A, still dirty, undo intact.
    assert_eq!(controller.selected_layer_id(), Some(id_a));
    assert!(controller.is_dirty());
    assert_eq!(controller.undo_layer_id(), Some(id_a));

    // The undo still applies to A specifically (id-keyed, not index-keyed).
    let current_a = scene.meshes()[index_a].clone();
    assert_eq!(
        controller.undo_last_layer_edit(&current_a).map(|l| l.id()),
        Some(id_a)
    );
}

#[test]
fn edit_on_other_layer_during_session_keeps_session_on_original() {
    // Scenario 8: session active on A; a whole-mesh op runs on a DIFFERENT layer
    // B. The global undo stack records B's edit (LIFO), but the session's
    // recoverable identity — selection and baseline — stays anchored to A.
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
    let mut controller = EditModeController::new(8, 10_000_000);

    assert!(controller.begin_face_selection(&layer_a, &scene));
    assert!(controller.select_all_faces());

    // Whole-mesh op on B (as the RMB "Invert normals" on another layer would).
    let Some(token) = controller.begin_layer_edit(&layer_b, EditModeCommand::InvertNormals) else {
        return;
    };
    assert_eq!(
        controller.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    controller.sync_to_scene(&scene);

    // The session panel stays on A (selection anchored to A, not B), and the
    // most recent undo is B's edit — a global LIFO stack, not contamination.
    assert_eq!(controller.selected_layer_id(), Some(id_a));
    assert_eq!(controller.undo_layer_id(), Some(id_b));
    assert!(controller.is_dirty());

    // Cancelling the session reverts to the pre-session baseline, which predates
    // B's edit too (edit mode was active when it ran) — Cancel means "undo
    // everything since I entered edit mode".
    let Some(baseline) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(baseline.meshes().len(), 2);
    assert_eq!(baseline.meshes()[0].id(), id_a);
    assert_eq!(baseline.meshes()[1].id(), id_b);
}

#[test]
fn begin_face_selection_arms_lasso_by_default_and_resets_between_sessions() {
    let Some(mesh) = two_triangle_mesh("armed") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    // A fresh controller (no session) leaves the camera free — lasso disarmed.
    assert!(!controller.lasso_armed());

    // Opening an edit session arms the lasso so the first click drops an
    // outline point instead of orbiting.
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.lasso_armed());

    // Mid-session the operator can switch to the marquee by disarming.
    assert!(controller.set_lasso_armed(false));
    assert!(!controller.lasso_armed());

    // Done disarms; the NEXT session re-arms regardless of the prior toggle.
    controller.finish_edit_session();
    assert!(!controller.lasso_armed());
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.lasso_armed());

    // Cancel behaves the same: disarm mid-session, cancel, re-open → armed.
    assert!(controller.set_lasso_armed(false));
    assert!(controller.cancel_edit_session().is_some());
    assert!(!controller.lasso_armed());
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.lasso_armed());
}

#[test]
fn begin_face_selection_defaults_to_through_mesh_and_resets_between_sessions() {
    let Some(mesh) = two_triangle_mesh("through") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    // Opening an edit session selects Through mode by default (the lasso marks
    // every enclosed face, not only the front-facing ones).
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.through_mesh());

    // Mid-session the operator can switch to surface mode.
    assert!(controller.set_through_mesh(false));
    assert!(!controller.through_mesh());

    // Done ends the session; the NEXT session re-forces Through regardless of
    // the prior toggle.
    controller.finish_edit_session();
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.through_mesh());

    // Cancel behaves the same: surface mid-session, cancel, re-open -> Through.
    assert!(controller.set_through_mesh(false));
    assert!(controller.cancel_edit_session().is_some());
    assert!(controller.begin_face_selection(&layer, &scene));
    assert!(controller.through_mesh());
}

#[test]
fn sync_to_scene_preserves_baseline_when_active_layer_disappears() {
    let Some(mesh) = triangle_mesh("gone") else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer = scene.meshes()[layer_index].clone();
    let mut controller = EditModeController::new(4, 1_000_000);

    assert!(controller.begin_face_selection(&layer, &scene));
    // Baseline held while the layer exists.
    assert!(controller.cancel_edit_session().is_some());
    // Re-open a session, then remove the layer: sync suspends the active
    // target but preserves the original baseline so Cancel remains exact.
    assert!(controller.begin_face_selection(&layer, &scene));
    scene.remove(layer_index);
    controller.sync_to_scene(&scene);
    let Some(restored) = controller.cancel_edit_session() else {
        return;
    };
    assert_eq!(restored.meshes().len(), 1);
    assert_eq!(restored.meshes()[0].id(), layer.id());
}

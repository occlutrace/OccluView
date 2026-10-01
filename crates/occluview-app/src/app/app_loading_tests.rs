#![allow(clippy::expect_used, clippy::panic)]

use super::align::drag::AlignDrag;
use super::mesh_edit::export::PendingLayerExports;
use super::app_test_support::{named_scene, push_named_layer, test_app};
use super::layers_overlay::LayerOverlayChanges;
use super::workspace::commands::SplitSide;
use super::*;
use crate::edit_mode::{BusyFinish, EditModeCommand};
use glam::Affine3A;

fn scene_names_for_context(context: &SceneContext<'_>) -> Vec<String> {
    context
        .document
        .scene
        .as_ref()
        .map(|scene| {
            scene
                .meshes()
                .iter()
                .map(|entry| entry.mesh.name().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn delivered_load_for_scene(
    scene_key: workspace::id::SceneKey,
    content_revision_at_request: u64,
    scene: Scene,
    mode: SceneLoadMode,
    path: &str,
) -> PendingSceneLoad {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(scene))
        .expect("the channel was just created");
    PendingSceneLoad {
        scene_key,
        paths: vec![PathBuf::from(path)],
        source: if mode == SceneLoadMode::Append {
            "add"
        } else {
            "open"
        },
        mode,
        started_at: Instant::now(),
        receiver,
        superseded: false,
        content_revision_at_request,
        dirty_at_request: false,
        requested_at: Instant::now(),
    }
}

fn apply_result(context: &mut SceneContext<'_>, pending: PendingSceneLoad, result: Result<Scene>) {
    context.apply_scene_load_result(pending, result, &egui::Context::default());
}

#[test]
fn late_replace_arriving_during_a_busy_edit_is_parked_not_applied() {
    let mut app = test_app("late-replace-busy");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    target
        .document
        .edit_mode
        .begin_layer_edit(
            &target.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    assert!(target.document.edit_mode.is_busy());
    assert!(!target.document.has_unsaved_mesh_edits());

    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-a".to_string()],
        "a live edit must not be replaced"
    );
    assert!(
        target.ui.pending_replace_open.is_some(),
        "the parked open is how the operator answers the guard"
    );
    assert!(target.document.edit_mode.is_busy());
    let _ = layer_id;
}

#[test]
fn late_replace_after_a_committed_edit_is_parked() {
    let mut app = test_app("late-replace-committed");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    let scene = target.document.scene.as_ref().expect("scene").clone();
    let token = target
        .document
        .edit_mode
        .begin_scene_edit(scene.as_ref(), layer_id, EditModeCommand::MoveLayer)
        .expect("scene edit session");
    assert_eq!(
        target
            .document
            .edit_mode
            .finish_scene_edit_success(token, scene.as_ref()),
        BusyFinish::Applied
    );
    target.document.mark_mesh_edits_unsaved(layer_id);
    assert!(target.document.has_unsaved_mesh_edits());

    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-a".to_string()]
    );
    assert!(target.ui.pending_replace_open.is_some());
    assert!(target.document.has_unsaved_mesh_edits());
}

#[test]
fn replace_delivered_into_an_idle_clean_session_is_applied() {
    let mut app = test_app("replace-idle-clean");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let paths = pending.paths.clone();

    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    assert!(
        target.ui.pending_replace_open.is_none(),
        "an idle clean session has nothing to guard"
    );
    assert!(target.loader.active.is_none());
    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-b".to_string()]
    );
    assert_eq!(target.document.current_paths, paths);
}

#[test]
fn a_completed_load_stays_with_its_captured_scene_after_focus_changes() {
    let mut app = test_app("load-target-survives-focus-change");
    let first_key = app.workspace.scenes[0].key;
    {
        let mut first = app.scene_context(first_key).expect("first scene");
        first.queue_new_scene(SplitSide::Right);
    }
    app.apply_workspace_commands(&egui::Context::default());
    let second_key = app.workspace.scenes[1].key;
    app.scene_context(first_key)
        .expect("first scene")
        .set_scene(named_scene("first", 0.0), true);
    let mut second_scene = named_scene("second", 10.0);
    push_named_layer(&mut second_scene, "second-existing", 20.0);
    app.scene_context(second_key)
        .expect("second scene")
        .set_scene(second_scene, true);

    let pending = delivered_load_for_scene(
        second_key,
        app.scene_context(second_key)
            .expect("second scene")
            .document
            .content_revision,
        named_scene("imported", 30.0),
        SceneLoadMode::Append,
        "/cases/imported.stl",
    );
    app.loader.install_active(pending);
    let first_target = app.workspace.scenes[0].target();
    app.workspace.input.request_activation(first_target);

    app.scene_context(first_key)
        .expect("first scene")
        .process_scene_loads(&egui::Context::default());
    assert!(app.loader.active_for(second_key));
    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("first scene content")
            .meshes()
            .len(),
        1,
        "polling another pane cannot take the result"
    );

    app.scene_context(second_key)
        .expect("second scene")
        .process_scene_loads(&egui::Context::default());
    assert_eq!(
        app.workspace.scenes[1]
            .document
            .scene
            .as_ref()
            .expect("second scene content")
            .meshes()
            .len(),
        3,
        "the result commits to the captured scene even when another pane has focus"
    );
}

#[test]
fn blocked_append_in_one_scene_does_not_stall_another_scenes_load() {
    let mut app = test_app("blocked-append-other-scene-load");
    let first_key = app.workspace.scenes[0].key;
    {
        let mut first = app.scene_context(first_key).expect("first scene");
        first.set_scene(named_scene("first", 0.0), true);
        let scene = first
            .document
            .scene
            .as_ref()
            .expect("first content")
            .clone();
        assert!(first
            .document
            .edit_mode
            .begin_face_selection(&scene.meshes()[0], &scene,));
        first.append_paths(&[PathBuf::from("/missing/first-append.stl")], "drop");
        assert!(first.loader.has_queued_for(first_key));
        assert!(first.loader.active.is_none());
    }

    {
        let mut first = app.scene_context(first_key).expect("first scene");
        first.queue_new_scene(SplitSide::Right);
    }
    app.apply_workspace_commands(&egui::Context::default());
    let second_key = app.workspace.scenes[1].key;

    {
        let mut second = app.scene_context(second_key).expect("second scene");
        second.replace_paths(&[PathBuf::from("/missing/second-open.stl")], "open");
    }

    assert!(
        app.loader.active_for(second_key),
        "the eligible second-scene request starts despite the hidden blocked Append"
    );
    assert!(app.loader.has_queued_for(first_key));
}

#[test]
fn successful_guarded_replace_rebinds_later_appends_without_reordering_other_scene_work() {
    let mut app = test_app("replace-rebinds-later-appends");
    let first_key = app.workspace.scenes[0].key;
    {
        let mut first = app.scene_context(first_key).expect("first scene");
        first.set_scene(named_scene("first", 0.0), true);
        let scene = first
            .document
            .scene
            .as_ref()
            .expect("first content")
            .clone();
        assert!(first
            .document
            .edit_mode
            .begin_face_selection(&scene.meshes()[0], &scene,));
    }
    {
        let mut first = app.scene_context(first_key).expect("first scene");
        first.queue_new_scene(SplitSide::Right);
    }
    app.apply_workspace_commands(&egui::Context::default());
    let second_key = app.workspace.scenes[1].key;
    app.scene_context(first_key)
        .expect("first scene")
        .replace_paths(&[PathBuf::from("/cases/replacement.stl")], "open");
    assert!(app.ui.pending_replace_open.is_some());
    app.scene_context(second_key)
        .expect("second scene")
        .append_paths(&[PathBuf::from("/cases/second-after-open.stl")], "drop");
    app.scene_context(first_key)
        .expect("first scene")
        .append_paths(&[PathBuf::from("/cases/first-after-open.stl")], "drop");

    let mut first = app.scene_context(first_key).expect("first scene");
    let pending_open = first
        .ui
        .pending_replace_open
        .take()
        .expect("guarded Replace");
    // Model the operator choosing Discard in the guard. The request timestamp
    // remains the original acceptance time, before both queued Appends.
    first.document.edit_mode.clear();
    first.replace_paths_confirmed(
        &pending_open.paths,
        pending_open.source,
        pending_open.requested_at,
    );
    let replace = first
        .loader
        .take_active_for(first_key)
        .expect("confirmed Replace starts before the later Appends");
    assert_eq!(replace.mode, SceneLoadMode::Replace);

    let mut delivered = delivered_load_for_scene(
        first_key,
        first.document.content_revision,
        named_scene("replacement", 10.0),
        SceneLoadMode::Replace,
        "/cases/replacement.stl",
    );
    delivered.requested_at = replace.requested_at;
    apply_result(&mut first, delivered, Ok(named_scene("replacement", 10.0)));
    let new_first_key = first.scene_key;
    assert_ne!(first_key, new_first_key, "Replace creates a new lifetime");
    drop(first);

    assert_eq!(
        app.loader
            .queued
            .iter()
            .map(|request| (request.scene_key, request.paths[0].clone()))
            .collect::<Vec<_>>(),
        vec![
            (second_key, PathBuf::from("/cases/second-after-open.stl")),
            (new_first_key, PathBuf::from("/cases/first-after-open.stl")),
        ],
        "later Appends follow the new lifetime while global order is preserved"
    );
}

#[test]
fn failed_guarded_replace_keeps_later_appends_on_the_original_scene() {
    let mut app = test_app("failed-replace-keeps-appends");
    let first_key = app.workspace.scenes[0].key;
    let mut first = app.scene_context(first_key).expect("first scene");
    first.set_scene(named_scene("first", 0.0), true);
    let scene = first
        .document
        .scene
        .as_ref()
        .expect("first content")
        .clone();
    assert!(first
        .document
        .edit_mode
        .begin_face_selection(&scene.meshes()[0], &scene,));
    first.replace_paths(&[PathBuf::from("/cases/replacement.stl")], "open");
    first.append_paths(&[PathBuf::from("/cases/after-open.stl")], "drop");

    let pending_open = first
        .ui
        .pending_replace_open
        .take()
        .expect("guarded Replace");
    first.document.edit_mode.clear();
    first.replace_paths_confirmed(
        &pending_open.paths,
        pending_open.source,
        pending_open.requested_at,
    );
    let replace = first
        .loader
        .take_active_for(first_key)
        .expect("confirmed Replace starts before the later Appends");
    let mut delivered = delivered_load_for_scene(
        first_key,
        first.document.content_revision,
        named_scene("replacement", 10.0),
        SceneLoadMode::Replace,
        "/cases/replacement.stl",
    );
    delivered.requested_at = replace.requested_at;
    apply_result(&mut first, delivered, Err(anyhow::anyhow!("unreadable")));

    assert_eq!(first.scene_key, first_key);
    assert_eq!(
        first
            .loader
            .queued
            .iter()
            .map(|request| (request.scene_key, request.paths[0].clone()))
            .collect::<Vec<_>>(),
        vec![(first_key, PathBuf::from("/cases/after-open.stl"))],
        "a failed Replace leaves the queued Append eligible for the old scene"
    );
}

#[test]
fn append_remains_queued_while_an_edit_checkpoint_is_open() {
    let mut app = test_app("append-queued-during-edit");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer = target.document.scene.as_ref().expect("scene").meshes()[0].clone();
    let baseline = target.document.scene.as_ref().expect("scene").clone();
    assert!(target
        .document
        .edit_mode
        .begin_face_selection(&layer, &baseline));

    target.append_paths(&[PathBuf::from("/cases/b.stl")], "test");

    assert!(target.loader.active.is_none());
    assert!(target.loader.decoded.is_none());
    assert!(target.loader.has_queued_for(target.scene_key));

    target.document.edit_mode.finish_edit_session();
    let request = target
        .loader
        .take_next_for(target.scene_key, &[])
        .expect("Append is eligible after Done closes the baseline");
    assert_eq!(request.scene_key, target.scene_key);
    assert_eq!(request.mode, SceneLoadMode::Append);
}

#[test]
fn decoded_append_waits_until_a_pending_mesh_edit_finishes() {
    let mut app = test_app("append-mid-edit");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    target.document.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let token = target
        .document
        .edit_mode
        .begin_layer_edit(
            &target.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 5.0),
        SceneLoadMode::Append,
        "/cases/b.stl",
    );
    target.loader.install_active(pending);

    target.process_scene_loads(&egui::Context::default());

    assert_eq!(
        target
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .len(),
        1,
        "Append cannot replace the scene while a mesh edit is pending"
    );
    assert!(
        target.loader.decoded_for(target.scene_key),
        "the completed result stays parked and keeps the one global decoder slot"
    );

    assert_eq!(
        target.document.edit_mode.finish_layer_edit_success(token),
        BusyFinish::Applied
    );
    target.process_scene_loads(&egui::Context::default());

    assert_eq!(
        target
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()
            .len(),
        2,
        "the Append commits after pending mesh work finishes"
    );
    assert!(target.loader.decoded.is_none());
    assert!(target.ui.pending_replace_open.is_none());
    assert_eq!(target.document.current_paths.len(), 2);
}

#[test]
fn failed_replace_leaves_the_scene_and_its_edits_untouched() {
    let mut app = test_app("replace-failed");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    apply_result(&mut target, pending, Err(anyhow::anyhow!("unreadable")));

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-a".to_string()]
    );
    assert!(
        target.ui.app_error.is_some(),
        "the failure must be reported"
    );
    assert!(target.loader.active.is_none());
    assert!(target.ui.pending_replace_open.is_none());
}

#[test]
fn superseded_replace_drops_its_result_and_keeps_other_scene_work_queued() {
    let mut app = test_app("replace-superseded");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let active = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let other_key = workspace::id::SceneKey::from_raw_for_test(2, 2).expect("other target key");
    target.loader.install_active(active);
    target.loader.enqueue(SceneLoadRequest {
        scene_key: other_key,
        paths: vec![PathBuf::from("/cases/other-scene.stl")],
        source: "open",
        mode: SceneLoadMode::Append,
        content_revision_at_request: 0,
        dirty_at_request: false,
        requested_at: Instant::now(),
    });
    target.loader.enqueue(SceneLoadRequest {
        scene_key: target.scene_key,
        paths: vec![PathBuf::from("/cases/newer.stl")],
        source: "open",
        mode: SceneLoadMode::Replace,
        content_revision_at_request: target.document.content_revision,
        dirty_at_request: false,
        requested_at: Instant::now(),
    });
    assert!(
        target
            .loader
            .active
            .as_ref()
            .expect("active load")
            .superseded
    );
    assert_eq!(target.loader.queued_len(), 2);

    target.process_scene_loads(&egui::Context::default());

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-a".to_string()],
        "the superseded Replace result cannot commit"
    );
    assert!(target.ui.pending_replace_open.is_none());
    assert!(target.loader.active.is_none());
    assert_eq!(
        target
            .loader
            .queued
            .iter()
            .map(|request| (request.scene_key, request.paths[0].clone()))
            .collect::<Vec<_>>(),
        vec![
            (other_key, PathBuf::from("/cases/other-scene.stl")),
            (target.scene_key, PathBuf::from("/cases/newer.stl")),
        ],
        "the other scene's earlier work keeps its place ahead of the newer Replace"
    );
}

#[test]
fn parked_replace_keeps_the_request_it_was_authorized_for() {
    let mut app = test_app("parked-request");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let expected_paths = pending.paths.clone();
    let expected_requested_at = pending.requested_at;
    target
        .document
        .edit_mode
        .begin_layer_edit(
            &target.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    let parked = target
        .ui
        .pending_replace_open
        .as_ref()
        .expect("parked open");
    assert_eq!(parked.paths, expected_paths);
    assert_eq!(parked.source, "open");
    assert_eq!(parked.requested_at, expected_requested_at);
}

#[test]
fn late_replace_arriving_mid_align_drag_does_not_discard_the_pose() {
    let mut app = test_app("replace-mid-align-drag");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start: Affine3A::IDENTITY,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "a pose the operator can see on screen is work, from the first frame"
    );

    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-a".to_string()],
        "a move in progress must not be discarded by an older open"
    );
    assert!(
        target.ui.pending_replace_open.is_some(),
        "the operator answers the guard"
    );
    let pose = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert_eq!(
        pose,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0))
    );
}
#[test]
fn removing_the_last_layer_does_not_edit_the_scene_while_a_handle_is_alive() {
    let mut app = test_app("remove-last-layer");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    target.document.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();

    let scene_snapshot = target.document.scene.as_ref().expect("scene").clone();
    let paths_snapshot = target.document.current_paths.clone();
    target.apply_layer_overlay_changes(
        scene_snapshot,
        &paths_snapshot,
        LayerOverlayChanges {
            context_request: Some(LayerContextRequest {
                index: 0,
                layer_id,
                action: LayerContextAction::Remove,
            }),
            layer_edits: Vec::new(),
            ..LayerOverlayChanges::default()
        },
        &egui::Context::default(),
    );

    assert!(target.document.scene.is_none(), "the last layer is gone");
    assert!(target.document.current_paths.is_empty());
    assert!(target.document.unsaved_edit_layer_ids.is_empty());
}

#[test]
fn a_drag_that_returns_to_its_start_leaves_no_unsaved_mark_or_history_step() {
    let mut app = test_app("drag-round-trip");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert!(
        !target.document.has_unsaved_mesh_edits(),
        "fixture starts clean"
    );

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });

    let out = Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0));
    let back = Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0));
    target.nudge_align_layer(layer_id, out);
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "an unreleased move is work the operator can see: the load guard reads this"
    );
    target.nudge_align_layer(layer_id, back);

    let acted = target.finish_align_drag();

    assert!(!acted, "a drag that ended where it started is not an edit");
    assert_eq!(
        target.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "the pose is back where it began"
    );
    assert!(
        !target.document.has_unsaved_mesh_edits(),
        "nothing changed, so nothing may be reported as unsaved"
    );
    assert!(
        target.document.edit_mode.undo_layer_id().is_none(),
        "and no undo step may have been recorded"
    );
}

#[test]
fn a_drag_that_returns_to_its_start_keeps_edits_that_were_already_unsaved() {
    let mut app = test_app("drag-round-trip-prior-edits");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.document.mark_mesh_edits_unsaved(layer_id);
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "fixture starts dirty"
    );

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );

    target.finish_align_drag();

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "a drag that cancelled itself must not erase edits the layer already had"
    );
}

#[test]
fn a_drag_that_returns_to_its_start_keeps_another_layers_unsaved_edits() {
    let mut app = test_app("drag-round-trip-other-layer");
    let mut target = app.active_context().expect("test scene");
    let mut scene = named_scene("scene-a", 0.0);
    let other_id = push_named_layer(&mut scene, "scene-b", 20.0);
    target.document.scene = Some(Arc::new(scene));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.document.mark_mesh_edits_unsaved(other_id);

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );

    target.finish_align_drag();

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "the other layer's unsaved work must survive the round-trip drag"
    );
    assert_eq!(scene_names_for_context(&target).len(), 2);
}

#[test]
fn a_drag_that_ends_somewhere_else_is_still_one_undoable_unsaved_edit() {
    let mut app = test_app("drag-real-move");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(4.0, 0.0, 0.0)),
    );
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(1.0, 0.0, 0.0)),
    );
    let ended = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert_ne!(ended, start, "fixture: the scan ended somewhere else");

    assert!(target.finish_align_drag(), "a real move is a recorded edit");
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "a move the operator kept is unsaved work"
    );
    assert_eq!(
        target.document.scene.as_ref().expect("scene").meshes()[0].transform,
        ended,
        "the pose the operator released on is the one that stays"
    );

    target.apply_history_navigation_now(false, &egui::Context::default());

    assert_eq!(
        target.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "one Ctrl+Z returns the whole gesture"
    );
}

#[test]
fn a_round_trip_drag_does_not_clear_work_committed_mid_gesture() {
    let mut app = test_app("drag-round-trip-mid-gesture");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });

    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    assert!(target.document.has_unsaved_mesh_edits());

    target.document.mark_mesh_edits_unsaved(layer_id);

    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );
    assert_eq!(
        target.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "fixture: the pose is back where it began"
    );

    target.finish_align_drag();

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "an edit that landed during the gesture must still be unsaved work"
    );
}

#[test]
fn a_second_nudge_does_not_forget_a_mid_gesture_commit() {
    let mut app = test_app("drag-round-trip-refreshed-witness");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });

    let out = Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0));
    target.nudge_align_layer(layer_id, out);
    target.document.mark_mesh_edits_unsaved(layer_id);
    target.nudge_align_layer(layer_id, out);
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-6.0, 0.0, 0.0)),
    );
    assert_eq!(
        target.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "fixture: the pose is back where it began"
    );

    target.finish_align_drag();

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "an edit that landed mid-gesture must survive however many nudges followed it"
    );
}

#[test]
fn an_unreleased_drag_does_not_enter_the_committed_edit_set() {
    let mut app = test_app("drag-provisional-separation");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;

    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );

    assert!(
        target.document.has_unsaved_mesh_edits(),
        "an unreleased move is work: the guards must still see it"
    );
    assert!(
        !target.document.unsaved_edit_layer_ids.contains(&layer_id),
        "but it is not a committed edit, so it must stay out of the set: a set \
         cannot tell the gesture's own mark from work that landed mid-drag"
    );

    target.finish_align_drag();

    assert!(
        target.document.unsaved_edit_layer_ids.contains(&layer_id),
        "releasing the gesture commits it into the set"
    );
}

#[test]
fn a_scene_replace_clears_an_open_drags_provisional_pose() {
    let mut app = test_app("replace-clears-drag-pose");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "fixture: the open drag is holding a moved pose"
    );

    let mut pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    pending.dirty_at_request = true;
    pending.content_revision_at_request = target.document.content_revision;
    apply_result(&mut target, pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names_for_context(&target),
        vec!["scene-b".to_string()],
        "fixture: the confirmed replace was applied"
    );
    assert!(
        !target.document.has_unsaved_mesh_edits(),
        "the replaced scene holds no work from the old one, drag pose included"
    );
}

#[test]
fn clearing_the_scene_clears_an_open_drags_provisional_pose() {
    let mut app = test_app("clear-clears-drag-pose");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );

    target.clear_scene();

    assert!(
        target.document.scene.is_none(),
        "fixture: the scene is gone"
    );
    assert!(
        !target.document.has_unsaved_mesh_edits(),
        "a cleared scene has nothing unsaved, drag pose included"
    );
}

#[test]
fn the_guard_save_flow_does_not_report_nothing_to_save_about_a_held_drag() {
    let mut app = test_app("guard-save-mid-drag");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "fixture: work is held"
    );

    let listed = target.pending_layer_exports();

    let pending = match listed {
        PendingLayerExports::Ready { pending, .. } => pending,
        PendingLayerExports::Nothing => panic!(
            "a held drag is work: the flow must build the work list, not answer \
             that there is nothing to save"
        ),
        PendingLayerExports::StrokeInFlight => panic!(
            "no Sculpt stroke is live here, so the flow must not report one as \
             still landing"
        ),
    };
    assert_eq!(
        pending.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
        vec![layer_id],
        "and the work list must name the layer the operator moved"
    );
    assert!(
        target.document.edit_mode.undo_layer_id() == Some(layer_id),
        "with the gesture released into the one recorded step"
    );
}

#[test]
fn an_append_does_not_discard_a_held_drag_pose_it_carries_forward() {
    let mut app = test_app("append-carries-held-pose");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    target.document.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let layer_id = target.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = target.document.scene.as_ref().expect("scene").meshes()[0].transform;
    target.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        pivot_local: glam::Vec3::ZERO,
    });
    target.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    let moved = target.document.scene.as_ref().expect("scene").meshes()[0].transform;

    let pending = delivered_load_for_scene(
        target.scene_key,
        target.document.content_revision,
        named_scene("scene-b", 20.0),
        SceneLoadMode::Append,
        "/cases/b.stl",
    );
    apply_result(&mut target, pending, Ok(named_scene("scene-b", 20.0)));

    let scene = target.document.scene.as_ref().expect("scene");
    assert_eq!(scene.meshes().len(), 2, "fixture: the append landed");
    assert_eq!(
        scene.meshes()[0].transform,
        moved,
        "fixture: the append kept the moved pose"
    );
    assert!(
        target.document.has_unsaved_mesh_edits(),
        "a pose that survived into the new scene is still unsaved work"
    );
    assert!(
        target.document.unsaved_edit_layer_ids.contains(&layer_id),
        "and it must be a committed edit, or the close guard cannot name it"
    );
    assert!(
        target.document.edit_mode.undo_layer_id() == Some(layer_id),
        "with the one history step the release would have recorded"
    );
}

/// A same-scene Replace supersedes old queued work but leaves other targets
/// untouched. This protects a scene-specific guard from opening an earlier
/// request after the operator chooses a newer file.
#[test]
fn a_parked_request_supersedes_replaces_already_in_the_queue() {
    let mut app = test_app("parked-supersedes-queued");
    let mut target = app.active_context().expect("test scene");
    target.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    // An older Replace is queued behind a decode.
    target.loader.enqueue(SceneLoadRequest {
        scene_key: target.scene_key,
        paths: vec![PathBuf::from("/cases/older.stl")],
        source: "open",
        mode: SceneLoadMode::Replace,
        content_revision_at_request: target.document.content_revision,
        dirty_at_request: false,
        requested_at: Instant::now(),
    });
    // …and the operator now asks for a newer file while an edit session is open,
    // which is what parks the request.
    target
        .document
        .edit_mode
        .begin_layer_edit(
            &target.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    target.replace_paths(&[PathBuf::from("/cases/newer.stl")], "open");

    assert!(
        target.ui.pending_replace_open.is_some(),
        "the newest request is held for the guard"
    );
    assert!(
        !target.loader.has_queued_for(target.scene_key),
        "the older queued Replace is obsolete and must not survive to clobber \
         the scene the operator asked for last"
    );
}

#![allow(clippy::expect_used, clippy::panic)]

use super::app_align_drag::AlignDrag;
use super::app_mesh_export::PendingLayerExports;
use super::app_test_support::{
    delivered_load, named_scene, push_named_layer, scene_names, test_app,
};
use super::layers_overlay::LayerOverlayChanges;
use super::*;
use crate::edit_mode::{BusyFinish, EditModeCommand};
use glam::Affine3A;

#[test]
fn late_replace_arriving_during_a_busy_edit_is_parked_not_applied() {
    let mut app = test_app("late-replace-busy");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    app.document
        .edit_mode
        .begin_layer_edit(
            &app.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    assert!(app.document.edit_mode.is_busy());
    assert!(!app.document.has_unsaved_mesh_edits());

    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names(&app),
        vec!["scene-a".to_string()],
        "a live edit must not be replaced"
    );
    assert!(
        app.ui.pending_replace_open.is_some(),
        "the parked open is how the operator answers the guard"
    );
    assert!(app.document.edit_mode.is_busy());
    let _ = layer_id;
}

#[test]
fn late_replace_after_a_committed_edit_is_parked() {
    let mut app = test_app("late-replace-committed");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    let scene = app.document.scene.as_ref().expect("scene").clone();
    let token = app
        .document
        .edit_mode
        .begin_scene_edit(scene.as_ref(), layer_id, EditModeCommand::MoveLayer)
        .expect("scene edit session");
    assert_eq!(
        app.document
            .edit_mode
            .finish_scene_edit_success(token, scene.as_ref()),
        BusyFinish::Applied
    );
    app.document.mark_mesh_edits_unsaved(layer_id);
    assert!(app.document.has_unsaved_mesh_edits());

    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(scene_names(&app), vec!["scene-a".to_string()]);
    assert!(app.ui.pending_replace_open.is_some());
    assert!(app.document.has_unsaved_mesh_edits());
}

#[test]
fn replace_delivered_into_an_idle_clean_session_is_applied() {
    let mut app = test_app("replace-idle-clean");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let paths = pending.paths.clone();

    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    assert!(
        app.ui.pending_replace_open.is_none(),
        "an idle clean session has nothing to guard"
    );
    assert!(app.document.active_load.is_none());
    assert_eq!(scene_names(&app), vec!["scene-b".to_string()]);
    assert_eq!(app.persistence.current_paths, paths);
}

#[test]
fn append_result_lands_while_an_edit_session_is_open() {
    let mut app = test_app("append-mid-edit");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    app.persistence.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    app.document
        .edit_mode
        .begin_layer_edit(
            &app.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");

    app.apply_scene_load_result(
        delivered_load(
            &app,
            named_scene("scene-b", 5.0),
            SceneLoadMode::Append,
            "/cases/b.stl",
        ),
        Ok(named_scene("scene-b", 5.0)),
    );

    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes().len(),
        2,
        "the append must land"
    );
    assert!(
        app.ui.pending_replace_open.is_none(),
        "an append needs no guard"
    );
    assert_eq!(app.persistence.current_paths.len(), 2);
    let _ = layer_id;
}

#[test]
fn failed_replace_leaves_the_scene_and_its_edits_untouched() {
    let mut app = test_app("replace-failed");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    app.apply_scene_load_result(pending, Err(anyhow::anyhow!("unreadable")));

    assert_eq!(scene_names(&app), vec!["scene-a".to_string()]);
    assert!(app.ui.app_error.is_some(), "the failure must be reported");
    assert!(app.document.active_load.is_none());
    assert!(app.ui.pending_replace_open.is_none());
}

#[test]
fn superseded_replace_result_is_dropped_and_the_newer_request_runs() {
    let mut app = test_app("replace-superseded");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let mut active = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    crate::scene_loading::queue_request_while_active(
        &mut active,
        &mut app.document.queued_loads,
        SceneLoadRequest {
            paths: vec![PathBuf::from("/cases/newer.stl")],
            source: "open",
            mode: SceneLoadMode::Replace,
            content_revision_at_request: app.document.content_revision,
            dirty_at_request: false,
        },
    );
    assert!(active.superseded);
    assert_eq!(app.document.queued_loads.len(), 1);
    app.document.active_load = Some(active);

    app.process_scene_loads(&egui::Context::default());

    assert_eq!(
        scene_names(&app),
        vec!["scene-a".to_string()],
        "the superseded decode must not apply"
    );
    assert!(app.ui.pending_replace_open.is_none());
    let next = app
        .document
        .active_load
        .as_ref()
        .expect("the newer request must start");
    assert_eq!(next.paths, vec![PathBuf::from("/cases/newer.stl")]);
    assert!(!next.superseded);
}

#[test]
fn parked_replace_keeps_the_request_it_was_authorized_for() {
    let mut app = test_app("parked-request");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    let expected_paths = pending.paths.clone();
    app.document
        .edit_mode
        .begin_layer_edit(
            &app.document.scene.as_ref().expect("scene").meshes()[0],
            EditModeCommand::MoveLayer,
        )
        .expect("layer edit session");
    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    let parked = app.ui.pending_replace_open.as_ref().expect("parked open");
    assert_eq!(parked.paths, expected_paths);
    assert_eq!(parked.source, "open");
}

#[test]
fn late_replace_arriving_mid_align_drag_does_not_discard_the_pose() {
    let mut app = test_app("replace-mid-align-drag");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start: Affine3A::IDENTITY,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a pose the operator can see on screen is work, from the first frame"
    );

    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names(&app),
        vec!["scene-a".to_string()],
        "a move in progress must not be discarded by an older open"
    );
    assert!(
        app.ui.pending_replace_open.is_some(),
        "the operator answers the guard"
    );
    let pose = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert_eq!(
        pose,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0))
    );
}
#[test]
fn removing_the_last_layer_does_not_edit_the_scene_while_a_handle_is_alive() {
    let mut app = test_app("remove-last-layer");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    app.persistence.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();

    app.apply_layer_overlay_changes(
        app.document.scene.as_ref().expect("scene").clone(),
        &app.persistence.current_paths.clone(),
        LayerOverlayChanges {
            context_request: Some(LayerContextRequest {
                index: 0,
                layer_id,
                action: LayerContextAction::Remove,
            }),
            layer_edits: Vec::new(),
        },
        &egui::Context::default(),
    );

    assert!(app.document.scene.is_none(), "the last layer is gone");
    assert!(app.persistence.current_paths.is_empty());
    assert!(app.document.unsaved_edit_layer_ids.is_empty());
}

#[test]
fn a_drag_that_returns_to_its_start_leaves_no_unsaved_mark_or_history_step() {
    let mut app = test_app("drag-round-trip");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "fixture starts clean"
    );

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });

    let out = Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0));
    let back = Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0));
    app.nudge_align_layer(layer_id, out);
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "an unreleased move is work the operator can see: the load guard reads this"
    );
    app.nudge_align_layer(layer_id, back);

    let acted = app.finish_align_drag();

    assert!(!acted, "a drag that ended where it started is not an edit");
    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "the pose is back where it began"
    );
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "nothing changed, so nothing may be reported as unsaved"
    );
    assert!(
        app.document.edit_mode.undo_layer_id().is_none(),
        "and no undo step may have been recorded"
    );
}

#[test]
fn a_drag_that_returns_to_its_start_keeps_edits_that_were_already_unsaved() {
    let mut app = test_app("drag-round-trip-prior-edits");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.document.mark_mesh_edits_unsaved(layer_id);
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "fixture starts dirty"
    );

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );

    app.finish_align_drag();

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a drag that cancelled itself must not erase edits the layer already had"
    );
}

#[test]
fn a_drag_that_returns_to_its_start_keeps_another_layers_unsaved_edits() {
    let mut app = test_app("drag-round-trip-other-layer");
    let mut scene = named_scene("scene-a", 0.0);
    let other_id = push_named_layer(&mut scene, "scene-b", 20.0);
    app.document.scene = Some(Arc::new(scene));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.document.mark_mesh_edits_unsaved(other_id);

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );

    app.finish_align_drag();

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "the other layer's unsaved work must survive the round-trip drag"
    );
    assert_eq!(scene_names(&app).len(), 2);
}

#[test]
fn a_drag_that_ends_somewhere_else_is_still_one_undoable_unsaved_edit() {
    let mut app = test_app("drag-real-move");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(4.0, 0.0, 0.0)),
    );
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(1.0, 0.0, 0.0)),
    );
    let ended = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    assert_ne!(ended, start, "fixture: the scan ended somewhere else");

    assert!(app.finish_align_drag(), "a real move is a recorded edit");
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a move the operator kept is unsaved work"
    );
    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[0].transform,
        ended,
        "the pose the operator released on is the one that stays"
    );

    app.apply_history_navigation_now(false, &egui::Context::default());

    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "one Ctrl+Z returns the whole gesture"
    );
}

#[test]
fn a_round_trip_drag_does_not_clear_work_committed_mid_gesture() {
    let mut app = test_app("drag-round-trip-mid-gesture");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });

    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    assert!(app.document.has_unsaved_mesh_edits());

    app.document.mark_mesh_edits_unsaved(layer_id);

    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-3.0, 0.0, 0.0)),
    );
    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "fixture: the pose is back where it began"
    );

    app.finish_align_drag();

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "an edit that landed during the gesture must still be unsaved work"
    );
}

#[test]
fn a_second_nudge_does_not_forget_a_mid_gesture_commit() {
    let mut app = test_app("drag-round-trip-refreshed-witness");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });

    let out = Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0));
    app.nudge_align_layer(layer_id, out);
    app.document.mark_mesh_edits_unsaved(layer_id);
    app.nudge_align_layer(layer_id, out);
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(-6.0, 0.0, 0.0)),
    );
    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[0].transform,
        start,
        "fixture: the pose is back where it began"
    );

    app.finish_align_drag();

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "an edit that landed mid-gesture must survive however many nudges followed it"
    );
}

#[test]
fn an_unreleased_drag_does_not_enter_the_committed_edit_set() {
    let mut app = test_app("drag-provisional-separation");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;

    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );

    assert!(
        app.document.has_unsaved_mesh_edits(),
        "an unreleased move is work: the guards must still see it"
    );
    assert!(
        !app.document.unsaved_edit_layer_ids.contains(&layer_id),
        "but it is not a committed edit, so it must stay out of the set: a set \
         cannot tell the gesture's own mark from work that landed mid-drag"
    );

    app.finish_align_drag();

    assert!(
        app.document.unsaved_edit_layer_ids.contains(&layer_id),
        "releasing the gesture commits it into the set"
    );
}

#[test]
fn a_scene_replace_clears_an_open_drags_provisional_pose() {
    let mut app = test_app("replace-clears-drag-pose");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "fixture: the open drag is holding a moved pose"
    );

    let mut pending = delivered_load(
        &app,
        named_scene("scene-b", 10.0),
        SceneLoadMode::Replace,
        "/cases/b.stl",
    );
    pending.dirty_at_request = true;
    pending.content_revision_at_request = app.document.content_revision;
    app.apply_scene_load_result(pending, Ok(named_scene("scene-b", 10.0)));

    assert_eq!(
        scene_names(&app),
        vec!["scene-b".to_string()],
        "fixture: the confirmed replace was applied"
    );
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "the replaced scene holds no work from the old one, drag pose included"
    );
}

#[test]
fn clearing_the_scene_clears_an_open_drags_provisional_pose() {
    let mut app = test_app("clear-clears-drag-pose");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );

    app.clear_scene();

    assert!(app.document.scene.is_none(), "fixture: the scene is gone");
    assert!(
        !app.document.has_unsaved_mesh_edits(),
        "a cleared scene has nothing unsaved, drag pose included"
    );
}

#[test]
fn the_guard_save_flow_does_not_report_nothing_to_save_about_a_held_drag() {
    let mut app = test_app("guard-save-mid-drag");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "fixture: work is held"
    );

    let listed = app.pending_layer_exports();

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
        app.document.edit_mode.undo_layer_id() == Some(layer_id),
        "with the gesture released into the one recorded step"
    );
}

#[test]
fn an_append_does_not_discard_a_held_drag_pose_it_carries_forward() {
    let mut app = test_app("append-carries-held-pose");
    app.document.scene = Some(Arc::new(named_scene("scene-a", 0.0)));
    app.persistence.current_paths = vec![PathBuf::from("/cases/a.stl")];
    let layer_id = app.document.scene.as_ref().expect("scene").meshes()[0].id();
    let start = app.document.scene.as_ref().expect("scene").meshes()[0].transform;
    app.tools.align.drag = Some(AlignDrag {
        layer: layer_id,
        start,
        centroid: glam::Vec3::ZERO,
    });
    app.nudge_align_layer(
        layer_id,
        Affine3A::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
    );
    let moved = app.document.scene.as_ref().expect("scene").meshes()[0].transform;

    app.apply_scene_load_result(
        delivered_load(
            &app,
            named_scene("scene-b", 20.0),
            SceneLoadMode::Append,
            "/cases/b.stl",
        ),
        Ok(named_scene("scene-b", 20.0)),
    );

    let scene = app.document.scene.as_ref().expect("scene");
    assert_eq!(scene.meshes().len(), 2, "fixture: the append landed");
    assert_eq!(
        scene.meshes()[0].transform,
        moved,
        "fixture: the append kept the moved pose"
    );
    assert!(
        app.document.has_unsaved_mesh_edits(),
        "a pose that survived into the new scene is still unsaved work"
    );
    assert!(
        app.document.unsaved_edit_layer_ids.contains(&layer_id),
        "and it must be a committed edit, or the close guard cannot name it"
    );
    assert!(
        app.document.edit_mode.undo_layer_id() == Some(layer_id),
        "with the one history step the release would have recorded"
    );
}

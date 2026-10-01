#![allow(clippy::expect_used, clippy::panic)]

use super::pane_geometry::reconcile_single_layout;
use super::pointer::{restore_divider_ratio, workspace_owns_escape};
use crate::app::workspace::id::{PaneId, SceneKey};
use crate::app::workspace::input::{
    ActivationResult, GestureKind, InputArbiter, PaneTarget, PointerButtons, PressResult,
    ReleaseResult,
};
use crate::app::workspace::layout::WorkspaceLayout;

fn captured(kind: GestureKind) -> crate::app::workspace::input::GestureOwner {
    let target = PaneTarget {
        scene: SceneKey::from_raw_for_test(1, 1).expect("test scene key"),
        pane: PaneId::from_raw_for_test(1).expect("test pane key"),
    };
    let mut input = InputArbiter::new(target);
    match input.primary_pressed(target, kind, None) {
        PressResult::Begin(owner) => owner,
        other => panic!("expected captured press, got {other:?}"),
    }
}

#[test]
fn canceling_manual_alignment_restores_pose_and_clears_pointer_state() {
    use crate::app::align::drag::AlignDrag;
    use crate::app::app_test_support::{named_scene, test_app};
    use glam::{Affine3A, Vec3};
    use std::sync::Arc;

    let mut app = test_app("cancel-manual-align");
    let mut model = named_scene("moving", 0.0);
    let layer = model.meshes()[0].id();
    let before = Affine3A::from_translation(Vec3::X);
    model.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::Y);
    app.workspace.scenes[0].document.scene = Some(Arc::new(model));
    let mut scene = app.active_context().expect("live scene");
    scene.tools.align.drag = Some(AlignDrag {
        layer,
        start: before,
        pivot_local: Vec3::ZERO,
    });
    scene.tools.align.drag_last_pointer_pos = Some(egui::pos2(10.0, 20.0));
    scene.tools.align.drag_modifiers = Some(egui::Modifiers::CTRL);
    scene.tools.align.drag_pose_changed = true;
    scene.document.unsaved_drag_pose = true;

    crate::app::align::drag::rollback_align_drag(&mut scene);

    assert_eq!(
        scene.document.scene.as_ref().expect("scene").meshes()[0].transform,
        before
    );
    assert!(scene.tools.align.drag.is_none());
    assert!(scene.tools.align.drag_last_pointer_pos.is_none());
    assert!(scene.tools.align.drag_modifiers.is_none());
    assert!(!scene.tools.align.drag_pose_changed);
    assert!(!scene.document.unsaved_drag_pose);
}

#[test]
fn click_capture_leaves_escape_for_scene_tools() {
    let click = captured(GestureKind::Click);
    assert!(!workspace_owns_escape(false, Some(click)));
    assert!(!workspace_owns_escape(true, Some(click)));
}

#[test]
fn workspace_cancels_owned_gestures_and_uncontested_layer_drag() {
    let ruler = captured(GestureKind::Ruler);
    assert!(workspace_owns_escape(false, Some(ruler)));
    assert!(workspace_owns_escape(true, None));
    assert!(!workspace_owns_escape(false, None));
}

#[test]
fn divider_cancel_restores_ratio_and_defers_focus_cycle_until_release() {
    let first = PaneTarget {
        scene: SceneKey::from_raw_for_test(1, 1).expect("first scene key"),
        pane: PaneId::from_raw_for_test(1).expect("first pane key"),
    };
    let second = PaneTarget {
        scene: SceneKey::from_raw_for_test(2, 2).expect("second scene key"),
        pane: PaneId::from_raw_for_test(2).expect("second pane key"),
    };
    let mut input = InputArbiter::new(first);
    let initial =
        WorkspaceLayout::side_by_side(first.pane, second.pane, 0.35).expect("distinct panes");
    let mut layout = initial;
    let PressResult::Begin(owner) = input.begin_divider_resize(first, 0.35) else {
        panic!("the active divider should capture its pointer");
    };
    layout = layout.with_ratio(0.72);

    assert_eq!(
        input.request_activation(second),
        ActivationResult::DeferredUntilGestureEnds
    );
    assert_eq!(input.active(), first);

    assert_eq!(
        input.escape(PointerButtons {
            primary: true,
            ..PointerButtons::default()
        }),
        Some(owner)
    );
    assert!(restore_divider_ratio(&mut layout, owner));
    assert_eq!(layout, initial);
    assert_eq!(
        input.active(),
        first,
        "Escape must not switch scenes mid-hold"
    );

    assert_eq!(input.primary_released(), ReleaseResult::Suppressed);
    assert_eq!(
        input.active(),
        second,
        "the deferred F6 switch applies on release"
    );
}

#[test]
fn deferred_activation_reconciles_the_single_visible_pane_after_release() {
    let first = PaneTarget {
        scene: SceneKey::from_raw_for_test(1, 1).expect("first scene key"),
        pane: PaneId::from_raw_for_test(1).expect("first pane key"),
    };
    let second = PaneTarget {
        scene: SceneKey::from_raw_for_test(2, 2).expect("second scene key"),
        pane: PaneId::from_raw_for_test(2).expect("second pane key"),
    };
    let mut input = InputArbiter::new(first);
    let mut layout = WorkspaceLayout::single(first.pane);
    let PressResult::Begin(owner) = input.primary_pressed(first, GestureKind::CameraPan, None)
    else {
        panic!("the active scene should own the camera gesture");
    };

    assert_eq!(
        input.request_activation(second),
        ActivationResult::DeferredUntilGestureEnds
    );
    assert_eq!(layout, WorkspaceLayout::single(first.pane));
    assert_eq!(input.primary_released(), ReleaseResult::Finished(owner));
    assert!(reconcile_single_layout(&mut layout, input.active().pane));
    assert_eq!(layout, WorkspaceLayout::single(second.pane));
}

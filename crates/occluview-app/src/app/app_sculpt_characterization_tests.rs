#![allow(clippy::float_cmp)]

use crate::mesh_editor::mesh_editor_overlay;
use crate::sculpt::sculpt_tool::{SculptTip, SculptToolKind};
use eframe::egui;

fn wheel_input(modifiers: egui::Modifiers, delta: egui::Vec2) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events: vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers,
            },
        ],
        ..Default::default()
    }
}

fn line_wheel_input(modifiers: egui::Modifiers, delta: egui::Vec2) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events: vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn sculpt_wheel_uses_ratio_detents_and_ctrl_wins_over_shift() {
    let ctx = egui::Context::default();
    mesh_editor_overlay::set_sculpt_radius_mm(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptTip::Ball,
        0.75,
    );
    mesh_editor_overlay::set_sculpt_strength(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptToolKind::AddRemove,
        0.35,
    );
    let ctrl_shift = egui::Modifiers {
        ctrl: true,
        command: false,
        shift: true,
        ..Default::default()
    };

    let mut changed = false;
    ctx.run_ui(wheel_input(ctrl_shift, egui::vec2(0.0, 50.0)), |ui| {
        changed = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert!(changed);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "Ctrl+wheel multiplies by 1.3 and snaps to the 0.05 catalog step"
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Ball
        ),
        0.75,
        "Ctrl wins when Ctrl and Shift are both held"
    );

    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    ctx.run_ui(wheel_input(shift, egui::vec2(0.0, 50.0)), |ui| {
        changed = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert!(changed);
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Ball
        ),
        0.9
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45
    );

    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            changed = super::app_sculpt::apply_sculpt_wheel_settings(
                ui.ctx(),
                crate::app::workspace::id::SceneKey::INITIAL,
                Some(SculptToolKind::AddRemove),
            );
        },
    )
    .drop_without_applying_deltas();
    assert!(!changed, "consumed wheel input must not replay next frame");
}

#[test]
fn switching_sculpt_tips_preserves_the_normalized_radius_share() {
    let ctx = egui::Context::default();
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Ball
        ),
        0.75
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Knife
        ),
        0.55,
        "the initial Ball radius's normalized share transfers to Knife"
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Cylinder
        ),
        0.5
    );

    mesh_editor_overlay::set_sculpt_radius_mm(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptTip::Ball,
        1.25,
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Knife
        ),
        0.85
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Cylinder
        ),
        0.7
    );

    mesh_editor_overlay::set_sculpt_radius_mm(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptTip::Knife,
        1.0,
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Ball
        ),
        1.5
    );
    assert_eq!(
        mesh_editor_overlay::sculpt_radius_mm(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptTip::Cylinder
        ),
        0.85
    );
}

#[test]
fn precision_wheel_accumulates_pixels_and_line_events_each_make_a_notch() {
    let ctx = egui::Context::default();
    let ctrl = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    mesh_editor_overlay::set_sculpt_strength(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptToolKind::AddRemove,
        0.35,
    );

    let mut consumed = false;
    ctx.run_ui(wheel_input(ctrl, egui::vec2(0.0, 20.0)), |ui| {
        consumed = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert!(consumed, "the modified wheel remains owned below one notch");
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.35
    );

    ctx.run_ui(wheel_input(ctrl, egui::vec2(0.0, 20.0)), |ui| {
        consumed = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert!(consumed);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "40 px of continuous precision scrolling is one donor notch"
    );

    let line_wheel = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events: vec![
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: ctrl,
            },
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: ctrl,
            },
        ],
        ..Default::default()
    };
    ctx.run_ui(line_wheel, |ui| {
        consumed = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert!(consumed);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.8,
        "each line-unit event is one notch, including multiple events in one frame"
    );
}

#[test]
fn modified_wheel_is_owned_and_applied_once_across_discard_passes() {
    let ctx = egui::Context::default();
    let ctrl = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    mesh_editor_overlay::set_sculpt_strength(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptToolKind::AddRemove,
        0.35,
    );

    let mut pass_ownership = Vec::new();
    let output = ctx.run_ui(line_wheel_input(ctrl, egui::vec2(0.0, 1.0)), |ui| {
        pass_ownership.push(super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        ));
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx()
                .request_discard("exercise sculpt wheel multipass ownership");
        }
    });
    assert_eq!(output.platform_output.num_completed_passes, 2);
    assert_eq!(pass_ownership, [true, true]);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "one line event applies one detent even when egui repeats the UI pass"
    );
    output.drop_without_applying_deltas();

    let point_ctx = egui::Context::default();
    mesh_editor_overlay::set_sculpt_strength(
        &point_ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptToolKind::AddRemove,
        0.35,
    );
    let mut point_ownership = Vec::new();
    let output = point_ctx.run_ui(wheel_input(ctrl, egui::vec2(0.0, 20.0)), |ui| {
        point_ownership.push(super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        ));
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx()
                .request_discard("exercise precision wheel multipass ownership");
        }
    });
    assert_eq!(output.platform_output.num_completed_passes, 2);
    assert_eq!(point_ownership, [true, true]);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &point_ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.35,
        "20 points count once in the frame, not twice toward the 40 point notch"
    );
    output.drop_without_applying_deltas();

    point_ctx
        .run_ui(wheel_input(ctrl, egui::vec2(0.0, 20.0)), |ui| {
            let _ = super::app_sculpt::apply_sculpt_wheel_settings(
                ui.ctx(),
                crate::app::workspace::id::SceneKey::INITIAL,
                Some(SculptToolKind::AddRemove),
            );
        })
        .drop_without_applying_deltas();
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &point_ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "20 points from a later frame combine with the retained 20 point remainder"
    );
}

#[test]
fn busy_modified_wheel_remains_owned_if_finish_drains_before_next_pass() {
    let ctx = egui::Context::default();
    let ctrl = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    mesh_editor_overlay::set_sculpt_strength(
        &ctx,
        crate::app::workspace::id::SceneKey::INITIAL,
        SculptToolKind::AddRemove,
        0.35,
    );

    let mut pass_ownership = Vec::new();
    let output = ctx.run_ui(line_wheel_input(ctrl, egui::vec2(0.0, 1.0)), |ui| {
        if ui.ctx().current_pass_index() == 0 {
            // This is the released-but-busy Finish branch in the viewport.
            pass_ownership.push(super::app_sculpt_stroke::has_sculpt_settings_wheel(
                ui.ctx(),
                crate::app::workspace::id::SceneKey::INITIAL,
            ));
            ui.ctx()
                .request_discard("exercise Finish draining during wheel multipass");
        } else {
            // If the worker becomes idle before the next pass, the same event
            // stays owned and must not suddenly edit the brush.
            pass_ownership.push(super::app_sculpt::apply_sculpt_wheel_settings(
                ui.ctx(),
                crate::app::workspace::id::SceneKey::INITIAL,
                Some(SculptToolKind::AddRemove),
            ));
        }
    });
    assert_eq!(output.platform_output.num_completed_passes, 2);
    assert_eq!(pass_ownership, [true, true]);
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.35,
        "a wheel event consumed while Finish is busy does not apply if idle on replay"
    );
    output.drop_without_applying_deltas();

    ctx.run_ui(line_wheel_input(ctrl, egui::vec2(0.0, 1.0)), |ui| {
        let _ = super::app_sculpt::apply_sculpt_wheel_settings(
            ui.ctx(),
            crate::app::workspace::id::SceneKey::INITIAL,
            Some(SculptToolKind::AddRemove),
        );
    })
    .drop_without_applying_deltas();
    assert_eq!(
        mesh_editor_overlay::sculpt_strength(
            &ctx,
            crate::app::workspace::id::SceneKey::INITIAL,
            SculptToolKind::AddRemove
        ),
        0.45,
        "the next frame remains available for one real detent"
    );
}

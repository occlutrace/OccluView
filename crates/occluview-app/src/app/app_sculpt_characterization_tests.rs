#![allow(clippy::float_cmp)]

use crate::mesh_editor_overlay;
use eframe::egui;

fn modified_wheel_input(
    modifiers: egui::Modifiers,
    mut events: Vec<egui::Event>,
) -> egui::RawInput {
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    }
}

#[test]
fn consumer_wheel_sculpt_shift_resizes_once_and_ctrl_changes_intensity() {
    let shift_ctx = egui::Context::default();
    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    mesh_editor_overlay::set_sculpt_size(&shift_ctx, 40.0);
    let mut size_changed = false;
    shift_ctx
        .run_ui(
            modified_wheel_input(
                shift,
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 50.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: shift,
                }],
            ),
            |ui| size_changed = super::app_sculpt::apply_sculpt_wheel_settings(ui.ctx()),
        )
        .drop_without_applying_deltas();
    assert!(size_changed);
    assert_eq!(mesh_editor_overlay::sculpt_size(&shift_ctx), 46.0);

    let mut replayed = true;
    shift_ctx
        .run_ui(modified_wheel_input(shift, Vec::new()), |ui| {
            replayed = super::app_sculpt::apply_sculpt_wheel_settings(ui.ctx());
        })
        .drop_without_applying_deltas();
    assert!(
        !replayed,
        "one physical notch must not replay from smoothing"
    );
    assert_eq!(mesh_editor_overlay::sculpt_size(&shift_ctx), 46.0);

    let ctrl_ctx = egui::Context::default();
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    mesh_editor_overlay::set_sculpt_size(&ctrl_ctx, 40.0);
    mesh_editor_overlay::set_sculpt_intensity(&ctrl_ctx, 40.0);
    let mut intensity_changed = false;
    ctrl_ctx
        .run_ui(
            modified_wheel_input(
                ctrl,
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 50.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: ctrl,
                }],
            ),
            |ui| intensity_changed = super::app_sculpt::apply_sculpt_wheel_settings(ui.ctx()),
        )
        .drop_without_applying_deltas();

    assert!(intensity_changed);
    assert_eq!(mesh_editor_overlay::sculpt_size(&ctrl_ctx), 40.0);
    assert_eq!(mesh_editor_overlay::sculpt_intensity(&ctrl_ctx), 46.0);
}

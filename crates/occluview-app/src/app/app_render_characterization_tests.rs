#![allow(clippy::float_cmp)]

use eframe::egui;

#[test]
fn consumer_wheel_section_drain_prevents_camera_zoom() {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events: vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 50.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut camera = occluview_core::Camera::default();
    let initial_height = camera.orthographic_height;
    let mut section = None;
    let mut camera_changed = true;

    ctx.run_ui(input, |ui| {
        section = Some(super::disc_frame::section_panel_wheel(
            ui.ctx(),
            true,
            false,
        ));
        camera_changed = super::app_viewport::zoom_camera_from_wheel(&mut camera, ui.ctx());
    })
    .drop_without_applying_deltas();

    assert_eq!(section, Some((0.0, 1.0)));
    assert!(
        !camera_changed,
        "Section must drain the camera's wheel event"
    );
    assert_eq!(camera.orthographic_height, initial_height);
}

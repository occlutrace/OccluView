//! The pointer-and-camera arithmetic both disc tools build a frame from.
//!
//! Cut View and Bridge Split drive the same [`crate::cut_manipulator`] with the
//! same gestures over the same Section panel. Everything here was written out
//! twice, once in each tool, which is how two tools come to disagree about what
//! the wheel does over one panel.

use super::app_cut_measure::CUT_WHEEL_PX_PER_NOTCH;
use super::egui;
use crate::viewer::{project_world_to_viewport, viewport_ray};
use glam::Vec3;

/// Split a wheel event over the Section panel into disc-resize and slice-zoom
/// notches, draining it so it never reaches the camera.
///
/// Both disc tools scope the wheel the same way: the same drain, the same
/// division by [`CUT_WHEEL_PX_PER_NOTCH`], the same ctrl branch. Change the
/// gesture in one copy and an operator gets two tools whose wheels behave
/// differently over one panel.
pub(super) fn section_panel_wheel(
    ctx: &egui::Context,
    over_section_panel: bool,
    ctrl: bool,
) -> (f32, f32) {
    let raw_scroll = super::app_input::raw_wheel_delta(ctx).y;
    if !over_section_panel || raw_scroll == 0.0 {
        return (0.0, 0.0);
    }
    let raw_scroll = super::app_input::take_raw_wheel_delta(ctx).y;
    let notches = raw_scroll / CUT_WHEEL_PX_PER_NOTCH;
    if ctrl {
        (notches, 0.0)
    } else {
        (0.0, notches)
    }
}

/// The camera basis and pointer ray origin a disc frame is built from.
pub(super) struct DiscViewGeometry {
    pub(super) eye: Vec3,
    pub(super) view_dir: Vec3,
    pub(super) camera_up: Vec3,
    pub(super) camera_right: Vec3,
    pub(super) ray_origin: Vec3,
}

/// Where the pointer's ray starts and which way the camera is facing.
///
/// With no pointer the ray starts at the eye, which is what a disc nobody is
/// aiming wants.
pub(super) fn disc_view_geometry(
    camera: &occluview_core::Camera,
    viewport_rect: egui::Rect,
    pointer: Option<egui::Pos2>,
) -> DiscViewGeometry {
    let view_dir = camera.view_direction();
    let camera_up = camera.view_up();
    let eye = camera.eye();
    DiscViewGeometry {
        eye,
        view_dir,
        camera_up,
        camera_right: view_dir.cross(camera_up).normalize_or_zero(),
        ray_origin: pointer
            .and_then(|point| viewport_ray(camera, viewport_rect, point))
            .map_or(eye, |(origin, _)| origin),
    }
}

/// Where a planted disc sits on screen, and how wide it looks there.
pub(super) fn disc_screen_placement(
    camera: &occluview_core::Camera,
    viewport_rect: egui::Rect,
    pose: Option<crate::cut_manipulator::DiscPose>,
) -> (Option<egui::Pos2>, f32) {
    let center = pose.and_then(|disc| {
        project_world_to_viewport(camera, viewport_rect, disc.center).map(|(screen, _)| screen)
    });
    let radius = pose.map_or(0.0, |disc| {
        disc.radius_mm * viewport_rect.height().max(1.0) / camera.orthographic_height.max(1.0e-3)
    });
    (center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::app_input;
    use crate::viewer::zoom_factor_from_scroll;

    const SCREEN_SIZE: egui::Vec2 = egui::vec2(800.0, 600.0);

    fn point_wheel(delta: egui::Vec2, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers,
        }
    }

    fn raw_input(mut events: Vec<egui::Event>) -> egui::RawInput {
        if let Some(modifiers) = events.iter().find_map(|event| match event {
            egui::Event::MouseWheel { modifiers, .. } => Some(*modifiers),
            _ => None,
        }) {
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
        }
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN_SIZE)),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn section_panel_point_wheel_is_one_notch_and_is_drained_before_camera() {
        let ctx = egui::Context::default();
        let key = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut section = None;
        let mut camera_scroll = None;
        let mut key_still_pressed = false;

        ctx.run_ui(
            raw_input(vec![
                key,
                point_wheel(
                    egui::vec2(0.0, CUT_WHEEL_PX_PER_NOTCH),
                    egui::Modifiers::NONE,
                ),
            ]),
            |ui| {
                section = Some(section_panel_wheel(ui.ctx(), true, false));
                camera_scroll = Some(app_input::raw_wheel_delta(ui.ctx()));
                key_still_pressed = ui.ctx().input(|input| input.key_pressed(egui::Key::A));
            },
        )
        .drop_without_applying_deltas();

        assert_eq!(section, Some((0.0, 1.0)));
        assert_eq!(camera_scroll, Some(egui::Vec2::ZERO));
        assert!(
            key_still_pressed,
            "draining wheel input must not steal key input"
        );
    }

    #[test]
    fn section_panel_ctrl_wheel_resizes_instead_of_zooming() {
        let ctx = egui::Context::default();
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let mut section = None;
        let mut camera_scroll = None;

        ctx.run_ui(
            raw_input(vec![point_wheel(
                egui::vec2(0.0, CUT_WHEEL_PX_PER_NOTCH),
                ctrl,
            )]),
            |ui| {
                section = Some(section_panel_wheel(ui.ctx(), true, true));
                camera_scroll = Some(app_input::raw_wheel_delta(ui.ctx()));
            },
        )
        .drop_without_applying_deltas();

        assert_eq!(section, Some((1.0, 0.0)));
        assert_eq!(camera_scroll, Some(egui::Vec2::ZERO));
    }

    #[test]
    fn shift_wheel_routes_to_horizontal_tool_input_once() {
        let ctx = egui::Context::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let mut first_frame = None;

        ctx.run_ui(
            raw_input(vec![point_wheel(egui::vec2(0.0, 24.0), shift)]),
            |ui| first_frame = Some(app_input::raw_wheel_delta(ui.ctx())),
        )
        .drop_without_applying_deltas();

        let mut second_frame = None;
        ctx.run_ui(raw_input(Vec::new()), |ui| {
            second_frame = Some(app_input::raw_wheel_delta(ui.ctx()));
        })
        .drop_without_applying_deltas();

        assert_eq!(first_frame, Some(egui::vec2(24.0, 0.0)));
        assert_eq!(second_frame, Some(egui::Vec2::ZERO));
    }

    #[test]
    fn viewport_wheel_preserves_zoom_factor_direction() {
        let ctx = egui::Context::default();
        let mut zoom_in = None;
        ctx.run_ui(
            raw_input(vec![point_wheel(
                egui::vec2(0.0, 120.0),
                egui::Modifiers::NONE,
            )]),
            |ui| {
                zoom_in = Some(zoom_factor_from_scroll(
                    app_input::raw_wheel_delta(ui.ctx()).y,
                ));
            },
        )
        .drop_without_applying_deltas();

        let mut zoom_out = None;
        ctx.run_ui(
            raw_input(vec![point_wheel(
                egui::vec2(0.0, -120.0),
                egui::Modifiers::NONE,
            )]),
            |ui| {
                zoom_out = Some(zoom_factor_from_scroll(
                    app_input::raw_wheel_delta(ui.ctx()).y,
                ));
            },
        )
        .drop_without_applying_deltas();

        assert!(zoom_in.is_some_and(|zoom| zoom < 1.0));
        assert!(zoom_out.is_some_and(|zoom| zoom > 1.0));
    }
}

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
    let raw_scroll = ctx.input(|input| input.raw_scroll_delta.y);
    if !over_section_panel || raw_scroll == 0.0 {
        return (0.0, 0.0);
    }
    ctx.input_mut(|input| {
        input.raw_scroll_delta = egui::Vec2::ZERO;
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
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

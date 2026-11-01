use super::*;

#[test]
fn zoom_at_cursor_keeps_the_view_plane_point_under_the_cursor() {
    let viewport = Vec2::new(800.0, 600.0);
    let pointer = Vec2::new(620.0, 180.0);
    let mut camera = Camera::default();
    let right = camera.view_direction().cross(camera.view_up()).normalize();
    let up = camera.view_up();
    let old_height = camera.orthographic_height;
    let old_half_height = old_height * 0.5;
    let old_half_width = old_half_height * viewport.x / viewport.y;
    let old_ndc = Vec2::new(
        pointer.x / viewport.x * 2.0 - 1.0,
        1.0 - pointer.y / viewport.y * 2.0,
    );
    let point_before =
        camera.target + right * (old_ndc.x * old_half_width) + up * (old_ndc.y * old_half_height);

    camera.zoom_at_screen_point(0.5, pointer, viewport);

    let new_height = camera.orthographic_height;
    let new_half_height = new_height * 0.5;
    let new_half_width = new_half_height * viewport.x / viewport.y;
    let point_after =
        camera.target + right * (old_ndc.x * new_half_width) + up * (old_ndc.y * new_half_height);

    assert!(new_height < old_height);
    assert!((point_after - point_before).length() < 1.0e-4);
    assert!(camera.target.distance(Vec3::ZERO) > 0.0);
}

#[test]
fn centered_zoom_does_not_pan_the_camera_target() {
    let mut camera = Camera::default();
    let target_before = camera.target;

    camera.zoom_at_screen_point(0.5, Vec2::new(400.0, 300.0), Vec2::new(800.0, 600.0));

    assert_eq!(camera.target, target_before);
}

#[test]
fn cursor_zoom_keeps_the_orbit_pivot_when_it_pans_the_view_center() {
    let viewport = Vec2::new(800.0, 600.0);
    let pointer = Vec2::new(620.0, 180.0);
    let mut camera = Camera::default();
    let pivot_before = camera.orbit_pivot;

    camera.zoom_at_screen_point(0.5, pointer, viewport);

    assert_eq!(camera.orbit_pivot, pivot_before);
    assert_ne!(camera.target, pivot_before);
}

#[test]
fn orbit_after_cursor_zoom_rotates_the_camera_rig_around_the_stable_pivot() {
    let viewport = Vec2::new(800.0, 600.0);
    let pointer = Vec2::new(620.0, 180.0);
    let mut camera = Camera::default();
    camera.zoom_at_screen_point(0.5, pointer, viewport);
    let pivot = camera.orbit_pivot;
    let target_before = camera.target;
    let eye_radius_before = camera.eye().distance(pivot);
    let pivot_ndc_before = view_plane_ndc(&camera, pivot, viewport);

    camera.orbit_view_by(0.35, -0.2);

    assert_eq!(camera.orbit_pivot, pivot);
    assert!(camera.target.distance(target_before) > 1.0e-4);
    assert!((camera.eye().distance(pivot) - eye_radius_before).abs() < 1.0e-4);
    assert!(
        (view_plane_ndc(&camera, pivot, viewport) - pivot_ndc_before).length() < 1.0e-4,
        "orbit pivot should stay visually planted in the viewport"
    );
}

#[test]
fn trackball_after_cursor_zoom_rotates_the_camera_rig_around_the_stable_pivot() {
    let mut camera = Camera::default();
    camera.zoom_at_screen_point(0.5, Vec2::new(620.0, 180.0), Vec2::new(800.0, 600.0));
    let pivot = camera.orbit_pivot;
    let target_before = camera.target;
    let eye_radius_before = camera.eye().distance(pivot);

    camera.orbit_trackball(Vec2::new(0.0, 0.0), Vec2::new(0.25, -0.15));

    assert_eq!(camera.orbit_pivot, pivot);
    assert!(camera.target.distance(target_before) > 1.0e-4);
    assert!((camera.eye().distance(pivot) - eye_radius_before).abs() < 1.0e-4);
}

#[test]
fn yaw_pitch_orbit_after_cursor_zoom_rotates_the_camera_rig_around_the_stable_pivot() {
    let mut camera = Camera::default();
    camera.zoom_at_screen_point(0.5, Vec2::new(620.0, 180.0), Vec2::new(800.0, 600.0));
    let pivot = camera.orbit_pivot;
    let target_before = camera.target;
    let eye_radius_before = camera.eye().distance(pivot);

    camera.orbit_by(0.35, -0.2);

    assert_eq!(camera.orbit_pivot, pivot);
    assert!(camera.target.distance(target_before) > 1.0e-4);
    assert!((camera.eye().distance(pivot) - eye_radius_before).abs() < 1.0e-4);
}

#[test]
fn focus_on_resets_the_view_center_and_orbit_pivot_to_the_picked_surface() {
    let mut camera = Camera::default();
    camera.zoom_at_screen_point(0.5, Vec2::new(620.0, 180.0), Vec2::new(800.0, 600.0));
    let picked_surface = Vec3::new(12.0, -3.0, 7.0);

    camera.focus_on(picked_surface);

    assert_eq!(camera.target, picked_surface);
    assert_eq!(camera.orbit_pivot, picked_surface);
}

#[test]
fn panning_the_view_center_does_not_replace_the_orbit_pivot() {
    let mut camera = Camera::default();
    let pivot = camera.orbit_pivot;

    camera.pan_screen(Vec2::new(80.0, -40.0), Vec2::new(800.0, 600.0));

    assert_eq!(camera.orbit_pivot, pivot);
    assert_ne!(camera.target, pivot);
}

#[test]
fn axis_snap_after_cursor_zoom_keeps_the_stable_pivot_visually_planted() {
    let viewport = Vec2::new(800.0, 600.0);
    let mut camera = Camera::default();
    camera.zoom_at_screen_point(0.5, Vec2::new(620.0, 180.0), viewport);
    let pivot = camera.orbit_pivot;
    let target_before = camera.target;
    let pivot_ndc_before = view_plane_ndc(&camera, pivot, viewport);

    camera.snap_to_axis(CameraAxisView::PositiveX);

    assert_eq!(camera.orbit_pivot, pivot);
    assert!(camera.target.distance(target_before) > 1.0e-4);
    assert!(
        (view_plane_ndc(&camera, pivot, viewport) - pivot_ndc_before).length() < 1.0e-4,
        "axis snap should keep the orbit pivot visually planted"
    );
}

fn view_plane_ndc(camera: &Camera, point: Vec3, viewport: Vec2) -> Vec2 {
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    assert!(right.length_squared() > f32::EPSILON);

    let half_height = camera.orthographic_height * 0.5;
    let half_width = half_height * viewport.x / viewport.y;
    let offset = point - camera.target;
    Vec2::new(offset.dot(right) / half_width, offset.dot(up) / half_height)
}

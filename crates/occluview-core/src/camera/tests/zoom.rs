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

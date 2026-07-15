use super::*;

#[test]
fn eye_orbits_target() {
    let c = Camera {
        target: Vec3::ZERO,
        distance: 10.0,
        yaw: 0.0,
        pitch: 0.0,
        ..Camera::default()
    };
    let eye = c.eye();
    assert!(
        (eye - Vec3::new(0.0, 0.0, 10.0)).length() < 1e-4,
        "eye={eye}"
    );
}

#[test]
fn frame_occlusal_centers_on_bbox() {
    let c = Camera::default().frame_occlusal(cube_bbox(), 45.0_f32.to_radians());
    assert!((c.target - Vec3::ZERO).length() < 1e-4);
    assert!(c.distance > 10.0);
    assert_eq!(c.projection, CameraProjection::Orthographic);
    assert!(c.orthographic_height > 20.0);
}

#[test]
fn frame_occlusal_handles_empty_bbox() {
    let c = Camera::default().frame_occlusal(Aabb::EMPTY, 45.0_f32.to_radians());
    assert_eq!(c, Camera::default());
}

#[test]
fn near_far_bracket_the_scene() {
    let bbox = cube_bbox();
    let c = Camera::default().frame_occlusal(bbox, 45.0_f32.to_radians());
    let eye = c.eye();
    let forward = c.view_direction();
    let (min_depth, max_depth) = [
        Vec3::new(bbox.min.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.max.z),
    ]
    .into_iter()
    .map(|corner| (corner - eye).dot(forward))
    .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), depth| {
        (min.min(depth), max.max(depth))
    });

    assert!(c.near < min_depth, "near={} min_depth={min_depth}", c.near);
    assert!(c.far > max_depth, "far={} max_depth={max_depth}", c.far);
}

#[test]
fn camera_presets_have_toolbar_labels() {
    let labels = CameraPreset::ALL.map(CameraPreset::label);
    assert_eq!(
        labels,
        ["Occlusal", "Buccal", "Lingual", "Mesial", "Distal"]
    );
}

#[test]
fn occlusal_preset_matches_default_framing() {
    let bbox = cube_bbox();
    let fovy = 45.0_f32.to_radians();
    let preset = CameraPreset::Occlusal.frame_bbox(bbox, fovy);
    let direct = Camera::default().frame_occlusal(bbox, fovy);

    assert_eq!(preset, direct);
}

#[test]
fn buccal_and_lingual_presets_view_from_opposite_z_sides() {
    let buccal = CameraPreset::Buccal.frame_bbox(cube_bbox(), 45.0_f32.to_radians());
    let lingual = CameraPreset::Lingual.frame_bbox(cube_bbox(), 45.0_f32.to_radians());

    assert!(
        (buccal.eye().x - buccal.target.x).abs() < 1e-4,
        "eye={}",
        buccal.eye()
    );
    assert!(
        (buccal.eye().y - buccal.target.y).abs() < 1e-4,
        "eye={}",
        buccal.eye()
    );
    assert!(
        buccal.eye().z > buccal.target.z + 10.0,
        "eye={}",
        buccal.eye()
    );

    assert!(
        (lingual.eye().x - lingual.target.x).abs() < 1e-4,
        "eye={}",
        lingual.eye()
    );
    assert!(
        (lingual.eye().y - lingual.target.y).abs() < 1e-4,
        "eye={}",
        lingual.eye()
    );
    assert!(
        lingual.eye().z < lingual.target.z - 10.0,
        "eye={}",
        lingual.eye()
    );
}

#[test]
fn mesial_and_distal_presets_view_from_opposite_x_sides() {
    let mesial = CameraPreset::Mesial.frame_bbox(cube_bbox(), 45.0_f32.to_radians());
    let distal = CameraPreset::Distal.frame_bbox(cube_bbox(), 45.0_f32.to_radians());

    assert!(mesial.eye().x > mesial.target.x + 10.0);
    assert!(distal.eye().x < distal.target.x - 10.0);
    assert!((mesial.eye().z - mesial.target.z).abs() < 1e-4);
    assert!((distal.eye().z - distal.target.z).abs() < 1e-4);
}

#[test]
fn orbit_by_preserves_distance_and_allows_pitch_past_vertical() {
    let mut camera = Camera {
        distance: 120.0,
        pitch: 1.2,
        ..Camera::default()
    };

    camera.orbit_by(0.5, 0.5);

    assert_eq!(camera.distance, 120.0);
    assert!((camera.yaw - 0.5).abs() < 1e-6, "yaw={}", camera.yaw);
    assert!(
        camera.pitch > core::f32::consts::FRAC_PI_2,
        "pitch={}",
        camera.pitch
    );
}

#[test]
fn orbit_by_wraps_large_pitch_to_stable_range() {
    let mut camera = Camera {
        pitch: 0.0,
        ..Camera::default()
    };

    camera.orbit_by(0.0, core::f32::consts::TAU * 2.0 + 0.25);

    assert!(camera.pitch.abs() < 0.5, "pitch={}", camera.pitch);
}

#[test]
fn view_relative_orbit_keeps_horizontal_drag_active_from_vertical_view() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.0,
        pitch: core::f32::consts::FRAC_PI_2,
        ..Camera::default()
    };
    let before = camera.eye();

    camera.orbit_view_by(0.35, 0.0);

    let after = camera.eye();
    assert!(after.is_finite(), "eye={after}");
    assert!(
        (after - before).length() > 1.0,
        "horizontal view-relative orbit should not stall at vertical pitch: before={before} after={after}"
    );
    assert!((after.distance(camera.target) - 100.0).abs() < 1e-3);
}

#[test]
fn view_relative_orbit_vertical_drag_uses_current_screen_right_axis() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: core::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        ..Camera::default()
    };
    let before = camera.eye();

    camera.orbit_view_by(0.0, 0.30);

    let after = camera.eye();
    assert!(after.y > before.y + 1.0, "before={before} after={after}");
    assert!((after.distance(camera.target) - 100.0).abs() < 1e-3);
}

#[test]
fn view_relative_orbit_crosses_vertical_without_bouncing() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.25,
        pitch: core::f32::consts::FRAC_PI_2 - 0.04,
        ..Camera::default()
    };
    let before = camera.eye();

    camera.orbit_view_by(0.0, 0.18);
    let after = camera.eye();
    camera.orbit_view_by(0.0, 0.18);
    let after_second = camera.eye();

    let first_step = after - before;
    let second_step = after_second - after;

    assert!(camera.orientation.is_some());
    assert!(after.is_finite(), "after={after}");
    assert!(after_second.is_finite(), "after_second={after_second}");
    assert!(
        first_step.length() > 1.0 && second_step.length() > 1.0,
        "orbit should keep moving smoothly: first={first_step}, second={second_step}"
    );
    assert!(
        first_step.normalize().dot(second_step.normalize()) > 0.65,
        "orbit step reversed while crossing vertical: first={first_step}, second={second_step}"
    );
    assert!((camera.eye().distance(camera.target) - 100.0).abs() < 1e-3);
}

#[test]
fn trackball_orbit_has_no_vertical_scroll_limit_or_snap() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.15,
        pitch: core::f32::consts::FRAC_PI_2 - 0.06,
        ..Camera::default()
    };
    let mut previous_eye = camera.eye();
    let mut previous_step: Option<Vec3> = None;

    for step in 0..14 {
        let y0 = -0.72 + step as f32 * 0.12;
        let y1 = y0 + 0.12;
        camera.orbit_trackball(Vec2::new(0.08, y0), Vec2::new(0.08, y1));
        let eye = camera.eye();
        let motion = eye - previous_eye;

        assert!(eye.is_finite(), "step={step} eye={eye}");
        assert!(
            motion.length() > 0.20,
            "trackball orbit stalled while crossing vertical: step={step} motion={motion}"
        );
        if let Some(prev) = previous_step {
            assert!(
                prev.normalize().dot(motion.normalize()) > 0.20,
                "trackball orbit snapped/reversed: step={step} prev={prev} motion={motion}"
            );
        }
        assert!((eye.distance(camera.target) - 100.0).abs() < 1e-3);
        previous_step = Some(motion);
        previous_eye = eye;
    }
}

#[test]
fn trackball_orbit_dragging_down_moves_camera_down_from_front_view() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        ..Camera::default()
    };
    let before = camera.eye();

    camera.orbit_trackball(Vec2::new(0.0, 0.0), Vec2::new(0.0, -0.35));
    let after = camera.eye();

    assert!(after.y < before.y - 1.0, "before={before} after={after}");
    assert!((after.distance(camera.target) - 100.0).abs() < 1e-3);
}

#[test]
fn trackball_orbit_dragging_right_moves_camera_right_from_front_view() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        ..Camera::default()
    };
    let before = camera.eye();

    camera.orbit_trackball(Vec2::new(0.0, 0.0), Vec2::new(0.35, 0.0));
    let after = camera.eye();

    assert!(after.x > before.x + 1.0, "before={before} after={after}");
    assert!((after.distance(camera.target) - 100.0).abs() < 1e-3);
}

#[test]
fn trackball_orbit_keeps_moving_during_large_repeated_drags() {
    let mut camera = Camera {
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        ..Camera::default()
    };
    let mut previous_eye = camera.eye();

    for step in 0..24 {
        camera.orbit_trackball(Vec2::new(-0.95, 0.0), Vec2::new(0.95, 0.0));
        let eye = camera.eye();
        let motion = eye - previous_eye;

        assert!(eye.is_finite(), "step={step} eye={eye}");
        assert!(
            motion.length() > 0.05,
            "large repeated orbit should not hit a virtual mouse wall: step={step} motion={motion}"
        );
        assert!((eye.distance(camera.target) - 100.0).abs() < 1e-3);
        previous_eye = eye;
    }
}

#[test]
fn sustained_zoom_out_never_bricks_the_camera() {
    // A free-spinning wheel can deliver hundreds of notches in seconds.
    // Without a ceiling, orthographic_height overflowed to infinity, the GPU
    // matrix went NaN, and no amount of zooming back in could recover.
    let mut camera = Camera::default();
    let out = zoom_factor_from_scroll(-120.0);
    for _ in 0..10_000 {
        camera.zoom_by(out);
    }
    assert!(
        camera.orthographic_height.is_finite(),
        "height must stay finite: {}",
        camera.orthographic_height
    );

    // Zooming back in must recover to a working magnification.
    let inward = zoom_factor_from_scroll(120.0);
    for _ in 0..10_000 {
        camera.zoom_by(inward);
    }
    assert!(
        camera.orthographic_height <= 1.0,
        "zoom-in must recover after a deep zoom-out: {}",
        camera.orthographic_height
    );

    // A camera already poisoned by legacy state heals on the next zoom.
    camera.orthographic_height = f32::INFINITY;
    camera.zoom_by(out);
    assert!(camera.orthographic_height.is_finite());

    // And a poisoned height must not leak into the target through a pan.
    camera.orthographic_height = f32::NAN;
    let target_before = camera.target;
    camera.pan_screen(Vec2::new(25.0, -12.0), Vec2::new(800.0, 600.0));
    assert_eq!(camera.target, target_before);
}

#[test]
fn pointer_motion_orbit_delta_is_relative_and_unbounded() {
    let viewport = Vec2::new(400.0, 200.0);
    let right = orbit_delta_from_pointer_motion(Vec2::new(100.0, 0.0), viewport);
    let down = orbit_delta_from_pointer_motion(Vec2::new(0.0, 50.0), viewport);
    let beyond_edge = orbit_delta_from_pointer_motion(Vec2::new(360.0, 0.0), viewport);

    assert!(right.is_some(), "right drag should map");
    assert!(down.is_some(), "down drag should map");
    assert!(
        beyond_edge.is_some(),
        "large relative drags should keep rotating"
    );

    let right = right.unwrap_or(Vec2::ZERO);
    let down = down.unwrap_or(Vec2::ZERO);
    let beyond_edge = beyond_edge.unwrap_or(Vec2::ZERO);

    assert!(right.x < -1.5 && right.y.abs() < 1e-6, "right={right}");
    assert!(down.y > 0.75 && down.x.abs() < 1e-6, "down={down}");
    assert!(
        beyond_edge.x < right.x,
        "relative orbit must not clamp at a virtual edge: right={right} beyond={beyond_edge}"
    );
}

#[test]
fn pointer_motion_orbit_delta_uses_crisp_responsive_gain() {
    // Owner rule (2026-07-10): no braked camera. Dragging half the smaller
    // viewport dimension must land near a half turn, and never overshoot
    // into twitchiness.
    let viewport = Vec2::new(400.0, 200.0);
    let right =
        orbit_delta_from_pointer_motion(Vec2::new(100.0, 0.0), viewport).unwrap_or(Vec2::ZERO);

    assert!(
        right.x < -2.75 && right.x > -3.4,
        "orbit gain should feel immediate without becoming twitchy: right={right}"
    );
}

#[test]
fn fitted_clip_planes_keep_large_close_surfaces_inside_view() {
    let bbox = Aabb::from_min_max(
        Vec3::new(-240.0, -35.0, -180.0),
        Vec3::new(240.0, 65.0, 180.0),
    );
    let mut camera = Camera::default().frame_occlusal(bbox, 45.0_f32.to_radians());
    camera.target = Vec3::new(0.0, 62.0, 0.0);

    camera.fit_clip_planes_to_bbox(bbox);

    let eye = camera.eye();
    let forward = camera.view_direction();
    let min_depth = [
        Vec3::new(bbox.min.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.max.z),
    ]
    .into_iter()
    .map(|corner| (corner - eye).dot(forward))
    .fold(f32::INFINITY, f32::min);

    assert!(
        camera.near < min_depth,
        "near plane should stay in front of the closest bbox corner: near={} min_depth={}",
        camera.near,
        min_depth
    );
}

#[test]
fn clip_planes_refit_to_visible_bbox_after_camera_target_moves() {
    let bbox = Aabb::from_min_max(Vec3::new(-20.0, -8.0, 98.8), Vec3::new(20.0, 8.0, 104.0));
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        near: 2.0,
        far: 200.0,
        ..Camera::default()
    };

    camera.fit_clip_planes_to_bbox(bbox);

    assert!(
        camera.near < 0.5,
        "near plane should move in front of close geometry: near={}",
        camera.near
    );
    assert!(
        camera.far > 5.0,
        "far plane should still bracket bbox depth: far={}",
        camera.far
    );
}

#[test]
fn view_up_remains_perpendicular_at_vertical_pitch() {
    let camera = Camera {
        pitch: core::f32::consts::FRAC_PI_2,
        ..Camera::default()
    };

    let forward = camera.view_direction();
    let up = camera.view_up();

    assert!(forward.is_finite(), "forward={forward}");
    assert!(up.is_finite(), "up={up}");
    assert!(up.length() > 0.9, "up={up}");
    assert!(forward.dot(up).abs() < 1e-4, "forward={forward} up={up}");
}

#[test]
fn view_up_changes_smoothly_near_vertical_pitch() {
    let before = Camera {
        pitch: 1.24,
        ..Camera::default()
    };
    let after = Camera {
        pitch: 1.27,
        ..Camera::default()
    };

    let before_up = before.view_up();
    let after_up = after.view_up();

    assert!(
        before_up.dot(after_up) > 0.99,
        "view up should not roll-jump near vertical orbit: before={before_up} after={after_up}"
    );
}

#[test]
fn default_camera_uses_orthographic_projection_for_mesh_inspection() {
    let camera = Camera::default();

    assert_eq!(camera.projection, CameraProjection::Orthographic);
    assert!(camera.orthographic_height > 0.0);
}

#[test]
fn orthographic_zoom_scales_view_height_without_dolly() {
    let mut camera = Camera {
        projection: CameraProjection::Orthographic,
        distance: 200.0,
        orthographic_height: 80.0,
        near: 2.0,
        far: 20_000.0,
        ..Camera::default()
    };

    camera.zoom_by(0.5);

    assert_eq!(camera.distance, 200.0);
    assert!((camera.orthographic_height - 40.0).abs() < 1e-6);
    assert_eq!(camera.near, 2.0);
    assert_eq!(camera.far, 20_000.0);
}

#[test]
fn pan_screen_moves_target_in_view_plane() {
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        fovy: 60.0_f32.to_radians(),
        projection: CameraProjection::Orthographic,
        ..Camera::default()
    };

    camera.pan_screen(Vec2::new(40.0, 0.0), Vec2::new(400.0, 400.0));

    assert!(camera.target.x < 0.0, "target={}", camera.target);
    assert!(camera.target.y.abs() < 1e-6, "target={}", camera.target);
    assert!(camera.target.z.abs() < 1e-6, "target={}", camera.target);
}

#[test]
fn orthographic_pan_uses_view_height_not_camera_distance() {
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 1000.0,
        yaw: 0.0,
        pitch: 0.0,
        orthographic_height: 40.0,
        projection: CameraProjection::Orthographic,
        ..Camera::default()
    };

    camera.pan_screen(Vec2::new(40.0, 0.0), Vec2::new(400.0, 400.0));

    assert!(
        (camera.target.x + 4.0).abs() < 1e-5,
        "target={}",
        camera.target
    );
    assert!(camera.target.y.abs() < 1e-6, "target={}", camera.target);
    assert!(camera.target.z.abs() < 1e-6, "target={}", camera.target);
}

#[test]
fn pan_screen_works_near_vertical_orbit() {
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        yaw: 0.0,
        pitch: core::f32::consts::FRAC_PI_2,
        fovy: 60.0_f32.to_radians(),
        ..Camera::default()
    };

    camera.pan_screen(Vec2::new(0.0, 40.0), Vec2::new(400.0, 400.0));

    assert!(camera.target.is_finite(), "target={}", camera.target);
    assert!(camera.target.length() > 0.1, "target={}", camera.target);
}

#[test]
fn axis_views_have_stable_labels() {
    let labels = CameraAxisView::ALL.map(CameraAxisView::label);
    assert_eq!(labels, ["+X", "-X", "+Y", "-Y", "+Z", "-Z"]);
}

#[test]
fn snap_to_axis_preserves_target_distance_and_planes() {
    let mut camera = Camera {
        target: Vec3::new(10.0, 20.0, 30.0),
        distance: 250.0,
        near: 2.5,
        far: 25_000.0,
        yaw: 0.7,
        pitch: 0.4,
        ..Camera::default()
    };

    camera.snap_to_axis(CameraAxisView::NegativeX);

    let eye = camera.eye();
    assert!(eye.x < camera.target.x - 200.0, "eye={eye}");
    assert!((eye.y - camera.target.y).abs() < 1e-3, "eye={eye}");
    assert!((eye.z - camera.target.z).abs() < 1e-3, "eye={eye}");
    assert_eq!(camera.distance, 250.0);
    assert_eq!(camera.near, 2.5);
    assert_eq!(camera.far, 25_000.0);
}

#[test]
fn snap_to_vertical_axes_uses_clamped_pitch() {
    let mut camera = Camera::default();

    camera.snap_to_axis(CameraAxisView::PositiveY);
    let top_eye = camera.eye();
    assert!(top_eye.y > camera.target.y + 90.0, "eye={top_eye}");

    camera.snap_to_axis(CameraAxisView::NegativeY);
    let bottom_eye = camera.eye();
    assert!(bottom_eye.y < camera.target.y - 90.0, "eye={bottom_eye}");
}

/// The owner bug (v0.1.20): open ~10 small objects, then scroll-zoom in. As the
/// eye closes on the wide scene, the nearest corners fall behind the eye and the
/// old `near = (min_depth - padding).max(0.001)` clamp planted an invisible clip
/// plane in front of the camera, eating half the objects. Sweep 20 zoom levels
/// across several orbit angles and require no corner is ever clipped.
#[test]
fn zoomed_in_multi_object_scene_never_clips_any_corner() {
    let bbox = spread_multi_object_bbox();
    // Orbit target parked on one corner object, as a click-to-focus would do.
    let target = Vec3::new(90.0, 0.0, 90.0);

    let mut saw_corner_behind_eye = false;
    for &(yaw, pitch) in &[
        (0.0_f32, 0.6_f32),
        (0.9, 0.3),
        (-1.4, 1.1),
        (2.3, -0.4),
        (0.4, core::f32::consts::FRAC_PI_2 - 0.05),
    ] {
        for step in 0..20 {
            // Distances from a comfortable frame (200 mm) down to a very close
            // orbit (0.5 mm), where the near half of the scene is behind the eye.
            let distance = 200.0 * (0.0025_f32).powf(step as f32 / 19.0);
            let mut camera = Camera {
                target,
                distance,
                yaw,
                pitch,
                ..Camera::default()
            };
            camera.set_yaw_pitch(yaw, pitch);

            camera.fit_clip_planes_to_bbox(bbox);

            let ctx = format!("yaw={yaw} pitch={pitch} distance={distance}");
            assert_all_corners_within_clip(&camera, bbox, &ctx);

            if corner_view_depths(&camera, bbox)
                .iter()
                .any(|&depth| depth < 0.0)
            {
                saw_corner_behind_eye = true;
                // This is exactly where the old clamp failed: near must be
                // allowed to go behind the eye rather than snapping to +0.001.
                assert!(
                    camera.near < 0.0,
                    "{ctx}: near={} should follow geometry behind the eye",
                    camera.near
                );
            }
        }
    }

    assert!(
        saw_corner_behind_eye,
        "sweep never reproduced the behind-eye condition that triggers the bug"
    );
}

#[test]
fn clip_planes_handle_eye_inside_bbox() {
    // Big room-sized box with the eye orbited to sit inside it.
    let bbox = Aabb::from_min_max(Vec3::splat(-50.0), Vec3::splat(50.0));
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 5.0,
        ..Camera::default()
    };
    camera.set_yaw_pitch(0.3, 0.4);

    camera.fit_clip_planes_to_bbox(bbox);

    let depths = corner_view_depths(&camera, bbox);
    let min = depths.iter().copied().fold(f32::INFINITY, f32::min);
    let max = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(min < 0.0 && max > 0.0, "eye should be inside: {depths:?}");
    assert!(
        camera.near < 0.0,
        "near must go behind the eye when the eye is inside the box: near={}",
        camera.near
    );
    assert_all_corners_within_clip(&camera, bbox, "eye-inside-bbox");
}

#[test]
fn clip_planes_handle_single_tiny_object() {
    let bbox = Aabb::from_min_max(Vec3::splat(-0.25), Vec3::splat(0.25));
    let mut camera = Camera {
        target: Vec3::ZERO,
        distance: 2.0,
        ..Camera::default()
    };
    camera.set_yaw_pitch(0.2, 0.5);

    camera.fit_clip_planes_to_bbox(bbox);

    assert!(camera.far > camera.near, "span collapsed: {camera:?}");
    assert_all_corners_within_clip(&camera, bbox, "single-tiny-object");
}

#[test]
fn clip_planes_handle_huge_scene() {
    let bbox = Aabb::from_min_max(Vec3::splat(-5_000.0), Vec3::splat(5_000.0));
    let mut camera = Camera {
        target: Vec3::new(4_800.0, 0.0, 4_800.0),
        distance: 50.0,
        ..Camera::default()
    };
    camera.set_yaw_pitch(1.1, 0.7);

    camera.fit_clip_planes_to_bbox(bbox);

    assert!(
        camera.near.is_finite() && camera.far.is_finite(),
        "{camera:?}"
    );
    assert!(camera.far > camera.near, "span collapsed: {camera:?}");
    assert_all_corners_within_clip(&camera, bbox, "huge-scene");
}

#[test]
fn clip_planes_handle_degenerate_zero_size_bbox() {
    // A single point is not `EMPTY`, so it flows through the fit path.
    let point = Vec3::new(3.0, -2.0, 7.0);
    let bbox = Aabb::from_min_max(point, point);
    assert!(!bbox.is_empty());

    let mut camera = Camera {
        target: point,
        distance: 10.0,
        ..Camera::default()
    };
    camera.set_yaw_pitch(0.0, 0.6);

    camera.fit_clip_planes_to_bbox(bbox);

    assert!(
        camera.far > camera.near,
        "degenerate box must keep a positive clip span: near={} far={}",
        camera.near,
        camera.far
    );
    assert_all_corners_within_clip(&camera, bbox, "degenerate-bbox");
}

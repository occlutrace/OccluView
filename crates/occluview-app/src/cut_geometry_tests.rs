//! Tests for [`crate::cut_geometry`], split into their own file to hold the
//! workspace's 800-line file budget. A `#[path]` child module of
//! `cut_geometry`, so private helpers stay reachable via `super::*`.

#![allow(clippy::float_cmp, clippy::expect_used)]
use super::*;
use eframe::egui::pos2;

fn probe_input(center: Pos2, pointer: Pos2, radius_px: f32, ctrl: bool) -> CutFrameInput {
    CutFrameInput {
        pointer: Some(pointer),
        over_viewport: true,
        primary_pressed: true,
        primary_down: true,
        ctrl,
        escape: false,
        flip: false,
        wheel_notches: 0.0,
        eye: Vec3::new(0.0, 0.0, 100.0),
        view_dir: Vec3::NEG_Z,
        camera_right: Vec3::X,
        camera_up: Vec3::Y,
        ray_origin: Vec3::new(0.0, 0.0, 100.0),
        surface_hit: None,
        disc_center_screen: Some(center),
        disc_radius_screen: radius_px,
    }
}

fn pose() -> DiscPose {
    DiscPose {
        center: Vec3::ZERO,
        plane_normal: Vec3::X,
        radius_mm: 8.0,
    }
}

#[test]
fn follow_normal_contains_surface_normal_and_view_dir_without_an_arch_frame() {
    let n = follow_plane_normal(None, Vec3::ZERO, Vec3::Y, Vec3::NEG_Z, Vec3::X);
    assert!((n.length() - 1.0).abs() < 1e-6);
    assert!(n.dot(Vec3::Y).abs() < 1e-6, "off surface normal: {n}");
    assert!(n.dot(Vec3::NEG_Z).abs() < 1e-6, "perp to view: {n}");
    assert!(n.x.abs() > 0.99, "expected an X-aligned normal: {n}");
}

#[test]
fn follow_normal_degenerate_view_down_normal_falls_back_to_camera_right() {
    let right = Vec3::new(1.0, 0.0, 0.0);
    assert_eq!(
        follow_plane_normal(None, Vec3::ZERO, Vec3::Y, Vec3::NEG_Y, right),
        right
    );
    assert_eq!(
        follow_plane_normal(None, Vec3::ZERO, Vec3::Y, Vec3::Y, right),
        right
    );
}

#[test]
fn follow_normal_changes_continuously_near_the_occlusal_view_without_an_arch_frame() {
    let almost_axial = follow_plane_normal(
        None,
        Vec3::ZERO,
        Vec3::new(0.03, 0.0, 1.0),
        Vec3::NEG_Z,
        Vec3::X,
    );
    let just_past_old_threshold = follow_plane_normal(
        None,
        Vec3::ZERO,
        Vec3::new(0.04, 0.0, 1.0),
        Vec3::NEG_Z,
        Vec3::X,
    );
    assert!(
        almost_axial.dot(just_past_old_threshold) > 0.95,
        "nearby surface samples must not snap the disc: {almost_axial} / {just_past_old_threshold}"
    );
}

#[test]
fn follow_normal_is_the_local_arch_tangent_at_the_hit_point() {
    // At the arch's right extreme the spoke is +X and the occlusal axis
    // is +Z, so the along-the-arch tangent -- and with it the disc's
    // plane normal -- must be Y: the disc plane spans the occlusal axis
    // and the spoke, standing upright and cutting radially across the
    // arch. The (wildly different) surface-normal argument is irrelevant.
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    let point = Vec3::new(5.0, 0.0, 0.0);
    let n = follow_plane_normal(
        Some(frame),
        point,
        Vec3::new(0.1, 0.9, 0.3),
        Vec3::NEG_Z,
        Vec3::X,
    );
    assert!(
        n.dot(Vec3::Y).abs() > 0.999,
        "expected the along-the-arch tangent (Y): {n}"
    );
}

#[test]
fn follow_normal_never_lays_the_disc_flat_at_the_side_of_the_arch() {
    // The reported bug: from a facial/tilted view the old view-coupled
    // cross product drifted toward the vertical axis at the arch's sides,
    // so the DISC lay flat and cut "top to bottom". With an arch frame the
    // plane normal must stay IN the arch plane (zero occlusal component)
    // for every view direction: the disc always stands upright.
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    let side_point = Vec3::new(30.0, 0.0, 4.0);
    for view_dir in [Vec3::NEG_Y, Vec3::NEG_Z, Vec3::new(0.4, -0.7, -0.6)] {
        let n = follow_plane_normal(Some(frame), side_point, Vec3::X, view_dir, Vec3::X);
        assert!(
            n.z.abs() < 1e-5,
            "view {view_dir}: plane normal must stay in the arch plane, got {n}"
        );
        assert!(
            n.dot(Vec3::Y).abs() > 0.999,
            "view {view_dir}: expected the arch tangent (Y) at the side extreme, got {n}"
        );
    }
}

#[test]
fn follow_normal_with_an_arch_frame_is_immune_to_per_triangle_surface_noise() {
    // The reported bug: as the cursor crosses triangles, the LOCAL
    // surface normal jumps around; with an arch frame available and the
    // hit POINT fixed, the result must not move at all.
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::Z,
        axis1: Vec3::X,
    };
    let point = Vec3::new(0.0, 0.0, 5.0);
    let view_dir = Vec3::NEG_Y;
    let camera_right = Vec3::X;
    let baseline = follow_plane_normal(Some(frame), point, Vec3::Y, view_dir, camera_right);
    for noisy_normal in [
        Vec3::new(0.9, 0.3, 0.1),
        Vec3::new(-0.4, 0.8, -0.2),
        Vec3::new(0.05, 0.99, 0.6),
        Vec3::Z,
        -Vec3::X,
    ] {
        let out = follow_plane_normal(Some(frame), point, noisy_normal, view_dir, camera_right);
        assert_eq!(
            out, baseline,
            "an arch frame must make the result independent of local surface noise: {out}"
        );
    }
}

#[test]
fn follow_normal_stays_anatomically_planted_as_the_camera_orbits() {
    // The cut orientation is a property of the surface point, not of the
    // camera: orbiting to inspect the same spot from another angle must
    // NOT re-tilt the disc (the view-coupled re-aim is exactly what laid
    // it flat at the arch's sides from a facial view).
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    let point = Vec3::new(5.0, 0.0, 0.0);
    let camera_right = Vec3::X;
    let looking_along_neg_z =
        follow_plane_normal(Some(frame), point, Vec3::Y, Vec3::NEG_Z, camera_right);
    let looking_along_neg_y =
        follow_plane_normal(Some(frame), point, Vec3::Y, Vec3::NEG_Y, camera_right);
    assert!(
        looking_along_neg_z.dot(looking_along_neg_y).abs() > 0.999,
        "the disc must keep its anatomical orientation under orbit: \
         {looking_along_neg_z} / {looking_along_neg_y}"
    );
}

#[test]
fn follow_normal_falls_back_to_local_surface_when_no_arch_frame_is_available() {
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    let real_point = Vec3::new(5.0, 0.0, 0.0); // off the centroid: a real direction exists
    let with_frame = follow_plane_normal(Some(frame), real_point, Vec3::Y, Vec3::NEG_Z, Vec3::X);
    let without_frame = follow_plane_normal(None, real_point, Vec3::Y, Vec3::NEG_Z, Vec3::X);
    // Same inputs, but a point sitting EXACTLY at the centroid has no
    // well-defined local direction, and must behave exactly like "no
    // frame at all" rather than silently returning a zero vector.
    let at_centroid =
        follow_plane_normal(Some(frame), frame.centroid, Vec3::Y, Vec3::NEG_Z, Vec3::X);
    assert_eq!(at_centroid, without_frame);
    assert_ne!(
        with_frame, without_frame,
        "a real local arch direction must take precedence over the local fallback"
    );
}

#[test]
fn follow_normal_rotates_as_the_point_moves_around_a_curved_arch() {
    // A circle in the axis0/axis1 plane stands in for a horseshoe arch's
    // own curve; the local direction from the centroid through a point on
    // it should track that point's own angle around the curve, not stay
    // fixed for the whole mesh like the old constant axis did -- the
    // reported "disc gets stuck facing one direction as you drag along
    // the arch" bug.
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    let camera_right = Vec3::Z; // unused by the arch path; only the fallback reads it.
    let at_angle = |degrees: f32| -> Vec3 {
        let radians = degrees.to_radians();
        let point = (Vec3::X * radians.cos() + Vec3::Y * radians.sin()) * 30.0;
        follow_plane_normal(Some(frame), point, Vec3::Z, Vec3::NEG_Z, camera_right)
    };

    let start_of_arc = at_angle(0.0);
    let quarter_turn = at_angle(90.0);
    let opposite_quarter_turn = at_angle(-90.0);
    assert!(
        start_of_arc.dot(quarter_turn).abs() < 0.05,
        "a quarter turn around the arch should rotate the direction ~90 degrees, not repeat it: {start_of_arc} / {quarter_turn}"
    );
    assert!(
        (quarter_turn + opposite_quarter_turn).length() < 0.05,
        "opposite sides of the arch should read opposite directions: {quarter_turn} / {opposite_quarter_turn}"
    );

    // Continuity: a small step in angle must not snap/jitter the direction.
    let just_before = at_angle(40.0);
    let just_after = at_angle(45.0);
    assert!(
        just_before.dot(just_after) > 0.99,
        "a small move along the arch must not jitter/snap the direction: {just_before} / {just_after}"
    );
}

#[test]
fn local_arch_normal_ignores_the_out_of_plane_component() {
    let frame = ArchFrame {
        centroid: Vec3::ZERO,
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    // Offset mostly along Z (perpendicular to the arch plane -- e.g. the
    // occlusal-gingival height) plus a bit along X: the result must still
    // be pure X, ignoring the out-of-plane component entirely.
    let point = Vec3::new(5.0, 0.0, 100.0);
    let n = local_arch_normal(frame, point).expect("well-defined direction");
    assert!(
        n.distance(Vec3::X) < 1e-6,
        "expected pure X, out-of-plane height ignored: {n}"
    );
}

#[test]
fn local_arch_normal_is_none_exactly_at_the_centroid() {
    let frame = ArchFrame {
        centroid: Vec3::new(1.0, 2.0, 3.0),
        axis0: Vec3::X,
        axis1: Vec3::Y,
    };
    assert!(local_arch_normal(frame, frame.centroid).is_none());
}

#[test]
fn smoothing_blends_toward_the_new_sample() {
    let out = smooth_normal(Some(Vec3::X), Vec3::Y, 0.3);
    assert!((out.length() - 1.0).abs() < 1e-6);
    assert!(out.x > out.y, "should stay closer to the previous: {out}");
    assert!(out.y > 0.0, "should tilt toward the new: {out}");
}

#[test]
fn smoothing_without_prior_returns_the_raw_normal() {
    assert_eq!(smooth_normal(None, Vec3::Y, 0.3), Vec3::Y);
}

#[test]
fn smoothing_treats_opposite_plane_normals_as_the_same_orientation() {
    assert_eq!(smooth_normal(Some(Vec3::X), Vec3::NEG_X, 0.7), Vec3::X);
}

#[test]
fn wheel_scales_radius_and_clamps() {
    assert!((scale_radius(8.0, 1.0) - 8.8).abs() < 1e-4);
    assert_eq!(scale_radius(3.0, -100.0), MIN_DISC_RADIUS_MM);
    assert_eq!(scale_radius(50.0, 100.0), MAX_DISC_RADIUS_MM);
}

#[test]
fn center_press_begins_translate_and_wins_priority_over_the_rim() {
    let center = pos2(200.0, 200.0);
    let translate = begin_drag(&pose(), &probe_input(center, center, 40.0, false));
    assert!(matches!(translate, Some(DiscDrag::Translate { .. })));
}

#[test]
fn primary_press_anywhere_inside_disc_begins_translate() {
    let center = pos2(200.0, 200.0);
    let translate = begin_drag(
        &pose(),
        &probe_input(center, pos2(224.0, 208.0), 40.0, false),
    );
    assert!(matches!(translate, Some(DiscDrag::Translate { .. })));
}

#[test]
fn rim_press_begins_push_pull() {
    let center = pos2(200.0, 200.0);
    let rim = begin_drag(
        &pose(),
        &probe_input(center, pos2(246.0, 200.0), 40.0, false),
    );
    assert!(matches!(rim, Some(DiscDrag::PushPull { .. })));
}

#[test]
fn ctrl_press_begins_tilt_and_misses_outside_the_disc() {
    let center = pos2(200.0, 200.0);
    let tilt = begin_drag(
        &pose(),
        &probe_input(center, pos2(210.0, 205.0), 40.0, true),
    );
    assert!(matches!(tilt, Some(DiscDrag::Tilt { .. })));
    let miss = begin_drag(
        &pose(),
        &probe_input(center, pos2(400.0, 200.0), 40.0, true),
    );
    assert!(miss.is_none());
}

#[test]
fn hover_cursor_grabs_over_a_handle_only() {
    let center = pos2(200.0, 200.0);
    assert_eq!(
        hover_cursor(&pose(), &probe_input(center, center, 40.0, false)),
        CutCursor::Grab
    );
    assert_eq!(
        hover_cursor(
            &pose(),
            &probe_input(center, pos2(260.0, 200.0), 40.0, false)
        ),
        CutCursor::Default
    );
}

#[test]
fn translate_in_plane_tracks_the_pointer_and_ignores_depth() {
    let out = translate_in_plane(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 100.0),
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::NEG_Z,
    );
    assert_eq!(out, Vec3::new(3.0, -2.0, 0.0));
}

#[test]
fn push_pull_moves_only_along_the_normal() {
    let out = push_pull(
        Vec3::ZERO,
        Vec3::X,
        Vec3::new(0.0, 0.0, 100.0),
        Vec3::new(4.0, 9.0, 100.0),
    );
    assert_eq!(out, Vec3::new(4.0, 0.0, 0.0));
}

#[test]
fn arcball_no_motion_is_identity() {
    let rot = arcball_rotation(
        pos2(200.0, 200.0),
        pos2(210.0, 200.0),
        pos2(210.0, 200.0),
        40.0,
        Vec3::X,
        Vec3::Y,
        Vec3::NEG_Z,
    );
    assert!(rot.is_near_identity());
}

#[test]
fn arcball_rotation_tilts_the_normal() {
    let rot = arcball_rotation(
        pos2(200.0, 200.0),
        pos2(230.0, 200.0),
        pos2(200.0, 170.0),
        40.0,
        Vec3::X,
        Vec3::Y,
        Vec3::NEG_Z,
    );
    let tilted = (rot * Vec3::X).normalize();
    assert!((tilted.length() - 1.0).abs() < 1e-6);
    assert!(
        tilted.distance(Vec3::X) > 0.1,
        "normal should tilt: {tilted}"
    );
}

/// An L-shaped contour in the z = 0 plane, projected to panel pixels by an
/// identity XY map; the two legs share the corner (10, 0).
fn l_segments() -> [(Vec3, Vec3); 2] {
    [
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)),
        (Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.0)),
    ]
}

fn xy(w: Vec3) -> Pos2 {
    pos2(w.x, w.y)
}

#[test]
fn snap_picks_the_true_nearest_segment_point_not_a_vertex() {
    // Click hovers over the interior of the horizontal leg: the exact snap is
    // the foot of the perpendicular (5, 0), NOT the nearer polyline vertex.
    let snapped = snap_to_contour(pos2(5.0, 1.0), l_segments(), xy, 8.0);
    let snapped = snapped.expect("within radius");
    assert!(
        (snapped - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4,
        "expected the exact perpendicular foot, got {snapped}"
    );
    // The nearest vertex would be (0,0) or (10,0); prove we did better.
    assert!(snapped.distance(Vec3::new(0.0, 0.0, 0.0)) > 4.0);
}

#[test]
fn snap_returns_none_when_no_segment_is_within_radius() {
    // (5, 4) sits 4 px from the horizontal leg and 5 px from the vertical
    // leg; a 3 px radius reaches neither, so placement stays free.
    assert!(snap_to_contour(pos2(5.0, 4.0), l_segments(), xy, 3.0).is_none());
}

#[test]
fn snap_radius_is_panel_pixels_so_zoom_tightens_it() {
    // Contour is the x = 0 line; the click is 5 world units off it. A uniform
    // scale `s` (zoom) makes that a 5·s px gap. The 8 px radius catches it at
    // s = 1 but not at s = 2 — the radius stays a true on-screen distance.
    let line = [(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 0.0))];
    let world_click = Vec3::new(5.0, 3.0, 0.0);
    for (scale, expect_snap) in [(1.0_f32, true), (2.0_f32, false)] {
        let project = move |w: Vec3| pos2(w.x * scale, w.y * scale);
        let click = project(world_click);
        let snapped = snap_to_contour(click, line, project, 8.0);
        assert_eq!(
            snapped.is_some(),
            expect_snap,
            "scale {scale}: gap is {} px",
            5.0 * scale
        );
        if let Some(snapped) = snapped {
            assert!((snapped - Vec3::new(0.0, 3.0, 0.0)).length() < 1e-4);
        }
    }
}

#[test]
fn closest_param_clamps_to_the_segment_ends() {
    // Beyond `b`: clamps to t = 1. Before `a`: clamps to t = 0.
    let (t_far, _) = closest_param_on_segment(pos2(20.0, 0.0), pos2(0.0, 0.0), pos2(10.0, 0.0));
    assert!((t_far - 1.0).abs() < 1e-6);
    let (t_near, _) = closest_param_on_segment(pos2(-5.0, 0.0), pos2(0.0, 0.0), pos2(10.0, 0.0));
    assert!(t_near.abs() < 1e-6);
}

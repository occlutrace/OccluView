//! Adversarial state-machine tests for the interactive cut disc.
//!
//! These are deliberately hostile sequences aimed at the drag lifecycle, the
//! Esc ladder, the follow/plant boundary, radius clamping, the kept-side
//! freeze, and the follow-normal degeneracy threshold. They complement the
//! per-function unit tests inside `cut_manipulator` / `cut_geometry` by driving
//! the *stateful* machine through multi-frame gestures.

#![allow(clippy::float_cmp, clippy::expect_used, clippy::unnecessary_wraps)]

use crate::cut_geometry::{follow_plane_normal, scale_radius};
use crate::cut_manipulator::{
    CutFrameInput, CutManipulator, SurfaceSample, MAX_DISC_RADIUS_MM, MIN_DISC_RADIUS_MM,
};
use eframe::egui::pos2;
use glam::Vec3;

fn base() -> CutFrameInput {
    CutFrameInput {
        pointer: Some(pos2(200.0, 200.0)),
        over_viewport: true,
        primary_pressed: false,
        primary_down: false,
        ctrl: false,
        escape: false,
        flip: false,
        wheel_notches: 0.0,
        eye: Vec3::new(0.0, 0.0, 100.0),
        view_dir: Vec3::NEG_Z,
        camera_right: Vec3::X,
        camera_up: Vec3::Y,
        ray_origin: Vec3::new(0.0, 0.0, 100.0),
        surface_hit: None,
        disc_center_screen: Some(pos2(200.0, 200.0)),
        disc_radius_screen: 40.0,
    }
}

fn hit(point: Vec3, normal: Vec3) -> Option<SurfaceSample> {
    Some(SurfaceSample {
        point,
        normal,
        arch_frame: None,
    })
}

/// Arm, hover the mesh, and plant a disc centered at the origin. With
/// surface +Y and view -Z the follow plane normal resolves to -X.
fn planted() -> CutManipulator {
    let mut m = CutManipulator::default();
    m.arm();
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y),
        primary_pressed: true,
        primary_down: true,
        ..base()
    });
    assert!(m.is_planted());
    m
}

/// Grab the center handle (pointer at the projected center) and hold it.
fn grab_center(m: &mut CutManipulator) {
    let out = m.update(&CutFrameInput {
        primary_pressed: true,
        primary_down: true,
        pointer: Some(pos2(200.0, 200.0)),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    assert!(out.consumed_pointer, "grabbing a handle must consume");
}

// ── Drag-kind stability ────────────────────────────────────────────────────

#[test]
fn ctrl_pressed_mid_translate_does_not_switch_to_tilt() {
    let mut m = planted();
    grab_center(&mut m); // Translate drag now active.
    let before = m.pose().expect("pose").plane_normal;
    // Ctrl comes down mid-drag and the pointer moves: must stay a Translate
    // (normal unchanged), never silently become a Tilt.
    m.update(&CutFrameInput {
        primary_down: true,
        ctrl: true,
        pointer: Some(pos2(260.0, 140.0)),
        ray_origin: Vec3::new(6.0, 6.0, 100.0),
        ..base()
    });
    let after = m.pose().expect("pose");
    assert_eq!(
        after.plane_normal, before,
        "Ctrl mid-drag must not rotate the plane (kind is frozen at grab)"
    );
    assert!(
        after.center.distance(Vec3::ZERO) > 1.0,
        "translate must still move the center"
    );
}

#[test]
fn releasing_ctrl_mid_tilt_keeps_tilting() {
    // Grab with Ctrl down => Tilt. Dropping Ctrl mid-gesture must not turn it
    // into a translate.
    let mut m = planted();
    let out = m.update(&CutFrameInput {
        primary_pressed: true,
        primary_down: true,
        ctrl: true,
        pointer: Some(pos2(230.0, 200.0)),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    assert!(out.consumed_pointer);
    let center_before = m.pose().expect("pose").center;
    m.update(&CutFrameInput {
        primary_down: true,
        ctrl: false, // Ctrl released mid-drag.
        pointer: Some(pos2(200.0, 150.0)),
        ray_origin: Vec3::new(9.0, 9.0, 100.0),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    let after = m.pose().expect("pose");
    assert_eq!(
        after.center, center_before,
        "tilt must never translate the center even after Ctrl is released"
    );
}

// ── Wheel + flip during an active drag ─────────────────────────────────────

#[test]
fn wheel_during_translate_scales_radius_and_keeps_translating() {
    let mut m = planted();
    grab_center(&mut m);
    let r0 = m.pose().expect("pose").radius_mm;
    m.update(&CutFrameInput {
        primary_down: true,
        wheel_notches: 3.0,
        ray_origin: Vec3::new(4.0, 0.0, 100.0),
        ..base()
    });
    let p = m.pose().expect("pose");
    assert!(p.radius_mm > r0, "wheel must still size the disc mid-drag");
    assert!(
        (p.radius_mm - scale_radius(r0, 3.0)).abs() < 1e-3,
        "radius follows the clamped wheel step"
    );
    assert!(p.center.x > 1.0, "translate still tracks the pointer");
}

#[test]
fn flip_during_drag_flips_side_without_disturbing_the_drag() {
    let mut m = planted();
    grab_center(&mut m);
    let clip_before = m.clip(Vec3::new(0.0, 0.0, 100.0)).expect("clip");
    m.update(&CutFrameInput {
        primary_down: true,
        flip: true,
        ray_origin: Vec3::new(3.0, 0.0, 100.0),
        ..base()
    });
    let clip_after = m.clip(Vec3::new(0.0, 0.0, 100.0)).expect("clip");
    assert_eq!(clip_after.0, -clip_before.0, "F must flip the kept side");
    assert!(m.is_planted());
}

// ── Esc during a drag ──────────────────────────────────────────────────────

#[test]
fn esc_mid_drag_aborts_the_drag_and_unplants() {
    let mut m = planted();
    grab_center(&mut m);
    // Escape arrives while the button is still held mid-drag.
    let out = m.update(&CutFrameInput {
        primary_down: true,
        escape: true,
        ray_origin: Vec3::new(20.0, 0.0, 100.0),
        ..base()
    });
    assert!(out.unplanted, "first Esc steps planted -> follow");
    assert!(!out.exited);
    assert!(m.is_active() && !m.is_planted(), "now in follow, no disc");
    assert!(m.pose().is_none(), "follow pose cleared on unplant");
    // A second Esc (still, defensively, with the button down) exits.
    let out = m.update(&CutFrameInput {
        escape: true,
        primary_down: true,
        ..base()
    });
    assert!(out.exited);
    assert!(!m.is_active());
}

// ── Pointer leaving the window mid-drag ────────────────────────────────────

#[test]
fn pointer_none_mid_translate_keeps_the_disc_planted_and_alive() {
    // If the pointer report vanishes for a frame (focus blip) while the button
    // is still down, the drag must not crash or unplant; it stays a live drag.
    let mut m = planted();
    grab_center(&mut m);
    let out = m.update(&CutFrameInput {
        primary_down: true,
        pointer: None,
        disc_center_screen: None,
        ray_origin: Vec3::new(2.0, 0.0, 100.0),
        ..base()
    });
    assert!(
        out.consumed_pointer,
        "an active drag keeps owning the pointer"
    );
    assert!(m.is_planted());
}

#[test]
fn button_release_ends_the_drag_cleanly() {
    let mut m = planted();
    grab_center(&mut m);
    // One held-and-moved frame so there is a real committed displacement.
    m.update(&CutFrameInput {
        primary_down: true,
        ray_origin: Vec3::new(7.0, 0.0, 100.0),
        ..base()
    });
    let moved = m.pose().expect("pose").center;
    assert!(moved.x > 1.0, "the held drag actually moved the disc");
    // Release frame: the gesture is completing, so it still consumes the
    // pointer this frame (nothing else should act on the up-edge) and clears
    // the drag. The pose must NOT jump to the release-frame ray origin.
    let out = m.update(&CutFrameInput {
        primary_down: false,
        ray_origin: Vec3::new(30.0, 0.0, 100.0),
        ..base()
    });
    assert!(
        out.consumed_pointer,
        "the up-edge completes the drag gesture"
    );
    assert!(m.is_planted());
    assert_eq!(
        m.pose().expect("pose").center,
        moved,
        "release must commit the last held pose, not lurch to the up-frame ray"
    );
    // The very next idle frame is fully free again (drag was cleared).
    let idle = m.update(&CutFrameInput {
        primary_down: true, // held again, but no fresh press edge
        ray_origin: Vec3::new(90.0, 0.0, 100.0),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    assert!(
        !idle.consumed_pointer,
        "no drag is armed after release, so the pointer is free"
    );
    assert_eq!(
        m.pose().expect("pose").center,
        moved,
        "a held button with no fresh grab must not resurrect the drag"
    );
}

// ── Plant gating ───────────────────────────────────────────────────────────

#[test]
fn press_off_viewport_never_plants() {
    // Over the strip / a panel: over_viewport is false, so a press cannot plant
    // even if a stale surface hit is present.
    let mut m = CutManipulator::default();
    m.arm();
    let out = m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y),
        primary_pressed: true,
        primary_down: true,
        over_viewport: false,
        ..base()
    });
    assert!(!out.planted);
    assert!(!m.is_planted());
    assert!(
        !out.consumed_pointer,
        "no plant => no pointer theft off-viewport"
    );
}

#[test]
fn press_without_a_surface_hit_never_plants() {
    let mut m = CutManipulator::default();
    m.arm();
    let out = m.update(&CutFrameInput {
        surface_hit: None,
        primary_pressed: true,
        primary_down: true,
        ..base()
    });
    assert!(!out.planted && !m.is_planted());
}

#[test]
fn primary_down_without_pressed_does_not_plant() {
    // The adapter suppresses `primary_pressed` when another gesture (armed
    // lasso) owns LMB; the machine must then follow, not plant, even though the
    // button is physically held.
    let mut m = CutManipulator::default();
    m.arm();
    let out = m.update(&CutFrameInput {
        surface_hit: hit(Vec3::new(1.0, 0.0, 0.0), Vec3::Y),
        primary_pressed: false,
        primary_down: true,
        ..base()
    });
    assert!(!out.planted, "no press edge => follow, not plant");
    assert!(m.pose().is_some(), "still following the surface");
}

// ── Radius clamp at both ends ──────────────────────────────────────────────

#[test]
fn wheel_radius_clamps_hard_at_both_ends() {
    let mut m = CutManipulator::default();
    m.arm();
    // Hover so a pose exists, then shrink far past the floor.
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y),
        wheel_notches: -500.0,
        ..base()
    });
    assert_eq!(m.pose().expect("pose").radius_mm, MIN_DISC_RADIUS_MM);
    // Now grow far past the ceiling.
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y),
        wheel_notches: 500.0,
        ..base()
    });
    assert_eq!(m.pose().expect("pose").radius_mm, MAX_DISC_RADIUS_MM);
}

// ── Kept-side freeze at plant ──────────────────────────────────────────────

#[test]
fn kept_side_freezes_at_plant_and_ignores_later_camera_moves() {
    // Plant with the eye on +X of an X-facing plane; keep_positive is frozen.
    // Orbiting the camera to the other side must NOT flip the clip side.
    let mut m = CutManipulator::default();
    m.arm();
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y), // plane normal ends up +X
        eye: Vec3::new(50.0, 0.0, 0.0),
        primary_pressed: true,
        primary_down: true,
        ..base()
    });
    let side_at_plant = m.clip(Vec3::new(50.0, 0.0, 0.0)).expect("clip").0;
    // Evaluate the clip from the OPPOSITE eye: planted side must not depend on
    // eye at all.
    let side_from_far_side = m.clip(Vec3::new(-50.0, 0.0, 0.0)).expect("clip").0;
    assert_eq!(
        side_at_plant, side_from_far_side,
        "planted clip side is frozen and eye-independent"
    );
}

#[test]
fn follow_side_tracks_the_camera_but_planted_does_not() {
    // In follow mode the kept side follows the eye; the same disc planted keeps
    // whatever side it had at plant.
    let mut m = CutManipulator::default();
    m.arm();
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::Y),
        ..base()
    });
    let from_plus = m.clip(Vec3::new(50.0, 0.0, 0.0)).expect("clip").0;
    let from_minus = m.clip(Vec3::new(-50.0, 0.0, 0.0)).expect("clip").0;
    assert_eq!(
        from_plus, -from_minus,
        "follow clip flips with the eye (camera-facing kept side)"
    );
}

// ── Follow-normal degeneracy threshold ─────────────────────────────────────

#[test]
fn follow_normal_blends_continuously_across_the_old_degenerate_boundary() {
    // The old hard threshold snapped from camera-right to the surface cross.
    // Nearby samples on opposite sides of that boundary must now stay nearby.
    let right = Vec3::new(0.0, 0.0, 1.0);
    let surface = Vec3::Y;
    let nearly_down = {
        let a = (0.9e-3_f32).sqrt().asin(); // sin^2 = 0.9e-3 < 1e-3
        Vec3::new(0.0, -a.cos(), a.sin()).normalize()
    };
    let n_deg = follow_plane_normal(None, Vec3::ZERO, surface, nearly_down, right);
    let clearly_off = {
        let a = (1.1e-3_f32).sqrt().asin(); // sin^2 = 1.1e-3 > 1e-3
        Vec3::new(0.0, -a.cos(), a.sin()).normalize()
    };
    let n_ok = follow_plane_normal(None, Vec3::ZERO, surface, clearly_off, right);
    assert!((n_deg.length() - 1.0).abs() < 1e-5);
    assert!((n_ok.length() - 1.0).abs() < 1e-5);
    assert!(
        n_deg.dot(n_ok) > 0.999,
        "straddling the old boundary must not snap: {n_deg} / {n_ok}"
    );
}

// ── Temporal smoothing damps a sharp-edge flip-flop ────────────────────────

#[test]
fn smoothing_damps_a_normal_jump_across_a_sharp_edge() {
    // Sweep the cursor so the raw surface normal jumps +X -> -X-ish (a scan
    // edge). The blended follow normal must move gradually, never snap, and
    // never oscillate frame to frame.
    let mut m = CutManipulator::default();
    m.arm();
    // Settle on surface +Y (follow normal -X) so the filter has real state.
    for _ in 0..8 {
        m.update(&CutFrameInput {
            surface_hit: hit(Vec3::ZERO, Vec3::Y),
            ..base()
        });
    }
    let n0 = m.pose().expect("pose").plane_normal;
    // Cross a sharp edge: surface flips to +X (follow normal +Y), 90 deg away,
    // for ONE frame.
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::X),
        ..base()
    });
    let n1 = m.pose().expect("pose").plane_normal;
    let step1 = n0.distance(n1);
    // Hold the new normal: second frame moves further in the SAME direction,
    // and by no more than the first step (a damped approach, not a bounce).
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::ZERO, Vec3::X),
        ..base()
    });
    let n2 = m.pose().expect("pose").plane_normal;
    let step2 = n1.distance(n2);
    assert!(step1 > 1e-4, "the normal does move toward the new sample");
    assert!(
        step2 <= step1 + 1e-4,
        "each blended step must not exceed the last (no oscillation): {step1} then {step2}"
    );
    assert!((n2.length() - 1.0).abs() < 1e-5, "stays unit length");
}

// ── Handle hit-test priority under Ctrl ────────────────────────────────────

#[test]
fn ctrl_press_anywhere_on_the_disc_begins_tilt_not_translate() {
    // With Ctrl held, even a press dead-center must be a Tilt (Ctrl wins over
    // the center-translate handle).
    let mut m = planted();
    let out = m.update(&CutFrameInput {
        primary_pressed: true,
        primary_down: true,
        ctrl: true,
        pointer: Some(pos2(200.0, 200.0)),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    assert!(out.consumed_pointer);
    // Move the pointer and confirm the normal rotates (tilt), center is fixed.
    let c0 = m.pose().expect("pose").center;
    m.update(&CutFrameInput {
        primary_down: true,
        ctrl: true,
        pointer: Some(pos2(240.0, 160.0)),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    let p = m.pose().expect("pose");
    assert_eq!(p.center, c0, "tilt keeps the center fixed");
}

// ── A miss on a planted disc leaves LMB free for other tools ───────────────

#[test]
fn press_that_misses_all_handles_does_not_consume() {
    // This is the load-bearing coexistence contract: a planted disc consumes
    // ONLY its own handle presses. A press well outside the disc must leave the
    // pointer free (so lasso / marquee / face-pick / camera still work).
    let mut m = planted();
    let out = m.update(&CutFrameInput {
        primary_pressed: true,
        primary_down: true,
        pointer: Some(pos2(600.0, 600.0)), // far from center (200,200) & rim
        disc_center_screen: Some(pos2(200.0, 200.0)),
        disc_radius_screen: 40.0,
        ..base()
    });
    assert!(
        !out.consumed_pointer,
        "a miss must not steal the press from other tools"
    );
    assert!(m.is_planted(), "the disc stays planted, just not grabbed");
    // And no drag is armed.
    let out2 = m.update(&CutFrameInput {
        primary_down: true,
        ray_origin: Vec3::new(99.0, 0.0, 100.0),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..base()
    });
    assert!(!out2.consumed_pointer, "no phantom drag after a miss");
    assert_eq!(
        m.pose().expect("pose").center,
        Vec3::ZERO,
        "center untouched"
    );
}

// ── View-locked planted disc: an RMB orbit sweeps the section ───────────────

/// Plant a disc off-target so an orbit sweeps both its center and its normal.
/// Surface +Y, view -Z, camera-right +X => follow plane normal resolves to -X.
fn planted_off_target() -> CutManipulator {
    let mut m = CutManipulator::default();
    m.arm();
    m.update(&CutFrameInput {
        surface_hit: hit(Vec3::new(5.0, 0.0, 0.0), Vec3::Y),
        primary_pressed: true,
        primary_down: true,
        ..base()
    });
    assert!(m.is_planted());
    let pose = m.pose().expect("planted pose");
    assert!(
        (pose.plane_normal - Vec3::NEG_X).length() < 1e-4,
        "sanity: planted normal is -X, got {}",
        pose.plane_normal
    );
    assert!((pose.center - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
    m
}

/// A frame carrying a camera basis yawed +90° about world Y (an orbit): the
/// basis (right, forward) rotates from (X, -Z) to (-Z, -X). A world-fixed
/// planted disc must ignore this entirely.
fn orbited_frame() -> CutFrameInput {
    CutFrameInput {
        camera_right: Vec3::NEG_Z,
        view_dir: Vec3::NEG_X,
        ..base()
    }
}

#[test]
fn idle_frame_leaves_the_planted_disc_fixed() {
    // An unchanged camera must reproduce the pose EXACTLY and report no pose
    // change (guards against spurious per-frame re-renders while idle).
    let mut m = planted_off_target();
    let before = m.pose().expect("pose");
    let out = m.update(&base());
    let after = m.pose().expect("pose");
    assert_eq!(after.center, before.center, "idle must not drift center");
    assert_eq!(
        after.plane_normal, before.plane_normal,
        "idle must not drift normal"
    );
    assert!(!out.pose_changed, "idle camera must not force a re-render");
}

#[test]
fn orbit_does_not_move_the_world_fixed_disc() {
    // Owner rule: with a disc planted, orbiting the main-viewport camera must
    // NOT sweep the section — the world pose stays put and nothing re-renders.
    let mut m = planted_off_target();
    let before = m.pose().expect("pose");
    let out = m.update(&orbited_frame());
    let after = m.pose().expect("pose");
    assert!(
        !out.pose_changed,
        "a camera orbit must not re-render a world-fixed section"
    );
    assert_eq!(
        after.center, before.center,
        "orbit must not move the planted center: {} -> {}",
        before.center, after.center
    );
    assert_eq!(
        after.plane_normal, before.plane_normal,
        "orbit must not rotate the planted normal: {} -> {}",
        before.plane_normal, after.plane_normal
    );
}

#[test]
fn handles_edit_the_disc_regardless_of_camera() {
    // The disc is authored by its handles, not the camera. Even under an
    // orbited basis (which the planted disc ignores) the center handle must
    // still grab, drag, and hold its edit.
    let mut m = planted_off_target();
    m.update(&orbited_frame());
    let planted_center = m.pose().expect("pose").center;

    let grab = m.update(&CutFrameInput {
        primary_pressed: true,
        primary_down: true,
        pointer: Some(pos2(200.0, 200.0)),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..orbited_frame()
    });
    assert!(grab.consumed_pointer, "the center handle must grab");
    let drag = m.update(&CutFrameInput {
        primary_down: true,
        ray_origin: Vec3::new(9.0, 4.0, 100.0),
        disc_center_screen: Some(pos2(200.0, 200.0)),
        ..orbited_frame()
    });
    assert!(drag.pose_changed, "the drag must move the pose");
    let edited = m.pose().expect("pose").center;
    assert!(
        edited.distance(planted_center) > 1.0,
        "the handle must move the center: {planted_center} -> {edited}"
    );

    // Release and hold still: the edit must persist, not snap back.
    m.update(&orbited_frame());
    let settled = m.pose().expect("pose").center;
    assert!(
        settled.distance(edited) < 1e-3,
        "an edit must not snap back: {edited} -> {settled}"
    );
}

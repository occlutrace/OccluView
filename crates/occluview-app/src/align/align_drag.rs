//! Moving a scan by hand, in the frame the operator is looking at.
//!
//! The viewport camera is orthographic, so a pixel maps to a fixed number of
//! millimetres regardless of depth: the conversion here is exact, not an
//! approximation that drifts as the operator zooms.

use eframe::egui;
use glam::{Affine3A, Quat, Vec3};

/// Largest turn one frame of a Ctrl-drag may apply, in radians.
///
/// Pointer motion can arrive coalesced after a stall. A per-frame cap keeps one
/// delayed event from snapping the scan through a large angle.
pub(crate) const MAX_TILT_STEP_RAD: f32 = 0.18;

/// Which directions a hand drag is allowed to move in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DragConstraint {
    /// Move in any direction.
    #[default]
    Free,
    /// Move only along the world Z axis.
    ZOnly,
    /// Move only within the world XY plane.
    XyPlane,
}

impl DragConstraint {
    /// Catalog key rendering the localized constraint label.
    pub(crate) fn label_key(self) -> crate::i18n::MessageId {
        match self {
            Self::Free => crate::i18n::message_id!("align-constraint-free"),
            Self::ZOnly => crate::i18n::message_id!("align-constraint-z"),
            Self::XyPlane => crate::i18n::message_id!("align-constraint-xy"),
        }
    }

    /// The glyph the panel shows.
    pub(crate) fn icon(self) -> crate::ui::icons::AppIcon {
        use crate::ui::icons::AppIcon;
        match self {
            Self::Free => AppIcon::MoveLayer,
            Self::ZOnly => AppIcon::MoveVertical,
            Self::XyPlane => AppIcon::MovePlane,
        }
    }

    /// Catalog key rendering the localized one-line hint.
    pub(crate) fn hint_key(self) -> crate::i18n::MessageId {
        match self {
            Self::Free => crate::i18n::message_id!("align-constraint-free-hint"),
            Self::ZOnly => crate::i18n::message_id!("align-constraint-z-hint"),
            Self::XyPlane => crate::i18n::message_id!("align-constraint-xy-hint"),
        }
    }
}

/// How many millimetres one viewport pixel spans.
///
/// The brush ring and the hand drag both need this, in opposite directions, so
/// they share one degenerate-input guard. Both operands are floored here, once:
/// a zero-height viewport or a near-zero camera height stays finite on both
/// paths.
pub(crate) fn mm_per_pixel(orthographic_height: f32, viewport_height: f32) -> f32 {
    orthographic_height.max(f32::EPSILON) / viewport_height.max(1.0)
}

/// How short a constrained axis, or how flat a constrained plane, may look on
/// screen and still be dragged along. Below it, keeping the scan under the
/// cursor would move it more than five times the cursor's travel, most of it
/// in depth, where the operator cannot see it.
const MIN_CONSTRAINT_FORESHORTENING: f32 = 0.2;

/// The motion inside a constraint that keeps the scan under the cursor.
///
/// `delta` is the cursor's travel across the view plane, in world units, and
/// `toward_viewer` is the view plane's normal. Along the constrained axis the
/// scan moves by whatever makes its picture travel as far along the axis as
/// the cursor did; within the constrained plane it moves by whatever makes its
/// picture travel exactly as the cursor did. A direction the view shows
/// end-on or edge-on cannot be dragged, and is left out instead of being made
/// up for in depth.
pub(crate) fn constrain_translation(
    delta: Vec3,
    constraint: DragConstraint,
    toward_viewer: Vec3,
) -> Vec3 {
    let normal = toward_viewer.normalize_or_zero();
    match constraint {
        DragConstraint::Free => delta,
        DragConstraint::ZOnly => {
            // The squared length of the Z axis as the view shows it.
            let shown = 1.0 - normal.z * normal.z;
            if shown < MIN_CONSTRAINT_FORESHORTENING * MIN_CONSTRAINT_FORESHORTENING {
                Vec3::ZERO
            } else {
                Vec3::new(0.0, 0.0, delta.z / shown)
            }
        }
        DragConstraint::XyPlane => {
            if normal.z.abs() < MIN_CONSTRAINT_FORESHORTENING {
                // Edge-on, the plane shows as one line: the one it shares with
                // the view plane.
                let shared = Vec3::Z.cross(normal).normalize_or_zero();
                shared * delta.dot(shared)
            } else {
                let in_plane = delta - normal * (delta.z / normal.z);
                Vec3::new(in_plane.x, in_plane.y, 0.0)
            }
        }
    }
}

/// Convert a screen drag into a world translation across the view plane.
///
/// `world_per_pixel` is the orthographic height over the viewport height. The
/// screen y axis points down and the camera's up axis points up, hence the
/// negation.
pub(crate) fn screen_delta_to_world(
    delta_px: egui::Vec2,
    camera_right: Vec3,
    camera_up: Vec3,
    world_per_pixel: f32,
) -> Vec3 {
    if !delta_px.is_finite()
        || !camera_right.is_finite()
        || !camera_up.is_finite()
        || !world_per_pixel.is_finite()
        || world_per_pixel <= 0.0
    {
        return Vec3::ZERO;
    }
    let delta =
        camera_right * (delta_px.x * world_per_pixel) - camera_up * (delta_px.y * world_per_pixel);
    if delta.is_finite() {
        delta
    } else {
        Vec3::ZERO
    }
}

/// A grab this many radii from the scan's centre is not a point on the scan;
/// it is read as a grab at the centre.
pub(crate) const MAX_GRAB_RADII: f32 = 4.0;

/// What one Ctrl-drag step turns a scan in: the camera's axes and the scan as
/// it stands.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TurnFrame {
    /// Unit vector from the scene towards the viewer.
    pub(crate) toward_viewer: Vec3,
    /// The camera's right axis.
    pub(crate) camera_right: Vec3,
    /// The camera's up axis.
    pub(crate) camera_up: Vec3,
    /// The world point the scan turns about: its own centre, so it turns in
    /// place instead of swinging away from the scan it is being seated on.
    pub(crate) centre: Vec3,
    /// The scan's bounding radius, in world units: the radius of the ball.
    pub(crate) radius: f32,
}

impl TurnFrame {
    /// The camera's right, up and towards-the-viewer axes, when they form the
    /// right-handed orthonormal basis the gesture is defined in.
    ///
    /// A partial or skewed basis would silently turn a two-axis gesture into
    /// one-axis motion or reverse it, so anything else turns nothing.
    fn axes(self) -> Option<[Vec3; 3]> {
        let right = self.camera_right.normalize_or_zero();
        let up = self.camera_up.normalize_or_zero();
        let toward = self.toward_viewer.normalize_or_zero();
        let handedness = right.cross(up).dot(toward);
        (handedness.is_finite()
            && handedness >= 0.99
            && self.centre.is_finite()
            && self.radius.is_finite()
            && self.radius > 1e-3)
            .then_some([right, up, toward])
    }
}

/// Where a point of the view plane lies on the virtual trackball, in camera
/// axes (right, up, towards the viewer).
///
/// Bell's trackball: a sphere of the scan's radius over the middle of the
/// picture and a hyperbolic sheet around it, meeting where the two have the
/// same height. The sheet is what keeps the surface continuous past the
/// sphere's outline, so the turn never jumps and a drag out there rolls the
/// scan about the view axis.
fn ball_point(offset: glam::Vec2, radius: f32) -> Vec3 {
    let reach = offset.length();
    let height = if reach < radius * std::f32::consts::FRAC_1_SQRT_2 {
        (radius * radius - reach * reach).sqrt()
    } else {
        radius * radius / (2.0 * reach)
    };
    offset.extend(height)
}

/// Turn a scan about its centre the way the cursor pushes it.
///
/// `grabbed` is where the grabbed surface point is now and `travel` is how far
/// the cursor moved across the view plane since, both in world units. The
/// scan is turned as a ball of its own radius would be (the virtual trackball
/// of Chen et al., in Bell's continuous form): the place on the ball under the
/// cursor goes with the cursor. Over the middle of the scan that tilts it
/// towards the drag; out at its outline the same drag rolls it about the view
/// axis, so one gesture reaches every orientation.
pub(crate) fn turn_following_grab(grabbed: Vec3, travel: Vec3, frame: TurnFrame) -> Quat {
    let Some([right, up, toward]) = frame.axes() else {
        return Quat::IDENTITY;
    };
    if !travel.is_finite() {
        return Quat::IDENTITY;
    }
    let arm = grabbed - frame.centre;
    let mut from = glam::Vec2::new(arm.dot(right), arm.dot(up));
    // A point of the scan cannot be far outside its own bounding sphere.
    if !from.is_finite() || from.length() > frame.radius * MAX_GRAB_RADII {
        from = glam::Vec2::ZERO;
    }
    let to = from + glam::Vec2::new(travel.dot(right), travel.dot(up));
    let handle = ball_point(from, frame.radius);
    let target = ball_point(to, frame.radius);
    // The arc from the handle to its target, taken from the cross and dot
    // products directly: a slow drag turns the scan by well under a milliradian
    // a step, which a cosine alone cannot tell from no turn at all.
    let normal = handle.cross(target);
    let angle = normal.length().atan2(handle.dot(target));
    let world_axis = (right * normal.x + up * normal.y + toward * normal.z).normalize_or_zero();
    if !angle.is_finite() || angle <= 0.0 || world_axis.length_squared() <= f32::EPSILON {
        return Quat::IDENTITY;
    }
    Quat::from_axis_angle(world_axis, angle.min(MAX_TILT_STEP_RAD))
}

/// Turn a scan around a world-space pivot.
///
/// The returned step is a world-space transform, meant to be pre-multiplied onto
/// the layer's pose exactly like the translation step.
///
/// A non-finite pivot or rotation yields the identity rather than turning
/// around an arbitrary point.
pub(crate) fn rotation_about_pivot(turn: Quat, pivot: Vec3) -> Affine3A {
    if !pivot.is_finite() || !turn.is_finite() {
        return Affine3A::IDENTITY;
    }
    Affine3A::from_translation(pivot)
        * Affine3A::from_quat(turn)
        * Affine3A::from_translation(-pivot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scan of radius 20 centred on the origin, seen down the -Z axis.
    fn front_view() -> TurnFrame {
        TurnFrame {
            toward_viewer: Vec3::Z,
            camera_right: Vec3::X,
            camera_up: Vec3::Y,
            centre: Vec3::ZERO,
            radius: 20.0,
        }
    }

    /// One conversion, one guard: the brush ring and the hand drag share it, so
    /// a zero-height viewport yields a finite scale on both paths.
    #[test]
    fn a_degenerate_viewport_never_produces_an_infinity() {
        for (camera_mm, viewport_px) in [
            (0.0, 0.0),
            (0.0, 800.0),
            (40.0, 0.0),
            (f32::EPSILON, 0.5),
            (1e9, 1.0),
        ] {
            let scale = mm_per_pixel(camera_mm, viewport_px);
            assert!(
                scale.is_finite() && scale > 0.0,
                "{camera_mm} mm over {viewport_px} px gave {scale}"
            );
        }
    }

    /// The ordinary case is exact: an orthographic camera has no perspective to
    /// approximate away.
    #[test]
    fn a_pixel_spans_the_camera_height_over_the_viewport_height() {
        assert!((mm_per_pixel(40.0, 800.0) - 0.05).abs() < f32::EPSILON);
        assert!(
            mm_per_pixel(40.0, 400.0) > mm_per_pixel(40.0, 800.0),
            "a shorter viewport puts more millimetres in a pixel"
        );
    }

    /// The default view: tilted over the occlusal plane, so neither the Z axis
    /// nor the XY plane is seen square on.
    fn tilted_view() -> (Vec3, Vec3, Vec3) {
        let (sin, cos) = 0.6_f32.sin_cos();
        (Vec3::X, Vec3::new(0.0, cos, -sin), Vec3::new(0.0, sin, cos))
    }

    /// What the view shows of a world motion: its part in the view plane.
    fn shown(motion: Vec3, toward_viewer: Vec3) -> Vec3 {
        motion - toward_viewer * motion.dot(toward_viewer)
    }

    #[test]
    fn free_movement_passes_the_delta_through() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(
            constrain_translation(delta, DragConstraint::Free, Vec3::Z),
            delta
        );
    }

    /// Within the XY plane the scan stays under the cursor, in a view that
    /// shows the plane at a slant as much as in one that shows it square on.
    #[test]
    fn a_plane_constrained_drag_keeps_the_scan_under_the_cursor() {
        let (right, up, toward) = tilted_view();
        for (across, along) in [(1.0, 0.0), (0.0, 1.0), (-0.7, 0.4)] {
            let delta = right * across + up * along;
            let moved = constrain_translation(delta, DragConstraint::XyPlane, toward);
            assert!(moved.z.abs() < 1e-6, "{moved:?} left the plane");
            assert!(
                (shown(moved, toward) - delta).length() < 1e-5,
                "the cursor went {delta:?} and the scan was seen to go {:?}",
                shown(moved, toward)
            );
        }
        let square_on =
            constrain_translation(Vec3::new(1.0, -2.0, 0.0), DragConstraint::XyPlane, Vec3::Z);
        assert_eq!(square_on, Vec3::new(1.0, -2.0, 0.0));
    }

    /// Along the Z axis the scan travels as far on screen, along the axis as
    /// the view shows it, as the cursor did.
    #[test]
    fn an_axis_constrained_drag_keeps_pace_with_the_cursor() {
        let (right, up, toward) = tilted_view();
        let axis_on_screen = shown(Vec3::Z, toward).normalize();
        for (across, along) in [(0.0, 1.0), (0.0, -0.3), (0.8, 0.5)] {
            let delta = right * across + up * along;
            let moved = constrain_translation(delta, DragConstraint::ZOnly, toward);
            assert_eq!((moved.x, moved.y), (0.0, 0.0), "{moved:?} left the axis");
            assert!(
                (shown(moved, toward).dot(axis_on_screen) - delta.dot(axis_on_screen)).abs() < 1e-5,
                "the cursor went {delta:?} and the scan moved {moved:?}"
            );
        }
        // Seen from the front the axis is square on and the drag is one to one.
        let front =
            constrain_translation(Vec3::new(1.0, 0.0, 3.0), DragConstraint::ZOnly, -Vec3::Y);
        assert_eq!(front, Vec3::new(0.0, 0.0, 3.0));
    }

    /// A direction the view cannot show is not dragged at all: making up for
    /// it would move the scan in depth by many times the cursor's travel.
    #[test]
    fn a_constraint_seen_end_on_does_not_move_the_scan_in_depth() {
        // The Z axis pointing at the viewer.
        assert_eq!(
            constrain_translation(Vec3::new(2.0, 3.0, 0.0), DragConstraint::ZOnly, Vec3::Z),
            Vec3::ZERO
        );
        // The XY plane seen edge-on keeps only the line it shares with the
        // view plane.
        assert_eq!(
            constrain_translation(Vec3::new(1.0, 0.0, 3.0), DragConstraint::XyPlane, -Vec3::Y),
            Vec3::new(1.0, 0.0, 0.0)
        );
    }

    #[test]
    fn a_screen_drag_moves_the_scan_the_way_the_pointer_went() {
        // Ten pixels right and four pixels down, at a tenth of a millimetre
        // per pixel: one millimetre along the camera's right axis and 0.4 mm
        // *down*, because screen y grows downward.
        let world = screen_delta_to_world(egui::vec2(10.0, 4.0), Vec3::X, Vec3::Y, 0.1);
        assert!((world.x - 1.0).abs() < 1e-6, "{world:?}");
        assert!((world.y + 0.4).abs() < 1e-6, "{world:?}");
    }

    #[test]
    fn a_zero_or_broken_scale_moves_nothing() {
        assert_eq!(
            screen_delta_to_world(egui::vec2(50.0, 50.0), Vec3::X, Vec3::Y, 0.0),
            Vec3::ZERO
        );
        assert_eq!(
            screen_delta_to_world(egui::vec2(50.0, 50.0), Vec3::X, Vec3::Y, f32::NAN),
            Vec3::ZERO
        );
    }

    #[test]
    fn invalid_or_overflowing_translation_input_cannot_poison_a_layer_pose() {
        for (motion, right, up, scale) in [
            (egui::vec2(f32::NAN, 0.0), Vec3::X, Vec3::Y, 0.1),
            (egui::vec2(0.0, f32::INFINITY), Vec3::X, Vec3::Y, 0.1),
            (egui::vec2(1.0, 0.0), Vec3::splat(f32::NAN), Vec3::Y, 0.1),
            (
                egui::vec2(0.0, 1.0),
                Vec3::X,
                Vec3::splat(f32::INFINITY),
                0.1,
            ),
            (egui::vec2(f32::MAX, 0.0), Vec3::X, Vec3::Y, 2.0),
        ] {
            let delta = screen_delta_to_world(motion, right, up, scale);
            assert_eq!(
                delta,
                Vec3::ZERO,
                "unusable motion must leave the scan still"
            );
            let pose = Affine3A::from_translation(Vec3::new(1.0, 2.0, 3.0));
            assert_eq!(Affine3A::from_translation(delta) * pose, pose);
        }
    }

    /// A Ctrl-drag at rest produces no rotation.
    #[test]
    fn an_empty_drag_produces_no_rotation() {
        let turn = turn_following_grab(Vec3::new(3.0, 4.0, 12.0), Vec3::ZERO, front_view());
        assert!(turn.to_axis_angle().1.abs() < 1e-6);
    }

    /// The ball is continuous where the sphere hands over to the sheet, so a
    /// drag across that line cannot make the scan jump.
    #[test]
    fn the_ball_is_continuous_across_the_sphere_outline() {
        let radius = 20.0;
        let seam = radius * std::f32::consts::FRAC_1_SQRT_2;
        let inside = ball_point(glam::Vec2::new(seam - 1e-3, 0.0), radius);
        let outside = ball_point(glam::Vec2::new(seam + 1e-3, 0.0), radius);
        assert!(
            (inside.z - outside.z).abs() < 1e-2,
            "{inside:?} {outside:?}"
        );
        assert!((ball_point(glam::Vec2::ZERO, radius).z - radius).abs() < 1e-6);
        // Far out the sheet flattens but never reaches the view plane.
        assert!(ball_point(glam::Vec2::new(10.0 * radius, 0.0), radius).z > 0.0);
    }

    /// A point on the ball goes with the cursor: grabbed on the side of the
    /// scan that faces the viewer, it stays under the cursor.
    #[test]
    fn a_point_on_the_ball_follows_the_cursor() {
        for offset in [
            glam::Vec2::ZERO,
            glam::Vec2::new(6.0, -3.0),
            glam::Vec2::new(-9.0, 8.0),
        ] {
            let grabbed = ball_point(offset, 20.0);
            for travel in [
                Vec3::new(1.5, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(-0.8, 1.1, 0.0),
            ] {
                let moved = turn_following_grab(grabbed, travel, front_view()) * grabbed;
                assert!(
                    (moved.truncate() - (grabbed + travel).truncate()).length() < 1e-3,
                    "{grabbed:?} dragged by {travel:?} landed at {moved:?}"
                );
            }
        }
    }

    /// A slow drag is a run of very small steps, and each one has to turn the
    /// scan: a step lost to rounding would make fine seating impossible.
    #[test]
    fn a_hair_of_travel_still_turns_the_scan() {
        let grabbed = Vec3::new(0.0, 0.0, 20.0);
        let travel = Vec3::new(0.002, 0.0, 0.0);
        let turn = turn_following_grab(grabbed, travel, front_view());
        let (axis, angle) = turn.to_axis_angle();
        assert!(axis.dot(Vec3::Y) > 0.999, "{axis:?}");
        assert!((angle - 1.0e-4).abs() < 2.0e-6, "{angle}");
    }

    /// The side facing the viewer goes the way the cursor goes: dragging right
    /// turns the near surface to the right, not the far one. The grabbed
    /// point's own depth does not enter, so a flat scan seen face on tilts
    /// exactly as a deep one does.
    #[test]
    fn the_near_surface_turns_the_way_the_cursor_moves() {
        for depth in [20.0, 0.0, -7.0] {
            let grabbed = Vec3::new(0.0, 0.0, depth);
            let right = turn_following_grab(grabbed, Vec3::new(2.0, 0.0, 0.0), front_view());
            let (axis, angle) = right.to_axis_angle();
            assert!(axis.dot(Vec3::Y) > 0.999, "rightward drag axis: {axis:?}");
            assert!((angle - (2.0_f32 / 20.0).asin()).abs() < 1e-4, "{angle}");

            let down = turn_following_grab(grabbed, Vec3::new(0.0, -2.0, 0.0), front_view());
            assert!(down.to_axis_angle().0.dot(Vec3::X) > 0.999);
        }

        // The same gesture seen through a rolled camera follows that camera.
        let near_pole = Vec3::new(0.0, 0.0, 20.0);
        let rolled = TurnFrame {
            camera_right: Vec3::Y,
            camera_up: -Vec3::X,
            ..front_view()
        };
        let turn = turn_following_grab(near_pole, Vec3::new(0.0, 2.0, 0.0), rolled);
        assert!((turn * near_pole).y > 1.9, "{:?}", turn * near_pole);
    }

    /// Every direction of drag turns the scan, wherever it was grabbed: a
    /// grab at the outline of a flat scan must not leave a direction dead.
    #[test]
    fn no_grab_leaves_a_drag_direction_dead() {
        for grabbed in [
            Vec3::ZERO,
            Vec3::new(14.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::new(0.0, -26.0, 3.0),
        ] {
            for travel in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y] {
                let angle = turn_following_grab(grabbed, travel, front_view())
                    .to_axis_angle()
                    .1;
                assert!(angle > 5e-3, "{grabbed:?} dragged by {travel:?}: {angle}");
            }
        }
    }

    /// Out at the outline a drag along it rolls the scan about the view axis.
    #[test]
    fn a_drag_along_the_outline_rolls_about_the_view_axis() {
        let on_outline = Vec3::new(20.0, 0.0, 0.0);
        let turn = turn_following_grab(on_outline, Vec3::new(0.0, 2.0, 0.0), front_view());
        let (axis, angle) = turn.to_axis_angle();
        assert!(axis.dot(Vec3::Z) > 0.85, "roll axis: {axis:?}");
        assert!(angle > 0.05, "{angle}");
    }

    /// A grab with no usable position turns the scan as a grab at its centre
    /// does, instead of freezing the gesture.
    #[test]
    fn a_grab_without_a_usable_position_turns_as_the_centre_does() {
        let travel = Vec3::new(0.5, 0.0, 0.0);
        let centred = turn_following_grab(Vec3::ZERO, travel, front_view());
        for grabbed in [
            Vec3::splat(f32::NAN),
            Vec3::new(f32::INFINITY, 0.0, 0.0),
            Vec3::new(1.0e9, 0.0, 0.0),
        ] {
            assert_eq!(turn_following_grab(grabbed, travel, front_view()), centred);
        }
        let (axis, angle) = centred.to_axis_angle();
        assert!(axis.dot(Vec3::Y) > 0.999, "{axis:?}");
        assert!((angle - (0.5_f32 / 20.0).asin()).abs() < 1e-4, "{angle}");
    }

    /// A coalesced pointer jump cannot spin the scan by a large angle at once.
    #[test]
    fn a_large_drag_is_capped_at_the_tilt_step() {
        let turn = turn_following_grab(
            Vec3::new(0.0, 0.0, 20.0),
            Vec3::new(0.0, -4000.0, 0.0),
            front_view(),
        );
        assert!(
            (turn.to_axis_angle().1 - MAX_TILT_STEP_RAD).abs() < 1e-5,
            "an extreme drag must clamp to {MAX_TILT_STEP_RAD} rad"
        );
    }

    /// Invalid camera input or scale cannot produce a non-finite turn.
    #[test]
    fn a_degenerate_camera_or_scale_rotates_nothing() {
        let grabbed = Vec3::new(0.0, 0.0, 15.0);
        let travel = Vec3::new(1.0, 1.0, 0.0);
        for frame in [
            TurnFrame {
                toward_viewer: Vec3::ZERO,
                ..front_view()
            },
            TurnFrame {
                camera_right: Vec3::ZERO,
                ..front_view()
            },
            TurnFrame {
                camera_up: Vec3::ZERO,
                ..front_view()
            },
            // A mirrored basis would reverse the gesture.
            TurnFrame {
                toward_viewer: -Vec3::Z,
                ..front_view()
            },
            TurnFrame {
                radius: 0.0,
                ..front_view()
            },
            TurnFrame {
                radius: f32::NAN,
                ..front_view()
            },
            TurnFrame {
                centre: Vec3::splat(f32::NAN),
                ..front_view()
            },
        ] {
            assert_eq!(turn_following_grab(grabbed, travel, frame), Quat::IDENTITY);
        }
        assert_eq!(
            turn_following_grab(grabbed, Vec3::new(f32::NAN, 0.0, 0.0), front_view()),
            Quat::IDENTITY
        );
    }

    /// The pivot stays where it is and everything else goes round it.
    #[test]
    fn a_turn_about_a_pivot_leaves_the_pivot_in_place() {
        let pivot = Vec3::new(2.0, -1.0, 0.5);
        let turn = Quat::from_axis_angle(Vec3::Y, 0.5);
        let step = rotation_about_pivot(turn, pivot);
        assert!((step.transform_point3(pivot) - pivot).length() < 1e-5);
        assert!(
            (step.transform_point3(Vec3::new(-9.0, 3.0, 0.0)) - Vec3::new(-9.0, 3.0, 0.0)).length()
                > 0.1
        );
    }

    #[test]
    fn a_broken_pivot_or_rotation_produces_identity() {
        assert_eq!(
            rotation_about_pivot(Quat::IDENTITY, Vec3::splat(f32::NAN)),
            Affine3A::IDENTITY
        );
        assert_eq!(
            rotation_about_pivot(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0), Vec3::ZERO),
            Affine3A::IDENTITY
        );
    }
}

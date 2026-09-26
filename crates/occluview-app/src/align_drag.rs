//! Moving a scan by hand, in the frame the operator is looking at.
//!
//! The viewport camera is orthographic, so a pixel maps to a fixed number of
//! millimetres regardless of depth: the conversion here is exact, not an
//! approximation that drifts as the operator zooms.

use eframe::egui;
use glam::{Affine3A, Quat, Vec3};

/// Degrees of rotation per pixel of drag. Slow enough that a small correction
/// stays small, fast enough that a half-turn does not need three gestures.
pub(crate) const DEGREES_PER_PIXEL: f32 = 0.35;

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
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Free => "align-constraint-free",
            Self::ZOnly => "align-constraint-z",
            Self::XyPlane => "align-constraint-xy",
        }
    }

    /// The glyph the panel shows.
    pub(crate) fn icon(self) -> crate::icons::AppIcon {
        use crate::icons::AppIcon;
        match self {
            Self::Free => AppIcon::MoveLayer,
            Self::ZOnly => AppIcon::MoveVertical,
            Self::XyPlane => AppIcon::MovePlane,
        }
    }

    /// Catalog key rendering the localized one-line hint.
    pub(crate) fn hint_key(self) -> &'static str {
        match self {
            Self::Free => "align-constraint-free-hint",
            Self::ZOnly => "align-constraint-z-hint",
            Self::XyPlane => "align-constraint-xy-hint",
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

/// Drop the components a constraint forbids.
pub(crate) fn constrain_translation(delta: Vec3, constraint: DragConstraint) -> Vec3 {
    match constraint {
        DragConstraint::Free => delta,
        DragConstraint::ZOnly => Vec3::new(0.0, 0.0, delta.z),
        DragConstraint::XyPlane => Vec3::new(delta.x, delta.y, 0.0),
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
    if !world_per_pixel.is_finite() || world_per_pixel <= 0.0 {
        return Vec3::ZERO;
    }
    camera_right * (delta_px.x * world_per_pixel) - camera_up * (delta_px.y * world_per_pixel)
}

/// Convert a screen drag into a rotation about the camera's own axes.
///
/// Horizontal drag turns about the camera's up axis and vertical drag about
/// its right axis, which is what makes the scan appear to follow the pointer
/// rather than spinning about some world axis the operator cannot see.
pub(crate) fn rotation_from_drag(
    delta_px: egui::Vec2,
    camera_right: Vec3,
    camera_up: Vec3,
    degrees_per_pixel: f32,
) -> Quat {
    let yaw = (delta_px.x * degrees_per_pixel).to_radians();
    let pitch = (delta_px.y * degrees_per_pixel).to_radians();
    let up = camera_up.normalize_or_zero();
    let right = camera_right.normalize_or_zero();
    if up.length_squared() <= 0.0 || right.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    (Quat::from_axis_angle(up, yaw) * Quat::from_axis_angle(right, pitch)).normalize()
}

/// Turn a screen drag into a rotation the chosen constraint allows.
///
/// The chips are labelled for movement — "Move in z-direction", "Move in
/// xy-plane" — and only Free says "Move/rotate in all directions". A Ctrl+drag
/// therefore obeys the selected chip, so the scan follows the restriction the
/// panel shows.
///
/// Both restricted modes turn about world **Z**, and in a dental scene that is
/// the rotation an operator needs: an arch spun about the vertical while it
/// stays seated. Horizontal drag only, because a vertical drag
/// under a Z-only rotation has nothing left to mean.
pub(crate) fn constrained_rotation_from_drag(
    delta_px: egui::Vec2,
    camera_right: Vec3,
    camera_up: Vec3,
    degrees_per_pixel: f32,
    constraint: DragConstraint,
) -> Quat {
    match constraint {
        DragConstraint::Free => {
            rotation_from_drag(delta_px, camera_right, camera_up, degrees_per_pixel)
        }
        DragConstraint::ZOnly | DragConstraint::XyPlane => {
            Quat::from_axis_angle(Vec3::Z, (delta_px.x * degrees_per_pixel).to_radians())
        }
    }
}

/// Turn a scan about the point the operator grabbed, not about its centre.
///
/// A hand drag that rotates about the mesh centre spins the whole arch around a
/// pivot the operator cannot see and did not choose: they pull a cusp and the
/// far side swings, which reads as the tool ignoring where the pointer went.
/// Pivoting about the grabbed surface point keeps that point under the cursor
/// for the whole gesture, so the scan turns about what was actually pulled.
///
/// The returned step is a world-space transform, meant to be pre-multiplied onto
/// the layer's pose exactly like the translation step.
///
/// A non-finite pivot yields the identity: it is unreachable from the drag
/// handler, because `drag_pivot_local` has already replaced an unusable grab
/// with the layer centre, and guessing a pivot here would turn the scan about a
/// point the operator did not choose.
pub(crate) fn rotation_about_pivot(turn: Quat, pivot: Vec3) -> Affine3A {
    if !pivot.is_finite() {
        return Affine3A::IDENTITY;
    }
    Affine3A::from_translation(pivot)
        * Affine3A::from_quat(turn)
        * Affine3A::from_translation(-pivot)
}

/// How far a grabbed pivot may sit from the layer centre, as a multiple of the
/// layer's own radius.
///
/// The bound is relative on purpose. An absolute metre says nothing about a
/// 70 mm arch — it passes a pivot fourteen times the whole scan — and it is
/// anchored to the world origin, so a layer legitimately placed a metre away
/// would have every Ctrl-drag refused. A grab further than this multiple of
/// the scan's own size is a pose artefact, not a point on the surface.
pub(crate) const DRAG_PIVOT_EXTENT_MULTIPLE: f32 = 10.0;

/// Floor for the relative bound, so a degenerate bounding box cannot reject
/// every grab.
pub(crate) const MIN_PIVOT_EXTENT_MM: f32 = 10.0;

/// Which local point a Ctrl-drag fixes.
///
/// Always the surface point the operator grabbed. The gesture exists to answer
/// "where am I pulling, and by what": that point must stay under the cursor
/// while the scan turns around it. The drag constraint chooses the rotation
/// *axis*, never the pivot — a cusp pulled under any chip must not slide
/// sideways.
///
/// The trade-off is worth stating, because it is visible: a rotation about a
/// point that is not the centre necessarily moves the centre. Under `ZOnly` and
/// `XyPlane` the turn is still about world Z, but the arch's centre travels in
/// XY by roughly `2*sin(angle/2)*offset`. Keeping the centre fixed instead would
/// put the turn on an axis the operator did not choose, so the grabbed point
/// wins. The constraint chips
/// describe *translation* ("Move in z-direction"); the Ctrl gesture is a turn,
/// and the panel says the turn follows the grab.
///
/// The layer centre is the fallback for a grab that cannot be trusted: a
/// non-finite value out of a singular pose, or a point far outside the scan.
/// Returning a sane point keeps the gesture alive instead of freezing it.
pub(crate) fn drag_pivot_local(grabbed_local: Vec3, centre_local: Vec3, radius_local: f32) -> Vec3 {
    let limit = (radius_local * DRAG_PIVOT_EXTENT_MULTIPLE).max(MIN_PIVOT_EXTENT_MM);
    let offset = (grabbed_local - centre_local).length();
    if grabbed_local.is_finite() && offset.is_finite() && offset <= limit {
        grabbed_local
    } else {
        centre_local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    use super::{
        constrain_translation, mm_per_pixel, rotation_from_drag, screen_delta_to_world,
        DragConstraint,
    };
    use eframe::egui;
    use glam::{Quat, Vec3};

    #[test]
    fn free_movement_passes_the_delta_through() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(constrain_translation(delta, DragConstraint::Free), delta);
    }

    #[test]
    fn z_only_keeps_the_vertical_component() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(
            constrain_translation(delta, DragConstraint::ZOnly),
            Vec3::new(0.0, 0.0, 3.0)
        );
    }

    #[test]
    fn the_xy_plane_drops_the_vertical_component() {
        let delta = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(
            constrain_translation(delta, DragConstraint::XyPlane),
            Vec3::new(1.0, -2.0, 0.0)
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
    fn an_empty_drag_produces_no_rotation() {
        let rotation = rotation_from_drag(egui::Vec2::ZERO, Vec3::X, Vec3::Y, 0.5);
        assert!(rotation.to_axis_angle().1.abs() < 1e-6);
    }

    #[test]
    fn a_horizontal_drag_turns_about_the_camera_up_axis() {
        let rotation = rotation_from_drag(egui::vec2(90.0, 0.0), Vec3::X, Vec3::Y, 1.0);
        let (axis, angle) = rotation.to_axis_angle();
        assert!(axis.dot(Vec3::Y).abs() > 0.99, "axis was {axis:?}");
        assert!((angle.to_degrees() - 90.0).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn a_vertical_drag_turns_about_the_camera_right_axis() {
        let rotation = rotation_from_drag(egui::vec2(0.0, 45.0), Vec3::X, Vec3::Y, 1.0);
        let (axis, angle) = rotation.to_axis_angle();
        assert!(axis.dot(Vec3::X).abs() > 0.99, "axis was {axis:?}");
        assert!((angle.to_degrees() - 45.0).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn a_rotation_is_always_a_unit_quaternion() {
        let rotation = rotation_from_drag(egui::vec2(37.0, -21.0), Vec3::X, Vec3::Y, 0.4);
        assert!((rotation.length() - 1.0).abs() < 1e-5);
        assert_ne!(rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_degenerate_camera_basis_rotates_nothing() {
        let rotation = rotation_from_drag(egui::vec2(30.0, 30.0), Vec3::ZERO, Vec3::Y, 1.0);
        assert_eq!(rotation, Quat::IDENTITY);
    }

    /// A Ctrl-drag must turn about the grabbed point, not the mesh centre.
    ///
    /// The operator pulls a cusp to tilt an arch. Pivoting about the centre
    /// swings the far side and leaves the grabbed surface sliding sideways,
    /// which reads as the tool ignoring where the pointer went; pivoting about
    /// the grab keeps the grabbed point exactly where the pointer is.
    #[test]
    fn a_ctrl_drag_turns_about_the_grabbed_point() {
        let pivot = Vec3::new(4.0, -2.0, 1.5);
        let turn = Quat::from_axis_angle(Vec3::Y, 0.5);
        let step = rotation_about_pivot(turn, pivot);

        // The pivot itself is a fixed point of the turn.
        let pinned = step.transform_point3(pivot);
        assert!(
            (pinned - pivot).length() < 1e-5,
            "the grabbed point moved: {pinned:?}"
        );

        // A point away from the pivot does move, and by the rotation about it.
        let far = Vec3::new(-9.0, 3.0, 0.0);
        let expected = pivot + turn * (far - pivot);
        let actual = step.transform_point3(far);
        assert!(
            (actual - expected).length() < 1e-5,
            "expected {expected:?}, got {actual:?}"
        );

        // And it is not the centre pivot: the world origin is not pinned.
        let origin_moved = step.transform_point3(Vec3::ZERO).length();
        assert!(
            origin_moved > 1e-3,
            "the step collapsed onto a centre pivot (origin unmoved)"
        );
    }

    /// A non-finite pivot never produces a non-finite transform.
    #[test]
    fn a_non_finite_pivot_still_returns_a_rotation() {
        let step = rotation_about_pivot(
            Quat::from_axis_angle(Vec3::Y, 0.25),
            Vec3::new(f32::NAN, 0.0, 0.0),
        );
        assert!(step.is_finite(), "{step:?}");
        // It must not guess a pivot: the identity leaves the pose alone.
        assert_eq!(step, Affine3A::IDENTITY);
    }

    /// The price of pivoting on the grab: an off-centre turn moves the centre.
    ///
    /// This is the documented trade-off, asserted so it cannot change unnoticed.
    /// Pivoting on the grabbed point keeps the pulled surface under the cursor,
    /// and a Z-axis turn about a point that is not the centre necessarily
    /// carries the centre in XY. This pins that consequence rather than
    /// forbidding it.
    #[test]
    fn an_off_centre_turn_moves_the_centre_within_the_plane() {
        let centre = Vec3::ZERO;
        let grabbed = Vec3::new(30.0, 0.0, 0.0);
        // 40 px at DEGREES_PER_PIXEL, the app's own Z-only turn.
        let turn = constrained_rotation_from_drag(
            egui::vec2(40.0, 0.0),
            Vec3::X,
            Vec3::Y,
            DEGREES_PER_PIXEL,
            DragConstraint::ZOnly,
        );
        let step = rotation_about_pivot(turn, grabbed);

        // The grabbed point is exactly fixed.
        let pinned = step.transform_point3(grabbed);
        assert!(
            (pinned - grabbed).length() < 1e-3,
            "the grabbed point must stay put, got {pinned:?}"
        );

        // The centre moves, by the closed form 2*sin(angle/2)*offset.
        let moved = (step.transform_point3(centre) - centre).length();
        let (_, angle) = turn.to_axis_angle();
        let expected = 2.0 * (angle / 2.0).sin() * 30.0;
        assert!(
            (moved - expected).abs() < 1e-2,
            "centre moved {moved:.4} mm, expected {expected:.4} mm"
        );
        // And the motion stays in the plane: a Z-turn adds no Z displacement.
        let full = step.transform_point3(centre);
        assert!(
            full.z.abs() < 1e-4,
            "a Z-only turn must not lift the centre, got {full:?}"
        );
    }

    /// The grabbed point is the pivot, under every drag constraint.
    ///
    /// A cusp pulled under any chip must stay under the cursor. The constraint
    /// chooses the rotation axis; letting it also choose the pivot would make a
    /// constrained Ctrl-drag spin around an axis with the pulled point sliding
    /// away.
    ///
    /// The axis must still follow the constraint, which is what keeps this from
    /// passing vacuously on a build where the constraint was dropped entirely:
    /// each case builds its turn the way the app does and checks both the fixed
    /// point and the axis.
    #[test]
    fn the_grabbed_point_is_the_pivot_under_every_constraint() {
        let grabbed = Vec3::new(12.0, -5.0, 3.0);
        let centre = Vec3::new(1.0, 2.0, 0.0);
        let radius = 35.0;
        let drag = egui::vec2(40.0, 0.0);

        for (constraint, expect_z_axis) in [
            (DragConstraint::Free, false),
            (DragConstraint::ZOnly, true),
            (DragConstraint::XyPlane, true),
        ] {
            // The app's own composition: the constraint picks the turn, the
            // grabbed point is always the pivot.
            let turn = constrained_rotation_from_drag(drag, Vec3::X, Vec3::Y, 0.25, constraint);
            let pivot = drag_pivot_local(grabbed, centre, radius);
            let step = rotation_about_pivot(turn, pivot);

            assert_eq!(
                pivot, grabbed,
                "{constraint:?}: the constraint must not move the pivot"
            );
            let pinned = step.transform_point3(grabbed);
            assert!(
                (pinned - grabbed).length() < 1e-4,
                "{constraint:?}: the grabbed point moved to {pinned:?}"
            );

            // The constrained chips must still turn about the vertical.
            let (axis, angle) = turn.to_axis_angle();
            if expect_z_axis {
                assert!(
                    axis.dot(Vec3::Z).abs() > 0.99,
                    "{constraint:?}: expected a Z-axis turn, got axis {axis:?}"
                );
                assert!(angle.abs() > 1e-3, "{constraint:?}: the turn was empty");
            } else {
                assert!(
                    axis.dot(Vec3::Z).abs() < 0.99,
                    "{constraint:?}: a free turn should not be locked to Z"
                );
            }
        }
    }

    /// An unusable grab falls back to the centre instead of freezing the drag.
    #[test]
    fn an_unusable_pivot_falls_back_to_the_layer_centre() {
        let centre = Vec3::new(1.0, 2.0, 0.0);
        let radius = 35.0;
        let limit = (radius * DRAG_PIVOT_EXTENT_MULTIPLE).max(MIN_PIVOT_EXTENT_MM);

        for bad in [
            Vec3::new(f32::NAN, 0.0, 0.0),
            Vec3::new(f32::INFINITY, 0.0, 0.0),
            // Just outside the layer's own extent bound.
            centre + Vec3::splat(limit + 1.0),
        ] {
            assert_eq!(
                drag_pivot_local(bad, centre, radius),
                centre,
                "a grab of {bad:?} must fall back to the centre"
            );
        }

        // The bound is relative to the scan, so a grab a sane distance out is
        // still honoured: this is what keeps a far-placed layer draggable.
        assert_eq!(
            drag_pivot_local(centre + Vec3::splat(limit * 0.5), centre, radius),
            centre + Vec3::splat(limit * 0.5),
            "a grab inside the layer's extent must be honoured"
        );
    }

    /// A tiny or degenerate bounding box still leaves a usable pivot.
    #[test]
    fn a_degenerate_layer_extent_still_allows_a_grab() {
        let centre = Vec3::ZERO;
        // A zero radius would make a purely relative bound reject everything.
        assert_eq!(
            drag_pivot_local(Vec3::new(1.0, 0.0, 0.0), centre, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            "the floor must keep a small scan draggable"
        );
    }
}

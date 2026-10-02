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

/// Camera and mesh scale for one anchored Ctrl-drag turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnchoredRotationFrame {
    camera_view_direction: Vec3,
    camera_right: Vec3,
    camera_up: Vec3,
    world_per_pixel: f32,
    radius_world: f32,
}

impl AnchoredRotationFrame {
    pub(crate) fn new(
        camera_view_direction: Vec3,
        camera_right: Vec3,
        camera_up: Vec3,
        world_per_pixel: f32,
        radius_world: f32,
    ) -> Self {
        Self {
            camera_view_direction,
            camera_right,
            camera_up,
            world_per_pixel,
            radius_world,
        }
    }
}

/// Turn a Ctrl-drag around the world point the operator grabbed.
///
/// The turn axis is `view_direction × screen_delta_to_world`, preserving the
/// camera basis and screen-y direction. Its angle scales with screen travel in
/// millimetres relative to the layer radius, so a similar gesture has
/// comparable effect across zoom levels and scan sizes. The clicked point
/// stays fixed; only the turn's orientation follows the camera basis.
pub(crate) fn anchored_rotation_from_drag(
    delta_px: egui::Vec2,
    frame: AnchoredRotationFrame,
) -> Quat {
    let AnchoredRotationFrame {
        camera_view_direction,
        camera_right,
        camera_up,
        world_per_pixel,
        radius_world,
    } = frame;
    if !delta_px.x.is_finite()
        || !delta_px.y.is_finite()
        || !world_per_pixel.is_finite()
        || world_per_pixel <= 0.0
        || !radius_world.is_finite()
        || radius_world <= 1e-3
        || !camera_view_direction.is_finite()
        || !camera_right.is_finite()
        || !camera_up.is_finite()
    {
        return Quat::IDENTITY;
    }
    let view = camera_view_direction.normalize_or_zero();
    let right = camera_right.normalize_or_zero();
    let up = camera_up.normalize_or_zero();
    // A partial or skewed camera basis would silently turn a two-axis gesture
    // into one-axis motion or change its polarity. Camera supplies an
    // orthonormal basis with `right = view × up`.
    let handedness = view.cross(right).dot(up);
    if view.length_squared() <= f32::EPSILON
        || right.length_squared() <= f32::EPSILON
        || up.length_squared() <= f32::EPSILON
        || !handedness.is_finite()
        || handedness > -0.99
    {
        return Quat::IDENTITY;
    }
    let desired = screen_delta_to_world(delta_px, right, up, world_per_pixel);
    let axis = view.cross(desired);
    let axis_length = axis.length();
    let distance_world = desired.length();
    if !view.is_finite()
        || view.length_squared() <= f32::EPSILON
        || !axis_length.is_finite()
        || axis_length <= 1e-6
        || !distance_world.is_finite()
    {
        return Quat::IDENTITY;
    }
    let angle = (distance_world / radius_world).min(MAX_TILT_STEP_RAD);
    Quat::from_axis_angle(axis / axis_length, angle)
}

/// Turn a scan around a world-space anchor.
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

/// Which local point a Ctrl-drag uses as its fixed world-space anchor.
///
/// The surface point the operator grabbed, validated against the layer's own
/// size. Invalid or distant points fall back to the bounds centre.
///
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

    fn rotation_frame(
        view: Vec3,
        right: Vec3,
        up: Vec3,
        world_per_pixel: f32,
        radius: f32,
    ) -> AnchoredRotationFrame {
        AnchoredRotationFrame::new(view, right, up, world_per_pixel, radius)
    }

    fn rotation(delta: egui::Vec2, frame: AnchoredRotationFrame) -> Quat {
        anchored_rotation_from_drag(delta, frame)
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

    use super::{
        anchored_rotation_from_drag, constrain_translation, drag_pivot_local, mm_per_pixel,
        rotation_about_pivot, screen_delta_to_world, DragConstraint, DRAG_PIVOT_EXTENT_MULTIPLE,
        MAX_TILT_STEP_RAD, MIN_PIVOT_EXTENT_MM,
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
        let rotation = rotation(
            egui::Vec2::ZERO,
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 0.5, 10.0),
        );
        assert!(rotation.to_axis_angle().1.abs() < 1e-6);
    }

    /// Camera orientation chooses the axis, while object radius chooses the
    /// angular amount for the same screen-space movement.
    #[test]
    fn an_anchored_turn_uses_camera_axes_and_relative_scan_scale() {
        let turn = rotation(
            egui::vec2(8.0, 0.0),
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 0.25, 20.0),
        );
        let (axis, angle) = turn.to_axis_angle();
        assert!(axis.dot(-Vec3::Y) > 0.999, "horizontal drag axis: {axis:?}");
        assert!((angle - 0.1).abs() < 1e-5, "relative angle: {angle}");

        let vertical = rotation(
            egui::vec2(0.0, 8.0),
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 0.25, 20.0),
        );
        let (vertical_axis, vertical_angle) = vertical.to_axis_angle();
        assert!(
            vertical_axis.dot(-Vec3::X) > 0.999,
            "downward drag axis: {vertical_axis:?}"
        );
        assert!((vertical_angle - angle).abs() < 1e-5);

        let turned_view = rotation(
            egui::vec2(8.0, 0.0),
            rotation_frame(-Vec3::Z, Vec3::Y, -Vec3::X, 0.25, 20.0),
        );
        let (view_axis, view_angle) = turned_view.to_axis_angle();
        assert!(
            view_axis.dot(Vec3::X) > 0.999,
            "rotated view axis: {view_axis:?}"
        );
        assert!((view_angle - angle).abs() < 1e-5);

        let larger_scan = rotation(
            egui::vec2(8.0, 0.0),
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 0.25, 40.0),
        );
        assert!((larger_scan.to_axis_angle().1 - angle * 0.5).abs() < 1e-5);
    }

    /// A coalesced pointer jump cannot spin the scan by a large angle at once.
    #[test]
    fn a_large_drag_is_capped_at_the_tilt_step() {
        let turn = rotation(
            egui::vec2(0.0, -4000.0),
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 1.0, 10.0),
        );
        assert!(
            (turn.to_axis_angle().1 - MAX_TILT_STEP_RAD).abs() < 1e-5,
            "an extreme drag must clamp to {MAX_TILT_STEP_RAD} rad"
        );
    }

    /// Invalid camera input or scale cannot produce a non-finite turn.
    #[test]
    fn a_degenerate_camera_or_scale_rotates_nothing() {
        for (view, right, up, world_per_pixel, radius) in [
            (Vec3::ZERO, Vec3::X, Vec3::Y, 1.0, 10.0),
            (-Vec3::Z, Vec3::ZERO, Vec3::Y, 1.0, 10.0),
            (-Vec3::Z, Vec3::X, Vec3::ZERO, 1.0, 10.0),
            (-Vec3::Z, Vec3::X, Vec3::Y, 0.0, 10.0),
            (-Vec3::Z, Vec3::X, Vec3::Y, f32::NAN, 10.0),
            (-Vec3::Z, Vec3::X, Vec3::Y, 1.0, 0.0),
            (-Vec3::Z, Vec3::X, Vec3::Y, 1.0, f32::NAN),
        ] {
            let turn = rotation(
                egui::vec2(30.0, 30.0),
                rotation_frame(view, right, up, world_per_pixel, radius),
            );
            assert_eq!(turn, Quat::IDENTITY);
        }
        assert_eq!(
            rotation(
                egui::vec2(f32::NAN, 0.0),
                rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 1.0, 10.0),
            ),
            Quat::IDENTITY
        );
    }

    /// The point the operator grabbed stays fixed through the turn.
    #[test]
    fn a_ctrl_drag_turns_about_the_grabbed_point() {
        let anchor = Vec3::new(2.0, -1.0, 0.5);
        let turn = Quat::from_axis_angle(Vec3::Y, 0.5);
        let step = rotation_about_pivot(turn, anchor);
        assert!((step.transform_point3(anchor) - anchor).length() < 1e-5);
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

    /// Constraint chips affect plain translation and leave the rotation law
    /// independent, because there is no constraint input to the turn.
    #[test]
    fn the_ctrl_turn_is_independent_of_translation_constraint() {
        let turn = rotation(
            egui::vec2(30.0, -18.0),
            rotation_frame(-Vec3::Z, Vec3::X, Vec3::Y, 0.2, 20.0),
        );
        for constraint in [
            DragConstraint::Free,
            DragConstraint::ZOnly,
            DragConstraint::XyPlane,
        ] {
            assert!(turn.to_axis_angle().1 > 1e-3, "{constraint:?}");
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

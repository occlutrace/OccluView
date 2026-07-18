//! Stateless geometry for the cut disc: follow orientation, normal smoothing,
//! radius scaling, the handle hit-test, and the translate / push-pull / arcball
//! transforms. Split out of [`crate::cut_manipulator`] so both the pure math and
//! the state machine stay well under the file-size budget; every function here
//! is a pure function of its inputs and unit-tested below.

use crate::cut_manipulator::{
    ArchFrame, CutCursor, CutFrameInput, DiscDrag, DiscPose, CENTER_GRAB_RADIUS_PX,
    MAX_DISC_RADIUS_MM, MIN_DISC_RADIUS_MM, RADIUS_WHEEL_STEP, RIM_GRAB_RADIUS_PX,
};
use eframe::egui::Pos2;
use glam::{Quat, Vec3};

/// Blend range from the camera-aligned axial fallback into the surface-driven
/// orientation (the local-normal fallback path only). A range, rather than one
/// hard threshold, prevents a tiny cursor move across adjacent facets from
/// snapping the disc by 90 degrees.
const FOLLOW_BLEND_START: f32 = 0.015;
const FOLLOW_BLEND_END: f32 = 0.12;

/// Follow orientation: the disc plane contains BOTH a surface-direction `n`
/// and the current view direction, so its normal is `n x view_dir` — this is
/// what keeps the disc edge-on and legible from the camera's current angle,
/// rotating naturally as the operator orbits to aim it (exactly like a
/// physical blade held up to the view), the way it always did before this
/// session's fixes.
///
/// `n` prefers the LOCAL arch direction derived from `arch_frame` — a mesh's
/// own PCA centroid and its two greatest-variance axes (see
/// [`occluview_core::Mesh::principal_frame_cached`]): the vector from
/// `centroid` to `point`, projected onto the `axis0`/`axis1` plane and
/// normalized (see [`local_arch_normal`]). A disc plane containing this LOCAL
/// direction cuts TRANSVERSE to the arch/span at that point: the anatomically
/// useful orientation for viewing occlusal contacts, and the one a Bridge
/// Split separator needs to divide a span into segments. Unlike a single
/// constant axis for the whole mesh, this rotates smoothly as `point` moves
/// around a curved dental arch — reducing to (roughly) `axis0` at the arch's
/// left/right extremes, where a constant axis happens to already be correct,
/// and adapting continuously in between. Because it is derived from a
/// per-mesh-constant frame rather than the hit triangle, it is still immune
/// to the per-triangle jitter the raw triangle normal had.
///
/// Falls back to that raw (noisy) triangle normal only when no arch frame is
/// available (a point cloud, or too few vertices for a well-defined frame),
/// or `point` projects too close to the centroid to define a direction
/// (never happens for a point on a real arch's surface; only a defensive
/// guard) — falling back further to the camera-right axis on a degenerate
/// (view staring straight down `n`) cross product.
pub(crate) fn follow_plane_normal(
    arch_frame: Option<ArchFrame>,
    point: Vec3,
    surface_normal: Vec3,
    view_dir: Vec3,
    camera_right: Vec3,
) -> Vec3 {
    let fallback = camera_right.normalize_or(Vec3::X);
    let n = arch_frame
        .and_then(|frame| local_arch_normal(frame, point))
        .unwrap_or_else(|| surface_normal.normalize_or_zero());
    let v = view_dir.normalize_or_zero();
    let cross = n.cross(v);
    let length = cross.length();
    if length <= f32::EPSILON {
        return fallback;
    }
    let surface_driven = cross / length;
    let oriented_fallback = if surface_driven.dot(fallback) < 0.0 {
        -fallback
    } else {
        fallback
    };
    let linear =
        ((length - FOLLOW_BLEND_START) / (FOLLOW_BLEND_END - FOLLOW_BLEND_START)).clamp(0.0, 1.0);
    let blend = linear * linear * (3.0 - 2.0 * linear);
    oriented_fallback
        .lerp(surface_driven, blend)
        .normalize_or(oriented_fallback)
}

/// The LOCAL cross-arch direction at `point`: the vector from `frame`'s own
/// PCA centroid to `point`, projected onto `frame`'s `axis0`/`axis1` plane
/// and normalized — the "spoke" direction pointing radially outward from the
/// arch's own center through this point. For a horseshoe/U-shaped dental
/// arch this smoothly ROTATES as `point` moves around the curve: it reduces
/// to (roughly) `axis0` at the arch's left/right extremes and to `axis1`
/// near its front-center, tracking the true local cross-arch direction
/// everywhere between — unlike a single constant axis, which is only correct
/// at those extremes. `None` when the projection collapses to (near) zero,
/// i.e. `point` sits (almost) exactly at the centroid — never the case for a
/// point on a real mesh's surface, since the centroid lies inside the solid;
/// only a defensive guard against a degenerate frame/point pairing.
fn local_arch_normal(frame: ArchFrame, point: Vec3) -> Option<Vec3> {
    let offset = point - frame.centroid;
    let in_plane = frame.axis0 * offset.dot(frame.axis0) + frame.axis1 * offset.dot(frame.axis1);
    let normalized = in_plane.normalize_or_zero();
    (normalized.length_squared() > f32::EPSILON).then_some(normalized)
}

/// Exponential temporal smoothing toward the fresh normal, killing scanner
/// jitter. Falls back to the raw sample if the blend collapses (near-opposite
/// normals) or there is no prior state.
pub(crate) fn smooth_normal(prev: Option<Vec3>, raw: Vec3, blend: f32) -> Vec3 {
    let mut raw = raw.normalize_or_zero();
    match prev {
        Some(prev) if prev.length_squared() > f32::EPSILON => {
            let prev = prev.normalize_or(raw);
            if prev.dot(raw) < 0.0 {
                raw = -raw;
            }
            prev.lerp(raw, blend).normalize_or(prev)
        }
        _ => raw,
    }
}

/// Whether the camera eye lies on the `+plane_normal` side of the disc.
pub(crate) fn camera_keep_side(pose: &DiscPose, eye: Vec3) -> bool {
    pose.plane_normal.dot(eye - pose.center) >= 0.0
}

/// Clamp the wheel-scaled radius to the allowed range.
pub(crate) fn scale_radius(radius: f32, notches: f32) -> f32 {
    (radius * RADIUS_WHEEL_STEP.powf(notches)).clamp(MIN_DISC_RADIUS_MM, MAX_DISC_RADIUS_MM)
}

/// Classify a primary press on a planted disc into a drag. Ctrl anywhere on the
/// disc tilts it; an unmodified press anywhere on its body translates it. The
/// narrow halo immediately outside the rim retains the depth push/pull gesture.
pub(crate) fn begin_drag(pose: &DiscPose, input: &CutFrameInput) -> Option<DiscDrag> {
    let center = input.disc_center_screen?;
    let pointer = input.pointer?;
    let distance = (pointer - center).length();
    let radius = input.disc_radius_screen.max(0.0);
    if input.ctrl {
        if distance <= radius + RIM_GRAB_RADIUS_PX {
            return Some(DiscDrag::Tilt {
                normal0: pose.plane_normal,
                pointer0: pointer,
            });
        }
        return None;
    }
    if distance <= radius.max(CENTER_GRAB_RADIUS_PX) {
        return Some(DiscDrag::Translate {
            center0: pose.center,
            ray_origin0: input.ray_origin,
        });
    }
    if distance <= radius + RIM_GRAB_RADIUS_PX {
        return Some(DiscDrag::PushPull {
            center0: pose.center,
            ray_origin0: input.ray_origin,
        });
    }
    None
}

/// The cursor a planted-but-idle disc should show given the hover position.
pub(crate) fn hover_cursor(pose: &DiscPose, input: &CutFrameInput) -> CutCursor {
    let probe = CutFrameInput {
        ctrl: false,
        ..*input
    };
    if begin_drag(pose, &probe).is_some() || (input.ctrl && begin_drag(pose, input).is_some()) {
        CutCursor::Grab
    } else {
        CutCursor::Default
    }
}

/// Apply a drag to the pose in place.
pub(crate) fn apply_drag(pose: &mut DiscPose, drag: DiscDrag, input: &CutFrameInput) {
    match drag {
        DiscDrag::Translate {
            center0,
            ray_origin0,
        } => {
            pose.center =
                translate_in_plane(center0, ray_origin0, input.ray_origin, input.view_dir);
        }
        DiscDrag::PushPull {
            center0,
            ray_origin0,
        } => {
            pose.center = push_pull(center0, pose.plane_normal, ray_origin0, input.ray_origin);
        }
        DiscDrag::Tilt { normal0, pointer0 } => {
            if let (Some(pointer), Some(center)) = (input.pointer, input.disc_center_screen) {
                let rotation = arcball_rotation(
                    center,
                    pointer0,
                    pointer,
                    input.disc_radius_screen.max(1.0),
                    input.camera_right,
                    input.camera_up,
                    input.view_dir,
                );
                pose.plane_normal = (rotation * normal0).normalize_or(normal0);
            }
        }
    }
}

/// Center-handle translate: move the disc within the screen plane by the change
/// in the pointer's world-ray origin (the view-direction component removed so
/// the orientation and the along-normal offset stay put).
pub(crate) fn translate_in_plane(
    center0: Vec3,
    ray_origin0: Vec3,
    ray_origin_now: Vec3,
    view_dir: Vec3,
) -> Vec3 {
    let delta = ray_origin_now - ray_origin0;
    let view = view_dir.normalize_or_zero();
    let planar = delta - view * delta.dot(view);
    center0 + planar
}

/// Rim-handle push/pull: slide the disc center along its plane normal by the
/// pointer motion projected onto that normal.
pub(crate) fn push_pull(
    center0: Vec3,
    plane_normal: Vec3,
    ray_origin0: Vec3,
    ray_origin_now: Vec3,
) -> Vec3 {
    let normal = plane_normal.normalize_or_zero();
    let along = (ray_origin_now - ray_origin0).dot(normal);
    center0 + normal * along
}

/// Screen-space arcball: map the press and current pointer to points on a
/// virtual sphere (radius = disc screen radius) centered on the disc, and
/// return the rotation carrying the first to the second.
#[allow(clippy::too_many_arguments)]
pub(crate) fn arcball_rotation(
    center: Pos2,
    pointer0: Pos2,
    pointer1: Pos2,
    radius_px: f32,
    camera_right: Vec3,
    camera_up: Vec3,
    view_dir: Vec3,
) -> Quat {
    let s0 = arcball_sphere_vec(
        center,
        pointer0,
        radius_px,
        camera_right,
        camera_up,
        view_dir,
    );
    let s1 = arcball_sphere_vec(
        center,
        pointer1,
        radius_px,
        camera_right,
        camera_up,
        view_dir,
    );
    if s0.length_squared() <= f32::EPSILON || s1.length_squared() <= f32::EPSILON {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(s0.normalize(), s1.normalize())
}

/// One arcball sphere vector in world space. The out-of-screen axis points
/// toward the camera (`-view_dir`).
#[allow(clippy::too_many_arguments)]
fn arcball_sphere_vec(
    center: Pos2,
    pointer: Pos2,
    radius_px: f32,
    camera_right: Vec3,
    camera_up: Vec3,
    view_dir: Vec3,
) -> Vec3 {
    let dx = (pointer.x - center.x) / radius_px;
    let dy = -(pointer.y - center.y) / radius_px;
    let planar_sq = dx * dx + dy * dy;
    let (sx, sy, sz) = if planar_sq <= 1.0 {
        (dx, dy, (1.0 - planar_sq).sqrt())
    } else {
        let inv = 1.0 / planar_sq.sqrt();
        (dx * inv, dy * inv, 0.0)
    };
    camera_right.normalize_or_zero() * sx + camera_up.normalize_or_zero() * sy
        - view_dir.normalize_or_zero() * sz
}

/// Closest parameter `t ∈ [0, 1]` along segment `a → b` to point `p`, plus the
/// squared pixel distance from `p` to that closest point. A degenerate segment
/// (`a == b`) yields `t = 0`. All inputs are 2D panel pixels.
pub(crate) fn closest_param_on_segment(point: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let ab = b - a;
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    let t = if len_sq <= f32::EPSILON {
        0.0
    } else {
        let ap = point - a;
        ((ap.x * ab.x + ap.y * ab.y) / len_sq).clamp(0.0, 1.0)
    };
    let closest = a + ab * t;
    let diff = point - closest;
    (t, diff.x * diff.x + diff.y * diff.y)
}

/// Magnet-snap a panel-space `click` to the nearest point on any world-space
/// contour segment within `radius_px` **panel pixels**. `project` maps a world
/// point to its panel pixel (the same mapping the ruler and the drawn contour
/// use), so the radius is a true on-screen distance and naturally tightens as
/// the view zooms in. Returns the EXACT segment-interpolated world point (not
/// just the nearest vertex), or `None` when no segment is within the radius.
pub(crate) fn snap_to_contour<I>(
    click: Pos2,
    segments: I,
    project: impl Fn(Vec3) -> Pos2,
    radius_px: f32,
) -> Option<Vec3>
where
    I: IntoIterator<Item = (Vec3, Vec3)>,
{
    let radius_sq = radius_px * radius_px;
    let mut best: Option<(f32, Vec3)> = None;
    for (world_a, world_b) in segments {
        let (t, dist_sq) = closest_param_on_segment(click, project(world_a), project(world_b));
        if dist_sq <= radius_sq && best.is_none_or(|(best_dist, _)| dist_sq < best_dist) {
            best = Some((dist_sq, world_a.lerp(world_b, t)));
        }
    }
    best.map(|(_, world)| world)
}

#[cfg(test)]
mod tests {
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
    fn follow_normal_crosses_the_local_arch_direction_with_the_view_direction() {
        // The disc's plane must still CONTAIN the view direction (so it
        // stays edge-on and legible from the camera's current angle,
        // rotating as the operator orbits to aim it) -- exactly like the raw
        // local-normal fallback below, just fed the STABLE local arch
        // direction instead of a noisy triangle normal.
        let frame = ArchFrame {
            centroid: Vec3::ZERO,
            axis0: Vec3::X,
            axis1: Vec3::Y,
        };
        let point = Vec3::new(5.0, 0.0, 0.0); // local arch direction here is +X
        let view_dir = Vec3::NEG_Z;
        let camera_right = Vec3::X;
        // The (wildly different) surface-normal argument must be irrelevant.
        let n = follow_plane_normal(
            Some(frame),
            point,
            Vec3::new(0.1, 0.9, 0.3),
            view_dir,
            camera_right,
        );
        let expected = Vec3::X.cross(view_dir).normalize();
        assert!(
            n.distance(expected) < 1e-5,
            "expected the local arch direction crossed with the view direction: {n} vs {expected}"
        );
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
    fn follow_normal_rotates_as_the_camera_orbits_around_a_fixed_point() {
        // Orbiting the camera must still re-aim the disc -- exactly like it
        // always did before this session's fixes -- so it stays edge-on and
        // legible as the operator turns to look at the cut from a better
        // angle, instead of freezing to one orientation regardless of view.
        let frame = ArchFrame {
            centroid: Vec3::ZERO,
            axis0: Vec3::X,
            axis1: Vec3::Y,
        };
        let point = Vec3::new(5.0, 0.0, 0.0); // local arch direction here is +X, fixed
        let camera_right = Vec3::X;
        let looking_along_neg_z =
            follow_plane_normal(Some(frame), point, Vec3::Y, Vec3::NEG_Z, camera_right);
        let looking_along_neg_y =
            follow_plane_normal(Some(frame), point, Vec3::Y, Vec3::NEG_Y, camera_right);
        assert!(
            looking_along_neg_z.dot(looking_along_neg_y).abs() < 0.05,
            "orbiting the camera to a very different view direction should visibly \
             re-aim the disc, not leave it frozen: {looking_along_neg_z} / {looking_along_neg_y}"
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
        let with_frame =
            follow_plane_normal(Some(frame), real_point, Vec3::Y, Vec3::NEG_Z, Vec3::X);
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
        let camera_right = Vec3::Z; // orthogonal to the arch plane: never flips sign here.
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
        let (t_near, _) =
            closest_param_on_segment(pos2(-5.0, 0.0), pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert!(t_near.abs() < 1e-6);
    }
}

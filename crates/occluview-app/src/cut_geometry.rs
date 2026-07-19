//! Stateless geometry for the cut disc: follow orientation, normal smoothing,
//! radius scaling, the handle hit-test, and the translate / push-pull / arcball
//! transforms. Split out of [`crate::cut_manipulator`] so both the pure math and
//! the state machine stay well under the file-size budget; every function here
//! is a pure function of its inputs, unit-tested in `cut_geometry_tests.rs`.

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

/// Follow orientation: the disc's plane normal is the LOCAL ARCH TANGENT at
/// `point` — the mesiodistal "along the arch" direction — so the disc plane
/// itself spans the occlusal (vertical) axis and the radial spoke through the
/// cursor: a saw blade standing upright, cutting TRANSVERSE to the arch at
/// exactly that spot. This is the anatomically correct separator orientation
/// everywhere around a horseshoe arch, and it is a WORLD-space property of
/// the surface point alone: orbiting the camera never re-tilts it. (The
/// previous `n x view_dir` construction only matched this from a straight
/// occlusal view; from a tilted or facial view its cross product drifted
/// toward the vertical axis and laid the disc flat — the reported
/// "top-to-bottom at the sides of the arch" bug.)
///
/// The tangent comes from [`local_arch_tangent`], built on `arch_frame` — the
/// mesh's own PCA centroid and greatest-variance axes (see
/// [`occluview_core::Mesh::principal_frame_cached`]). Because it derives from
/// a per-mesh-constant frame plus the hit POINT, it rotates smoothly as the
/// cursor sweeps along the arch and is immune to per-triangle normal jitter.
///
/// Only when no arch frame is available (a point cloud, or too few vertices
/// for a well-defined frame), or `point` projects onto the centroid exactly
/// (a defensive guard; never a real surface point), does it fall back to the
/// legacy view-coupled construction: `surface_normal x view_dir`, blended to
/// the camera-right axis when that cross product degenerates.
pub(crate) fn follow_plane_normal(
    arch_frame: Option<ArchFrame>,
    point: Vec3,
    surface_normal: Vec3,
    view_dir: Vec3,
    camera_right: Vec3,
) -> Vec3 {
    if let Some(tangent) = arch_frame.and_then(|frame| local_arch_tangent(frame, point)) {
        return tangent;
    }
    let fallback = camera_right.normalize_or(Vec3::X);
    let n = surface_normal.normalize_or_zero();
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

/// The LOCAL along-the-arch tangent at `point`: the occlusal axis
/// (`axis0 x axis1`, the frame's least-variance direction — perpendicular to
/// the arch plane) crossed with the radial spoke from [`local_arch_normal`].
/// A disc whose plane NORMAL is this tangent contains both the occlusal axis
/// and the spoke: it stands upright and cuts radially across the arch,
/// turning continuously as `point` sweeps around the curve. `None` only when
/// the spoke itself is undefined (point at the centroid) or the frame's axes
/// are degenerate.
fn local_arch_tangent(frame: ArchFrame, point: Vec3) -> Option<Vec3> {
    let spoke = local_arch_normal(frame, point)?;
    let occlusal_up = frame.axis0.cross(frame.axis1);
    let tangent = occlusal_up.cross(spoke).normalize_or_zero();
    (tangent.length_squared() > f32::EPSILON).then_some(tangent)
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
#[path = "cut_geometry_tests.rs"]
mod tests;

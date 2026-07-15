//! Pure geometry bridging the wall-thickness probe and the Section view.
//!
//! Two stateless, unit-tested pieces:
//!   * [`disc_pose_through_chord`] — the cross-section plane for feature D. Given
//!     a probe chord (entry -> exit through a wall), it builds a world cut disc
//!     whose plane CONTAINS the chord, so the wall reads edge-on in the section
//!     panel with the chord crossing it. Deterministic; safe on a degenerate or
//!     non-finite chord (returns `None`, so the caller never auto-plants).
//!   * [`slice_wall_thickness`] / [`wall_thickness_2d`] — feature E's one-click
//!     in-slice probe. From a click on the section contour it casts a ray along
//!     the local contour normal to the true nearest opposite segment (exact
//!     segment intersection, not a vertex), reporting the in-slice wall
//!     thickness. Honest `None` when nothing is hit.

use crate::cut_manipulator::{DiscPose, MAX_DISC_RADIUS_MM, MIN_DISC_RADIUS_MM};
use glam::{Vec2, Vec3};
use occluview_render::slice_view_basis;

/// A chord shorter than this (mm) is treated as degenerate: entry == exit gives
/// no direction to build a plane from, so no cut view is planted.
const MIN_CHORD_MM: f32 = 1.0e-4;
/// Skip ray hits closer than this (mm) to the origin so the probe never reports
/// its own segment (or a segment sharing the entry vertex) as the opposite wall.
const SELF_HIT_EPS_MM: f32 = 1.0e-3;
/// Radius of the planted disc relative to the measured wall thickness.
const RADIUS_PER_THICKNESS: f32 = 4.0;

/// Build a world cut disc whose plane contains the probe chord `entry -> exit`.
///
/// The plane normal is perpendicular to the chord (so the plane CONTAINS it and
/// both endpoints lie on it), oriented deterministically: `chord x ref`, where
/// `ref` is the world axis least parallel to the chord (a stable tie-break).
/// The disc is centered on `entry` and sized from the wall thickness, capped by
/// `scale_hint` (e.g. the mesh half-diagonal) so it never dwarfs the model.
///
/// Returns `None` for a degenerate (zero-length) or non-finite chord — an Open
/// reading has no exit and must never reach this path.
#[must_use]
pub(crate) fn disc_pose_through_chord(
    entry: Vec3,
    exit: Vec3,
    scale_hint: f32,
) -> Option<DiscPose> {
    let chord = exit - entry;
    if !entry.is_finite() || !chord.is_finite() || chord.length_squared() < MIN_CHORD_MM.powi(2) {
        return None;
    }
    let dir = chord.normalize();
    let reference = least_parallel_axis(dir);
    let mut normal = dir.cross(reference);
    if normal.length_squared() < f32::EPSILON {
        normal = dir.any_orthonormal_vector();
    }
    let plane_normal = normal.normalize_or_zero();
    if plane_normal.length_squared() < 0.5 {
        return None;
    }
    let thickness = chord.length();
    let cap = (scale_hint * 0.5).clamp(MIN_DISC_RADIUS_MM, MAX_DISC_RADIUS_MM);
    let radius_mm = (thickness * RADIUS_PER_THICKNESS).clamp(MIN_DISC_RADIUS_MM, cap);
    Some(DiscPose {
        center: entry,
        plane_normal,
        radius_mm,
    })
}

/// The world axis (`X`/`Y`/`Z`) least parallel to `dir`, chosen deterministically
/// (ties resolve X < Y < Z) so the plane orientation is reproducible.
fn least_parallel_axis(dir: Vec3) -> Vec3 {
    let (ax, ay, az) = (dir.x.abs(), dir.y.abs(), dir.z.abs());
    if ax <= ay && ax <= az {
        Vec3::X
    } else if ay <= az {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

/// One in-slice wall-thickness reading, in world coordinates on the section plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SliceProbe {
    /// Entry point on the clicked contour segment.
    pub(crate) entry: Vec3,
    /// Exit point on the nearest opposite contour segment.
    pub(crate) exit: Vec3,
    /// In-slice wall thickness (mm) = the entry->exit distance.
    pub(crate) thickness_mm: f32,
}

/// One-click in-slice wall probe in world space.
///
/// Projects the section-plane click and `segments` (world contour edges) into the
/// plane's 2D basis, runs [`wall_thickness_2d`], and maps the result back to
/// world. `None` when the click is off any segment or no opposite edge is hit.
#[must_use]
pub(crate) fn slice_wall_thickness(
    click_world: Vec3,
    normal: Vec3,
    segments: &[(Vec3, Vec3)],
) -> Option<SliceProbe> {
    if !click_world.is_finite() || segments.is_empty() {
        return None;
    }
    let (right, up) = slice_view_basis(normal);
    let origin = click_world;
    let to_2d = |world: Vec3| -> Vec2 {
        let d = world - origin;
        Vec2::new(right.dot(d), up.dot(d))
    };
    let segments_2d: Vec<(Vec2, Vec2)> = segments
        .iter()
        .map(|&(a, b)| (to_2d(a), to_2d(b)))
        .collect();
    let probe = wall_thickness_2d(Vec2::ZERO, &segments_2d)?;
    let to_world = |p: Vec2| origin + right * p.x + up * p.y;
    Some(SliceProbe {
        entry: to_world(probe.entry),
        exit: to_world(probe.exit),
        thickness_mm: probe.thickness_mm,
    })
}

/// The 2D result of an in-slice wall probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WallProbe2d {
    pub(crate) entry: Vec2,
    pub(crate) exit: Vec2,
    pub(crate) thickness_mm: f32,
}

/// Pure 2D one-click wall probe.
///
/// Snaps `click` to the nearest contour segment (the origin), then casts a ray
/// along both segment-normal directions and keeps the NEAREST crossing with any
/// other segment (exact ray-segment intersection). For a real wall the nearest
/// crossing is the opposite face, so the reading is the local wall thickness.
/// `None` when there is no segment to snap to or no opposite edge is hit.
#[must_use]
pub(crate) fn wall_thickness_2d(click: Vec2, segments: &[(Vec2, Vec2)]) -> Option<WallProbe2d> {
    let (origin_index, entry) = nearest_segment_point(click, segments)?;
    let (a, b) = segments[origin_index];
    let along = (b - a).normalize_or_zero();
    if along.length_squared() < 0.5 {
        return None;
    }
    let perp = Vec2::new(-along.y, along.x);
    let mut best: Option<(f32, Vec2)> = None;
    for dir in [perp, -perp] {
        if let Some(t) = nearest_ray_crossing(entry, dir, segments, origin_index) {
            if best.is_none_or(|(best_t, _)| t < best_t) {
                best = Some((t, entry + dir * t));
            }
        }
    }
    let (thickness_mm, exit) = best?;
    Some(WallProbe2d {
        entry,
        exit,
        thickness_mm,
    })
}

/// The `(segment index, closest point)` of the segment nearest `click`.
fn nearest_segment_point(click: Vec2, segments: &[(Vec2, Vec2)]) -> Option<(usize, Vec2)> {
    let mut best: Option<(f32, usize, Vec2)> = None;
    for (index, &(a, b)) in segments.iter().enumerate() {
        let point = closest_point_on_segment(click, a, b);
        let dist_sq = click.distance_squared(point);
        if best.is_none_or(|(best_dist, _, _)| dist_sq < best_dist) {
            best = Some((dist_sq, index, point));
        }
    }
    best.map(|(_, index, point)| (index, point))
}

/// Closest point to `p` on segment `a -> b` (clamped to the endpoints).
fn closest_point_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= f32::EPSILON {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

/// Nearest ray parameter `t > eps` where the ray `origin + t*dir` crosses a
/// segment other than `skip_index`. `dir` is unit, so `t` is a world distance.
fn nearest_ray_crossing(
    origin: Vec2,
    dir: Vec2,
    segments: &[(Vec2, Vec2)],
    skip_index: usize,
) -> Option<f32> {
    let mut nearest: Option<f32> = None;
    for (index, &(p, q)) in segments.iter().enumerate() {
        if index == skip_index {
            continue;
        }
        let Some(t) = ray_segment_t(origin, dir, p, q) else {
            continue;
        };
        if t > SELF_HIT_EPS_MM && nearest.is_none_or(|best| t < best) {
            nearest = Some(t);
        }
    }
    nearest
}

/// Ray parameter `t` where ray `origin + t*dir` meets segment `seg_a -> seg_b`,
/// or `None` on a parallel/degenerate configuration or a miss. Standard 2D
/// line-segment intersection; the segment parameter in `[0, 1]` keeps the hit on
/// the segment.
fn ray_segment_t(origin: Vec2, dir: Vec2, seg_a: Vec2, seg_b: Vec2) -> Option<f32> {
    let edge = seg_b - seg_a;
    let denom = cross_2d(dir, edge);
    if denom.abs() <= f32::EPSILON {
        return None;
    }
    let to_start = seg_a - origin;
    let ray_t = cross_2d(to_start, edge) / denom;
    let edge_s = cross_2d(to_start, dir) / denom;
    if !ray_t.is_finite() || !(0.0..=1.0).contains(&edge_s) {
        return None;
    }
    Some(ray_t)
}

/// 2D scalar cross product `u.x*v.y - u.y*v.x`.
fn cross_2d(u: Vec2, v: Vec2) -> f32 {
    u.x * v.y - u.y * v.x
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::expect_used)]
    use super::*;

    // ---- disc_pose_through_chord (feature D plane) -------------------------

    #[test]
    fn plane_contains_the_chord_and_centers_on_entry() {
        let entry = Vec3::new(1.0, 2.0, 3.0);
        let exit = Vec3::new(1.0, 2.0, 1.0); // 2 mm chord along -Z
        let pose = disc_pose_through_chord(entry, exit, 40.0).expect("pose");
        assert_eq!(pose.center, entry, "disc centers on the entry point");
        // Plane contains the chord: its normal is perpendicular to entry->exit,
        // and BOTH endpoints lie on the plane through `center`.
        let chord = exit - entry;
        assert!(
            pose.plane_normal.dot(chord).abs() < 1.0e-5,
            "normal not perpendicular to the chord: {}",
            pose.plane_normal
        );
        assert!(pose.plane_normal.dot(entry - pose.center).abs() < 1.0e-5);
        assert!(pose.plane_normal.dot(exit - pose.center).abs() < 1.0e-5);
        assert!((pose.plane_normal.length() - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn plane_from_chord_is_deterministic() {
        let entry = Vec3::new(-4.0, 0.5, 2.0);
        let exit = Vec3::new(-4.6, 0.5, 3.1);
        let a = disc_pose_through_chord(entry, exit, 30.0).expect("pose a");
        let b = disc_pose_through_chord(entry, exit, 30.0).expect("pose b");
        assert_eq!(a, b, "same inputs must give an identical pose");
    }

    #[test]
    fn degenerate_and_non_finite_chords_plant_nothing() {
        let p = Vec3::new(2.0, 2.0, 2.0);
        assert!(
            disc_pose_through_chord(p, p, 40.0).is_none(),
            "entry == exit"
        );
        assert!(
            disc_pose_through_chord(p, Vec3::NAN, 40.0).is_none(),
            "NaN exit"
        );
        assert!(
            disc_pose_through_chord(Vec3::new(f32::INFINITY, 0.0, 0.0), p, 40.0).is_none(),
            "non-finite entry"
        );
    }

    #[test]
    fn radius_tracks_thickness_and_is_capped_by_scale() {
        // A 2 mm wall with a generous scale hint: radius follows the thickness.
        let thin =
            disc_pose_through_chord(Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0), 100.0).expect("pose");
        assert!((thin.radius_mm - 8.0).abs() < 1.0e-4, "2mm*4 = 8mm radius");
        // A tiny model caps the radius so the disc never dwarfs it.
        let capped =
            disc_pose_through_chord(Vec3::ZERO, Vec3::new(0.0, 0.0, 5.0), 6.0).expect("pose");
        assert!(capped.radius_mm <= 3.0 + 1.0e-4, "scale_hint*0.5 caps it");
        assert!(capped.radius_mm >= MIN_DISC_RADIUS_MM);
    }

    // ---- wall_thickness_2d (feature E in-slice probe) ----------------------

    /// Two parallel horizontal walls 2 mm apart, each spanning x in [0, 10].
    fn parallel_walls() -> Vec<(Vec2, Vec2)> {
        vec![
            (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)),
            (Vec2::new(0.0, 2.0), Vec2::new(10.0, 2.0)),
        ]
    }

    #[test]
    fn parallel_walls_measure_the_gap_across_the_normal() {
        // Click just below the lower wall's interior: entry snaps to (5, 0), the
        // inward normal points +y, and the ray hits the upper wall at (5, 2).
        let probe = wall_thickness_2d(Vec2::new(5.0, -0.3), &parallel_walls()).expect("probe");
        assert!((probe.entry - Vec2::new(5.0, 0.0)).length() < 1.0e-4);
        assert!((probe.exit - Vec2::new(5.0, 2.0)).length() < 1.0e-4);
        assert!(
            (probe.thickness_mm - 2.0).abs() < 1.0e-4,
            "{}",
            probe.thickness_mm
        );
    }

    /// An L: a horizontal leg y=0 (x in [0,10]) and a vertical leg x=10
    /// (y in [0,10]) meeting at the corner (10, 0), plus a far parallel wall.
    fn l_with_far_wall() -> Vec<(Vec2, Vec2)> {
        vec![
            (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)),
            (Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)),
            // A far opposite wall 6 mm above the horizontal leg.
            (Vec2::new(0.0, 6.0), Vec2::new(10.0, 6.0)),
        ]
    }

    #[test]
    fn in_slice_probe_picks_the_true_nearest_opposite_segment() {
        // Click on the horizontal leg near x=3: entry (3,0). Casting +y hits the
        // far wall at (3,6) => 6 mm. The vertical leg (the OTHER near segment)
        // is skipped as it is parallel to the ray, and the reading is the exact
        // segment intersection, not the nearest vertex.
        let probe = wall_thickness_2d(Vec2::new(3.0, 0.1), &l_with_far_wall()).expect("probe");
        assert!((probe.entry - Vec2::new(3.0, 0.0)).length() < 1.0e-4);
        assert!(
            (probe.exit - Vec2::new(3.0, 6.0)).length() < 1.0e-4,
            "exit must be the exact intersection on the far wall, got {}",
            probe.exit
        );
        assert!((probe.thickness_mm - 6.0).abs() < 1.0e-4);
    }

    #[test]
    fn open_edge_with_no_opposite_wall_reads_nothing() {
        // A lone segment: casting either normal direction hits nothing.
        let lone = vec![(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0))];
        assert!(wall_thickness_2d(Vec2::new(5.0, 0.2), &lone).is_none());
    }

    #[test]
    fn empty_contour_is_a_no_op() {
        assert!(wall_thickness_2d(Vec2::ZERO, &[]).is_none());
    }

    #[test]
    fn world_probe_round_trips_through_the_plane_basis() {
        // Two parallel walls 2 mm apart in the world z=0 plane (normal +Z). The
        // world probe must recover a 2 mm thickness and endpoints on the plane.
        let normal = Vec3::Z;
        let segments = vec![
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)),
            (Vec3::new(0.0, 2.0, 0.0), Vec3::new(10.0, 2.0, 0.0)),
        ];
        let click = Vec3::new(5.0, -0.3, 0.0);
        let probe = slice_wall_thickness(click, normal, &segments).expect("probe");
        assert!(
            (probe.thickness_mm - 2.0).abs() < 1.0e-3,
            "{}",
            probe.thickness_mm
        );
        assert!(probe.entry.z.abs() < 1.0e-4 && probe.exit.z.abs() < 1.0e-4);
        assert!(probe.entry.distance(Vec3::new(5.0, 0.0, 0.0)) < 1.0e-3);
        assert!(probe.exit.distance(Vec3::new(5.0, 2.0, 0.0)) < 1.0e-3);
    }
}

//! Ruler records and the geometry that resolves them into drawable segments.
//!
//! A ruler is a free two-point segment between picked surface points, or it
//! ends on the line of an earlier ruler. On a line the end either sits at a
//! chosen place along it, so the angle to the line is whatever the operator
//! made it, or it is the foot of the perpendicular, re-derived on every read so
//! the angle stays at exactly 90 degrees in 3D. Either way the reading is the
//! 3D length and the smaller angle between the two rulers.
//!
//! This is the orthodontic study-model measurement where the end lies in the
//! air: Korkhaus' anterior arch length runs from the incisal point between the
//! central incisors to the line through Pont's premolar points, and that line
//! passes above the palate, not over any surface.

use glam::{DVec3, Vec3};

/// A base line shorter than this has no usable direction; a ruler ending on it
/// then measures to its (single) point. Far below the point spacing of any
/// dental scan.
const MIN_BASE_LENGTH_MM: f32 = 1.0e-3;

/// How close to parallel (sine of the angle between them) a pointer ray and a
/// base line may come before a place along the line can no longer be read from
/// the ray: seen end-on, every place on the line is under the same pixel.
const MIN_RAY_LINE_SINE: f64 = 1.0e-3;

/// Where a completed ruler ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RulerEnd {
    /// A picked surface point.
    Point(Vec3),
    /// A place on the line of the ruler at index `base` (always an earlier
    /// ruler): `t` is 0 at its start, 1 at its end, and outside `[0, 1]` on
    /// the line's extension. The place follows the base when either of its
    /// ends moves; the angle is whatever the geometry makes it.
    OnLine { base: usize, t: f32 },
    /// The foot of the perpendicular from the ruler's start onto the line of
    /// the ruler at this index. Always an earlier ruler.
    FootOn(usize),
}

/// How a ruler meets the line of another ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LinePlacement {
    /// At this place along the line (see [`RulerEnd::OnLine`]).
    At(f32),
    /// At the foot of the perpendicular, held at 90 degrees.
    Perpendicular,
}

/// One stored ruler: the picked start and how it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerMeasurement {
    pub(crate) a: Vec3,
    pub(crate) end: RulerEnd,
}

/// Where a ruler's end falls on the line of its base ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerFoot {
    /// Index of the base ruler.
    pub(crate) base: usize,
    /// Position along the base: 0 at its start, 1 at its end, outside
    /// `[0, 1]` on the line's extension.
    pub(crate) t: f32,
    /// Held at 90 degrees rather than placed along the line.
    pub(crate) perpendicular: bool,
    /// The smaller angle between the ruler and the base line, in degrees
    /// (0 to 90); `None` when either has no direction.
    pub(crate) angle_deg: Option<f64>,
}

/// A ruler resolved to world-space endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerSegment {
    pub(crate) a: Vec3,
    pub(crate) b: Vec3,
    /// Present for a ruler that ends on another ruler's line.
    pub(crate) foot: Option<RulerFoot>,
}

impl RulerSegment {
    /// Straight-line distance in millimeters (`f64` accumulation).
    pub(crate) fn distance_mm(&self) -> f64 {
        DVec3::from(self.a).distance(DVec3::from(self.b))
    }
}

/// The closest point to `point` on the infinite line through `line_a` and
/// `line_b`, with its parameter along `line_a -> line_b`. A line shorter than
/// [`MIN_BASE_LENGTH_MM`] collapses to `line_a` (`t = 0`).
pub(crate) fn foot_on_line(point: Vec3, line_a: Vec3, line_b: Vec3) -> (Vec3, f32) {
    let direction = line_b - line_a;
    let length_sq = direction.length_squared();
    if !length_sq.is_finite() || length_sq < MIN_BASE_LENGTH_MM * MIN_BASE_LENGTH_MM {
        return (line_a, 0.0);
    }
    let t = (point - line_a).dot(direction) / length_sq;
    (line_a + direction * t, t)
}

/// The point at `t` along the line through `line_a` and `line_b`. A line
/// shorter than [`MIN_BASE_LENGTH_MM`] collapses to `line_a`.
fn point_on_line(line_a: Vec3, line_b: Vec3, t: f32) -> Vec3 {
    let direction = line_b - line_a;
    let length_sq = direction.length_squared();
    if !length_sq.is_finite() || length_sq < MIN_BASE_LENGTH_MM * MIN_BASE_LENGTH_MM {
        return line_a;
    }
    line_a + direction * t
}

/// The place on the line through `line_a` and `line_b` nearest to the ray from
/// `origin` along `direction`: the parameter along `line_a -> line_b` of the
/// closest pair of points between the two lines. This is what the pointer
/// picks on a line drawn in the air, and it is exact under perspective.
/// `None` for a degenerate line or ray, or when the ray runs along the line.
pub(crate) fn line_place_under_ray(
    line_a: Vec3,
    line_b: Vec3,
    origin: Vec3,
    direction: Vec3,
) -> Option<f32> {
    let along = DVec3::from(line_b - line_a);
    let ray = DVec3::from(direction);
    let offset = DVec3::from(line_a - origin);
    let along_sq = along.length_squared();
    let ray_sq = ray.length_squared();
    let min_length_sq = f64::from(MIN_BASE_LENGTH_MM * MIN_BASE_LENGTH_MM);
    if !(along_sq.is_finite() && ray_sq.is_finite())
        || along_sq < min_length_sq
        || ray_sq <= f64::EPSILON
    {
        return None;
    }
    let cross_term = along.dot(ray);
    // |along|^2 |ray|^2 sin^2 of the angle between them.
    let denominator = along_sq.mul_add(ray_sq, -(cross_term * cross_term));
    let parallel_limit = MIN_RAY_LINE_SINE * MIN_RAY_LINE_SINE * along_sq * ray_sq;
    if !denominator.is_finite() || denominator <= parallel_limit {
        return None;
    }
    let t = cross_term.mul_add(ray.dot(offset), -(ray_sq * along.dot(offset))) / denominator;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a place along a ruler line is stored in the ruler's own f32"
    )]
    let t = t as f32;
    t.is_finite().then_some(t)
}

/// The smaller angle, in degrees, between the ruler `a -> b` and the line
/// through `line_a` and `line_b`: 0 when they are parallel, 90 when they are
/// perpendicular. `None` when either has no direction.
fn angle_to_line_deg(a: Vec3, b: Vec3, line_a: Vec3, line_b: Vec3) -> Option<f64> {
    let ruler = DVec3::from(a) - DVec3::from(b);
    let line = DVec3::from(line_b) - DVec3::from(line_a);
    let lengths = ruler.length() * line.length();
    let min_length = f64::from(MIN_BASE_LENGTH_MM);
    if !lengths.is_finite() || lengths < min_length * min_length {
        return None;
    }
    let cosine = (ruler.dot(line).abs() / lengths).clamp(0.0, 1.0);
    Some(cosine.acos().to_degrees())
}

/// The ruler from `from` that ends on the line of `base` as `placement` says.
pub(crate) fn onto_line(
    from: Vec3,
    base: &RulerSegment,
    base_index: usize,
    placement: LinePlacement,
) -> RulerSegment {
    let (end, t, perpendicular) = match placement {
        LinePlacement::At(t) => (point_on_line(base.a, base.b, t), t, false),
        LinePlacement::Perpendicular => {
            let (foot, t) = foot_on_line(from, base.a, base.b);
            (foot, t, true)
        }
    };
    RulerSegment {
        a: from,
        b: end,
        foot: Some(RulerFoot {
            base: base_index,
            t,
            perpendicular,
            angle_deg: angle_to_line_deg(from, end, base.a, base.b),
        }),
    }
}

/// Resolve every ruler in order. The output has exactly one segment per
/// ruler, at the same index. A ruler ending on a line always refers to an
/// earlier ruler, so one forward pass sees every base already resolved.
pub(crate) fn resolve(rulers: &[RulerMeasurement]) -> Vec<RulerSegment> {
    let mut segments: Vec<RulerSegment> = Vec::with_capacity(rulers.len());
    for ruler in rulers {
        let (base, placement) = match ruler.end {
            RulerEnd::Point(b) => {
                segments.push(RulerSegment {
                    a: ruler.a,
                    b,
                    foot: None,
                });
                continue;
            }
            RulerEnd::OnLine { base, t } => (base, LinePlacement::At(t)),
            RulerEnd::FootOn(base) => (base, LinePlacement::Perpendicular),
        };
        let segment = match segments.get(base) {
            Some(base_segment) => onto_line(ruler.a, base_segment, base, placement),
            // Unreachable through `MeasureTool`, which only ends a ruler on an
            // existing ruler: read as zero length.
            None => RulerSegment {
                a: ruler.a,
                b: ruler.a,
                foot: None,
            },
        };
        segments.push(segment);
    }
    segments
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    fn free(a: Vec3, b: Vec3) -> RulerMeasurement {
        RulerMeasurement {
            a,
            end: RulerEnd::Point(b),
        }
    }

    fn perpendicular(a: Vec3, base: usize) -> RulerMeasurement {
        RulerMeasurement {
            a,
            end: RulerEnd::FootOn(base),
        }
    }

    fn on_line(a: Vec3, base: usize, t: f32) -> RulerMeasurement {
        RulerMeasurement {
            a,
            end: RulerEnd::OnLine { base, t },
        }
    }

    fn angle(segment: &RulerSegment) -> f64 {
        segment
            .foot
            .and_then(|foot| foot.angle_deg)
            .expect("an angle to the base")
    }

    /// Korkhaus layout: a transverse premolar line and an incisal point in
    /// front of it and above it (the palate is lower, the incisal edge is not
    /// on the premolar line's height). The reading is the 3D perpendicular.
    #[test]
    fn perpendicular_reads_the_3d_distance_to_the_premolar_line() {
        let left_premolar = Vec3::new(-16.5, 0.0, 0.0);
        let right_premolar = Vec3::new(16.5, 2.0, 0.0);
        let incisal = Vec3::new(0.5, 18.0, 3.0);
        let segments = resolve(&[
            free(left_premolar, right_premolar),
            perpendicular(incisal, 0),
        ]);
        let perpendicular = segments[1];
        let line = right_premolar - left_premolar;
        let leg = perpendicular.a - perpendicular.b;
        assert!(
            leg.dot(line).abs() < 1.0e-4,
            "the reading must stand at 90 degrees on the base line"
        );
        // Distance from a point to a line: |(P - A) x d| / |d|.
        let expected = f64::from((incisal - left_premolar).cross(line).length() / line.length());
        assert!((perpendicular.distance_mm() - expected).abs() < 1.0e-4);
        // The foot is on the base line itself, not on some surface below it.
        let (_, t) = foot_on_line(incisal, left_premolar, right_premolar);
        let on_line = left_premolar + line * t;
        assert!(perpendicular.b.distance(on_line) < 1.0e-5);
        let foot = perpendicular.foot.expect("ends on a line");
        assert_eq!(foot.base, 0);
        assert!(foot.perpendicular);
        assert!((angle(&perpendicular) - 90.0).abs() < 1.0e-3);
    }

    /// A place along the line gives the 3D length to that place and the
    /// smaller of the two angles the rulers make, whichever side it leans to.
    #[test]
    fn a_place_on_the_line_reads_its_3d_length_and_the_smaller_angle() {
        let rulers = [
            free(Vec3::ZERO, Vec3::X * 20.0),
            // Leans toward the base's end: 45 degrees in the XY plane.
            on_line(Vec3::new(15.0, 5.0, 0.0), 0, 0.5),
            // Leans toward the base's start by the same amount.
            on_line(Vec3::new(5.0, 5.0, 0.0), 0, 0.5),
            // Lifted out of the base's plane: still the 3D angle.
            on_line(Vec3::new(10.0, 3.0, 4.0), 0, 0.5),
        ];
        let segments = resolve(&rulers);
        let toward_end = segments[1];
        assert_eq!(toward_end.b, Vec3::new(10.0, 0.0, 0.0));
        assert!((toward_end.distance_mm() - 50.0_f64.sqrt()).abs() < 1.0e-6);
        assert!((angle(&toward_end) - 45.0).abs() < 1.0e-4);
        let foot = toward_end.foot.expect("ends on a line");
        assert!(!foot.perpendicular);
        assert_eq!(foot.t, 0.5);
        assert!(
            (angle(&segments[2]) - 45.0).abs() < 1.0e-4,
            "the smaller angle, not its 135 degree supplement"
        );
        assert_eq!(segments[3].distance_mm(), 5.0);
        assert!(
            (angle(&segments[3]) - 90.0).abs() < 1.0e-4,
            "a ruler square to the line in 3D reads 90 degrees"
        );
    }

    /// A place on the line keeps its place along the base when the base or the
    /// start moves; only a perpendicular re-derives its foot.
    #[test]
    fn a_place_on_the_line_follows_the_base_and_keeps_its_share_of_it() {
        let mut rulers = vec![
            free(Vec3::ZERO, Vec3::X * 10.0),
            on_line(Vec3::new(3.0, 5.0, 0.0), 0, 0.25),
        ];
        assert_eq!(resolve(&rulers)[1].b, Vec3::new(2.5, 0.0, 0.0));
        rulers[0].end = RulerEnd::Point(Vec3::X * 20.0);
        assert_eq!(resolve(&rulers)[1].b, Vec3::new(5.0, 0.0, 0.0));
        rulers[1].a = Vec3::new(-7.0, 1.0, 0.0);
        let moved = resolve(&rulers)[1];
        assert_eq!(moved.b, Vec3::new(5.0, 0.0, 0.0));
        assert!(
            angle(&moved) < 5.0,
            "a start moved along the line flattens it"
        );
    }

    #[test]
    fn a_foot_beyond_the_base_ends_lies_on_the_extension() {
        let (foot, t) = foot_on_line(Vec3::new(10.0, 3.0, 0.0), Vec3::ZERO, Vec3::X * 4.0);
        assert_eq!(foot, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(t, 2.5);
        let (foot, t) = foot_on_line(Vec3::new(-2.0, 3.0, 0.0), Vec3::ZERO, Vec3::X * 4.0);
        assert_eq!(foot, Vec3::new(-2.0, 0.0, 0.0));
        assert_eq!(t, -0.5);
        let segments = resolve(&[
            free(Vec3::ZERO, Vec3::X * 4.0),
            on_line(Vec3::new(9.0, 3.0, 0.0), 0, 1.5),
        ]);
        assert_eq!(segments[1].b, Vec3::new(6.0, 0.0, 0.0));
    }

    #[test]
    fn a_degenerate_base_measures_to_its_point_not_nan() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        let segments = resolve(&[
            free(p, p),
            perpendicular(Vec3::new(4.0, 6.0, 3.0), 0),
            on_line(Vec3::new(4.0, 6.0, 3.0), 0, 0.7),
        ]);
        for segment in &segments[1..] {
            assert_eq!(segment.b, p);
            assert_eq!(segment.distance_mm(), 5.0);
            assert_eq!(
                segment.foot.and_then(|foot| foot.angle_deg),
                None,
                "a base with no direction has no angle to report"
            );
        }
        let (foot, t) = foot_on_line(Vec3::ONE, Vec3::NAN, Vec3::ZERO);
        assert!(
            foot.is_nan() && t == 0.0,
            "a poisoned base must not invent a foot"
        );
    }

    /// The perpendicular's foot is derived, not stored: moving an end of the
    /// base re-derives it, so the reading never drifts off 90 degrees.
    #[test]
    fn moving_the_base_keeps_the_right_angle() {
        let mut rulers = vec![
            free(Vec3::ZERO, Vec3::X * 10.0),
            perpendicular(Vec3::new(3.0, 5.0, 0.0), 0),
        ];
        let before = resolve(&rulers)[1];
        assert_eq!(before.b, Vec3::new(3.0, 0.0, 0.0));
        rulers[0].end = RulerEnd::Point(Vec3::new(10.0, 10.0, 0.0));
        let after = resolve(&rulers)[1];
        let line = Vec3::new(10.0, 10.0, 0.0);
        assert!((after.a - after.b).dot(line).abs() < 1.0e-4);
        assert!(after.b.distance(Vec3::new(4.0, 4.0, 0.0)) < 1.0e-5);
    }

    #[test]
    fn a_ruler_can_end_on_a_ruler_that_itself_ends_on_a_line() {
        let segments = resolve(&[
            free(Vec3::ZERO, Vec3::X * 10.0),
            perpendicular(Vec3::new(4.0, 8.0, 0.0), 0),
            perpendicular(Vec3::new(9.0, 2.0, 0.0), 1),
            on_line(Vec3::new(9.0, 2.0, 0.0), 1, 0.5),
        ]);
        assert_eq!(segments[1].b, Vec3::new(4.0, 0.0, 0.0));
        assert_eq!(segments[2].b, Vec3::new(4.0, 2.0, 0.0));
        assert_eq!(segments[2].distance_mm(), 5.0);
        assert_eq!(segments[3].b, Vec3::new(4.0, 4.0, 0.0));
    }

    #[test]
    fn resolve_keeps_one_segment_per_ruler_even_for_a_dangling_base() {
        let segments = resolve(&[
            perpendicular(Vec3::ONE, 3),
            on_line(Vec3::ONE, 5, 0.5),
            free(Vec3::ZERO, Vec3::X),
        ]);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].distance_mm(), 0.0);
        assert_eq!(segments[1].distance_mm(), 0.0);
        assert_eq!(segments[2].b, Vec3::X);
    }

    /// The pointer ray picks the place on the line it passes closest to, also
    /// when the line runs toward the camera (where a 2D projection of the
    /// pointer onto the drawn line would be off under perspective).
    #[test]
    fn a_pointer_ray_picks_the_place_on_the_line_it_passes_closest_to() {
        let (a, b) = (Vec3::ZERO, Vec3::X * 10.0);
        let t = line_place_under_ray(a, b, Vec3::new(3.0, 5.0, 40.0), -Vec3::Z).expect("a place");
        assert!((t - 0.3).abs() < 1.0e-6);
        // A line receding from the camera, seen by an oblique ray.
        let (a, b) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -20.0));
        let origin = Vec3::new(0.0, 30.0, 10.0);
        let target = Vec3::new(0.0, 0.0, -15.0);
        let t = line_place_under_ray(a, b, origin, target - origin).expect("a place");
        assert!((t - 0.75).abs() < 1.0e-5);
        // Beyond the drawn ends the place lies on the extension.
        let t = line_place_under_ray(Vec3::ZERO, Vec3::X, Vec3::new(3.0, 0.0, 9.0), -Vec3::Z)
            .expect("a place");
        assert!((t - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn a_ray_along_the_line_or_a_degenerate_line_picks_no_place() {
        assert_eq!(
            line_place_under_ray(Vec3::ZERO, Vec3::X, Vec3::new(-5.0, 0.0, 0.0), Vec3::X),
            None,
            "seen end-on every place is under the same pixel"
        );
        assert_eq!(
            line_place_under_ray(Vec3::ONE, Vec3::ONE, Vec3::Z * 5.0, -Vec3::Z),
            None
        );
        assert_eq!(
            line_place_under_ray(Vec3::ZERO, Vec3::X, Vec3::Z * 5.0, Vec3::ZERO),
            None
        );
        assert_eq!(
            line_place_under_ray(Vec3::ZERO, Vec3::X, Vec3::NAN, -Vec3::Z),
            None
        );
    }
}

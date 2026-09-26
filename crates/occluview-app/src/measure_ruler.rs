//! Ruler records and the geometry that resolves them into drawable segments.
//!
//! A ruler is either a free two-point segment between picked surface points,
//! or a perpendicular dropped from a picked point onto the line of an earlier
//! ruler. The perpendicular stores which ruler it stands on, never its foot:
//! the foot is re-derived on every read, so moving the picked point or either
//! end of the base ruler keeps the angle at exactly 90 degrees in 3D.
//!
//! This is the orthodontic study-model measurement where the foot lies in the
//! air: Korkhaus' anterior arch length is the perpendicular from the incisal
//! point between the central incisors to the line through Pont's premolar
//! points, and that line passes above the palate, not over any surface.

use glam::Vec3;

/// A base line shorter than this has no usable direction; the perpendicular
/// then measures to its (single) point. Far below the point spacing of any
/// dental scan.
const MIN_BASE_LENGTH_MM: f32 = 1.0e-3;

/// Where a completed ruler ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RulerEnd {
    /// A picked surface point.
    Point(Vec3),
    /// The foot of the perpendicular from the ruler's start onto the line of
    /// the ruler at this index. Always an earlier ruler.
    FootOn(usize),
}

/// One stored ruler: the picked start and how it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerMeasurement {
    pub(crate) a: Vec3,
    pub(crate) end: RulerEnd,
}

/// Where a perpendicular's foot falls on its base ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerFoot {
    /// Index of the base ruler.
    pub(crate) base: usize,
    /// Position along the base: 0 at its start, 1 at its end, outside
    /// `[0, 1]` on the line's extension.
    pub(crate) t: f32,
}

/// A ruler resolved to world-space endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerSegment {
    pub(crate) a: Vec3,
    pub(crate) b: Vec3,
    /// Present for a perpendicular.
    pub(crate) foot: Option<RulerFoot>,
}

impl RulerSegment {
    /// Straight-line distance in millimeters (`f64` accumulation).
    pub(crate) fn distance_mm(&self) -> f64 {
        let dx = f64::from(self.a.x) - f64::from(self.b.x);
        let dy = f64::from(self.a.y) - f64::from(self.b.y);
        let dz = f64::from(self.a.z) - f64::from(self.b.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
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

/// The perpendicular from `from` onto the line of `base`.
pub(crate) fn perpendicular_onto(
    from: Vec3,
    base: &RulerSegment,
    base_index: usize,
) -> RulerSegment {
    let (foot, t) = foot_on_line(from, base.a, base.b);
    RulerSegment {
        a: from,
        b: foot,
        foot: Some(RulerFoot {
            base: base_index,
            t,
        }),
    }
}

/// Resolve every ruler in order. The output has exactly one segment per
/// ruler, at the same index. A perpendicular always refers to an earlier
/// ruler, so one forward pass sees every base already resolved.
pub(crate) fn resolve(rulers: &[RulerMeasurement]) -> Vec<RulerSegment> {
    let mut segments: Vec<RulerSegment> = Vec::with_capacity(rulers.len());
    for ruler in rulers {
        let segment = match ruler.end {
            RulerEnd::Point(b) => RulerSegment {
                a: ruler.a,
                b,
                foot: None,
            },
            RulerEnd::FootOn(base) => match segments.get(base) {
                Some(base_segment) => perpendicular_onto(ruler.a, base_segment, base),
                // Unreachable through `MeasureTool`, which only stores a
                // perpendicular onto an existing ruler: read as zero length.
                None => RulerSegment {
                    a: ruler.a,
                    b: ruler.a,
                    foot: None,
                },
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
        assert_eq!(perpendicular.foot.map(|foot| foot.base), Some(0));
    }

    #[test]
    fn a_foot_beyond_the_base_ends_lies_on_the_extension() {
        let (foot, t) = foot_on_line(Vec3::new(10.0, 3.0, 0.0), Vec3::ZERO, Vec3::X * 4.0);
        assert_eq!(foot, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(t, 2.5);
        let (foot, t) = foot_on_line(Vec3::new(-2.0, 3.0, 0.0), Vec3::ZERO, Vec3::X * 4.0);
        assert_eq!(foot, Vec3::new(-2.0, 0.0, 0.0));
        assert_eq!(t, -0.5);
    }

    #[test]
    fn a_degenerate_base_measures_to_its_point_not_nan() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        let segments = resolve(&[free(p, p), perpendicular(Vec3::new(4.0, 6.0, 3.0), 0)]);
        assert_eq!(segments[1].b, p);
        assert_eq!(segments[1].distance_mm(), 5.0);
        let (foot, t) = foot_on_line(Vec3::ONE, Vec3::NAN, Vec3::ZERO);
        assert!(
            foot.is_nan() && t == 0.0,
            "a poisoned base must not invent a foot"
        );
    }

    /// The foot is derived, not stored: moving an end of the base re-derives
    /// it, so the reading never drifts off 90 degrees.
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
    fn a_perpendicular_can_stand_on_another_perpendicular() {
        let segments = resolve(&[
            free(Vec3::ZERO, Vec3::X * 10.0),
            perpendicular(Vec3::new(4.0, 8.0, 0.0), 0),
            perpendicular(Vec3::new(9.0, 2.0, 0.0), 1),
        ]);
        assert_eq!(segments[1].b, Vec3::new(4.0, 0.0, 0.0));
        assert_eq!(segments[2].b, Vec3::new(4.0, 2.0, 0.0));
        assert_eq!(segments[2].distance_mm(), 5.0);
    }

    #[test]
    fn resolve_keeps_one_segment_per_ruler_even_for_a_dangling_base() {
        let segments = resolve(&[perpendicular(Vec3::ONE, 3), free(Vec3::ZERO, Vec3::X)]);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].distance_mm(), 0.0);
        assert_eq!(segments[1].b, Vec3::X);
    }
}

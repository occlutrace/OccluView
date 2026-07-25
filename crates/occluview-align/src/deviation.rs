//! PROOF: the signed deviation map, its statistics, and the ramp the viewport
//! paints it with.
//!
//! The sign lives in the measured value; the ramp only answers "how far". A
//! vertex with no fixed surface inside the influence radius is **not**
//! measured — it is marked and painted grey, because painting it at full scale
//! would be a lie, and letting it into the statistics would be a worse one.

use glam::DVec3;
use rayon::prelude::*;

use crate::icp::Orientation;
use crate::sample::vertex_at;
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

/// Colour for a vertex whose deviation could not be measured.
pub const NO_DATA_COLOR: [u8; 4] = [128, 128, 128, 255];

/// Below this normal length a hit carries no usable direction, so the sign of
/// the deviation would be arbitrary.
const MIN_NORMAL_LENGTH: f64 = 1e-9;

/// Why a vertex does or does not carry a measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    /// The value is a real measurement.
    Measured,
    /// No fixed surface within the influence radius.
    OutOfReach,
    /// The fixed surface there has no usable normal.
    DegenerateNormal,
    /// The moving vertex itself is not finite.
    NonFinite,
}

/// What to measure and how far to look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviationSettings {
    /// Farthest a moving vertex may look for fixed surface, in millimetres.
    pub influence_radius_mm: f64,
    /// How the two surfaces are taken to face each other.
    pub orientation: Orientation,
}

impl Default for DeviationSettings {
    fn default() -> Self {
        Self {
            influence_radius_mm: 2.0,
            orientation: Orientation::Match,
        }
    }
}

/// One signed value and one validity per moving vertex, in vertex order.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviationMap {
    /// Signed distance in millimetres. Positive means the moving surface lies
    /// outside the fixed one along its outward normal.
    pub signed_mm: Vec<f32>,
    /// Whether each entry is a measurement.
    pub validity: Vec<Validity>,
}

/// Summary over the measured vertices only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviationStats {
    /// Share of measured vertices within the tolerance band, 0 to 1.
    pub within_tolerance: f64,
    /// Mean absolute deviation, in millimetres.
    pub mean_abs: f64,
    /// Root-mean-square deviation, in millimetres.
    pub rms: f64,
    /// Median signed deviation, in millimetres.
    pub median: f64,
    /// 95th-percentile absolute deviation, in millimetres.
    pub p95: f64,
    /// Vertices that carry a measurement.
    pub measured: u32,
    /// Vertices that do not.
    pub skipped: u32,
}

/// How to turn measurements into colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampSettings {
    /// Deviation mapped to the ramp ends, in millimetres.
    pub scale_mm: f64,
    /// Tolerance band the statistics report, in millimetres. The ramp itself
    /// does not change with it.
    pub tolerance_mm: f64,
    /// Steps per side for a banded ramp; `None` is continuous. A stepped map
    /// shows where a boundary falls far more sharply than a smooth one.
    pub bands: Option<u32>,
}

impl Default for RampSettings {
    fn default() -> Self {
        Self {
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
        }
    }
}

/// Ramp stops from `-1` to `+1`, saturated at both ends so the display scale
/// reads hard. Blue is undersize, green is nominal, red is oversize — the
/// convention every metrology package shares.
const RAMP: [(f64, [u8; 3]); 5] = [
    (-1.0, [0, 32, 255]),
    (-0.5, [0, 200, 255]),
    (0.0, [0, 220, 60]),
    (0.5, [255, 220, 0]),
    (1.0, [255, 24, 0]),
];

/// Measure every moving vertex against the fixed surface under `pose`.
#[must_use]
pub fn deviation(
    moving: Soup<'_>,
    fixed: &SurfaceIndex,
    pose: Rigid,
    settings: &DeviationSettings,
    cancel: &CancelFlag,
) -> DeviationMap {
    let count = moving.vertex_count();
    let measured: Vec<(f32, Validity)> = (0..count)
        .into_par_iter()
        .map(|vertex| {
            // Checked per vertex, not per block: a relaxed atomic load costs a
            // few nanoseconds against a surface query that costs hundreds, and
            // a block stride would leave a small mesh finishing a cancelled
            // job.
            if cancel.is_cancelled() {
                return (0.0, Validity::OutOfReach);
            }
            measure_vertex(moving, fixed, pose, settings, vertex)
        })
        .collect();

    let mut signed_mm = Vec::with_capacity(count);
    let mut validity = Vec::with_capacity(count);
    for (value, state) in measured {
        signed_mm.push(value);
        validity.push(state);
    }
    DeviationMap {
        signed_mm,
        validity,
    }
}

/// One vertex's measurement.
#[allow(clippy::cast_possible_truncation)]
fn measure_vertex(
    moving: Soup<'_>,
    fixed: &SurfaceIndex,
    pose: Rigid,
    settings: &DeviationSettings,
    vertex: usize,
) -> (f32, Validity) {
    let Some(local) = vertex_at(moving.positions, vertex) else {
        return (0.0, Validity::NonFinite);
    };
    let point = pose.apply(local);
    let Some(hit) = fixed.nearest(point, settings.influence_radius_mm) else {
        return (0.0, Validity::OutOfReach);
    };
    if hit.normal.length() < MIN_NORMAL_LENGTH {
        return (0.0, Validity::DegenerateNormal);
    }
    let offset = point - hit.point;
    let distance = offset.length();
    let signed = match settings.orientation {
        Orientation::Ignored => distance,
        Orientation::Match => signed_along(offset, hit.normal, distance),
        Orientation::Inverted => -signed_along(offset, hit.normal, distance),
    };
    (signed as f32, Validity::Measured)
}

/// Distance carrying the sign of which side of the surface the point sits on.
fn signed_along(offset: DVec3, normal: DVec3, distance: f64) -> f64 {
    if offset.dot(normal) >= 0.0 {
        distance
    } else {
        -distance
    }
}

/// Summarize a map over its measured vertices only.
#[must_use]
pub fn deviation_stats(map: &DeviationMap, tolerance_mm: f64) -> DeviationStats {
    let mut values: Vec<f64> = map
        .signed_mm
        .iter()
        .zip(&map.validity)
        .filter(|(_, state)| **state == Validity::Measured)
        .map(|(value, _)| f64::from(*value))
        .collect();
    let measured = u32::try_from(values.len()).unwrap_or(u32::MAX);
    let skipped =
        u32::try_from(map.validity.len().saturating_sub(values.len())).unwrap_or(u32::MAX);
    if values.is_empty() {
        return DeviationStats {
            within_tolerance: 0.0,
            mean_abs: 0.0,
            rms: 0.0,
            median: 0.0,
            p95: 0.0,
            measured,
            skipped,
        };
    }

    #[allow(clippy::cast_precision_loss)]
    let count = values.len() as f64;
    let inside = values
        .iter()
        .filter(|value| value.abs() <= tolerance_mm)
        .count();
    let mean_abs = values.iter().map(|value| value.abs()).sum::<f64>() / count;
    let rms = (values.iter().map(|value| value * value).sum::<f64>() / count).sqrt();

    let mut magnitudes: Vec<f64> = values.iter().map(|value| value.abs()).collect();
    magnitudes.sort_by(f64::total_cmp);
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let p95_slot = (count * 0.95).ceil() as usize;

    DeviationStats {
        #[allow(clippy::cast_precision_loss)]
        within_tolerance: inside as f64 / count,
        mean_abs,
        rms,
        median: values.get(values.len() / 2).copied().unwrap_or(0.0),
        p95: magnitudes
            .get(p95_slot.clamp(1, magnitudes.len()) - 1)
            .copied()
            .unwrap_or(0.0),
        measured,
        skipped,
    }
}

/// One RGBA colour per map entry, grey wherever there is no measurement.
#[must_use]
pub fn deviation_colors(map: &DeviationMap, ramp: &RampSettings) -> Vec<[u8; 4]> {
    map.signed_mm
        .iter()
        .zip(&map.validity)
        .map(|(value, state)| {
            if *state == Validity::Measured {
                ramp_color(f64::from(*value), ramp)
            } else {
                NO_DATA_COLOR
            }
        })
        .collect()
}

/// The colour for one measured deviation.
#[must_use]
pub fn ramp_color(value_mm: f64, ramp: &RampSettings) -> [u8; 4] {
    let scale = if ramp.scale_mm.is_finite() && ramp.scale_mm > 0.0 {
        ramp.scale_mm
    } else {
        1.0
    };
    let mut position = (value_mm / scale).clamp(-1.0, 1.0);
    if let Some(bands) = ramp.bands.filter(|count| *count > 0) {
        let steps = f64::from(bands);
        position = ((position * steps).floor() / steps).clamp(-1.0, 1.0);
    }
    let [red, green, blue] = sample_ramp(position);
    [red, green, blue, 255]
}

/// Linear interpolation through [`RAMP`].
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_ramp(position: f64) -> [u8; 3] {
    let mut low = RAMP[0];
    for stop in RAMP {
        if position >= stop.0 {
            low = stop;
        }
    }
    let Some(high) = RAMP.iter().copied().find(|stop| stop.0 > low.0) else {
        return low.1;
    };
    let span = high.0 - low.0;
    let blend = if span.abs() <= f64::EPSILON {
        0.0
    } else {
        ((position - low.0) / span).clamp(0.0, 1.0)
    };
    let mut out = [0u8; 3];
    for (slot, channel) in out.iter_mut().enumerate() {
        let start = f64::from(low.1[slot]);
        let end = f64::from(high.1[slot]);
        *channel = (start + (end - start) * blend).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        deviation, deviation_colors, deviation_stats, DeviationMap, DeviationSettings,
        RampSettings, Validity, NO_DATA_COLOR,
    };
    use crate::icp::Orientation;
    use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

    /// Two triangles forming a 10 x 10 sheet on z = 0, outward normal +Z.
    fn sheet() -> (Vec<f32>, Vec<u32>) {
        (
            vec![
                0.0, 0.0, 0.0, //
                10.0, 0.0, 0.0, //
                10.0, 10.0, 0.0, //
                0.0, 10.0, 0.0,
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn settings() -> DeviationSettings {
        DeviationSettings {
            influence_radius_mm: 3.0,
            orientation: Orientation::Match,
        }
    }

    fn lifted(positions: &[f32], by: f32) -> Vec<f32> {
        positions
            .chunks_exact(3)
            .flat_map(|point| [point[0], point[1], point[2] + by])
            .collect()
    }

    fn map_of(moving: &[f32], indices: &[u32], settings: &DeviationSettings) -> DeviationMap {
        let (fixed_positions, fixed_indices) = sheet();
        let index = SurfaceIndex::build(Soup {
            positions: &fixed_positions,
            indices: &fixed_indices,
            mask: None,
        })
        .unwrap();
        deviation(
            Soup {
                positions: moving,
                indices,
                mask: None,
            },
            &index,
            Rigid::IDENTITY,
            settings,
            &CancelFlag::new(),
        )
    }

    #[test]
    fn a_surface_above_the_target_reads_positive() {
        let (positions, indices) = sheet();
        let map = map_of(&lifted(&positions, 0.4), &indices, &settings());
        for (value, state) in map.signed_mm.iter().zip(&map.validity) {
            assert_eq!(*state, Validity::Measured);
            assert!((*value - 0.4).abs() < 1e-4, "expected +0.4, got {value}");
        }
    }

    #[test]
    fn a_surface_below_the_target_reads_negative() {
        let (positions, indices) = sheet();
        let map = map_of(&lifted(&positions, -0.25), &indices, &settings());
        assert!(map
            .signed_mm
            .iter()
            .all(|value| (*value + 0.25).abs() < 1e-4));
    }

    #[test]
    fn inverted_orientation_flips_every_sign() {
        let (positions, indices) = sheet();
        let inverted = DeviationSettings {
            orientation: Orientation::Inverted,
            ..settings()
        };
        let map = map_of(&lifted(&positions, 0.4), &indices, &inverted);
        assert!(map
            .signed_mm
            .iter()
            .all(|value| (*value + 0.4).abs() < 1e-4));
    }

    #[test]
    fn ignored_orientation_reports_magnitude_only() {
        let (positions, indices) = sheet();
        let ignored = DeviationSettings {
            orientation: Orientation::Ignored,
            ..settings()
        };
        let map = map_of(&lifted(&positions, -0.3), &indices, &ignored);
        assert!(map
            .signed_mm
            .iter()
            .all(|value| (*value - 0.3).abs() < 1e-4));
    }

    #[test]
    fn vertices_beyond_the_radius_are_out_of_reach_and_grey() {
        let (positions, indices) = sheet();
        let map = map_of(&lifted(&positions, 50.0), &indices, &settings());
        assert!(map
            .validity
            .iter()
            .all(|state| *state == Validity::OutOfReach));

        let stats = deviation_stats(&map, 0.2);
        assert_eq!(stats.measured, 0);
        assert_eq!(stats.skipped as usize, map.validity.len());

        let colors = deviation_colors(&map, &RampSettings::default());
        assert!(colors.iter().all(|color| *color == NO_DATA_COLOR));
    }

    #[test]
    fn statistics_count_only_measured_vertices() {
        let (positions, indices) = sheet();
        let mut moving = lifted(&positions, 0.1);
        moving[2] = 40.0;
        let map = map_of(&moving, &indices, &settings());

        let stats = deviation_stats(&map, 0.2);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.measured, 3);
        assert!((stats.within_tolerance - 1.0).abs() < 1e-9);
        assert!((stats.mean_abs - 0.1).abs() < 1e-4);
    }

    #[test]
    fn a_non_finite_vertex_is_named_as_such() {
        let (positions, indices) = sheet();
        let mut moving = lifted(&positions, 0.1);
        moving[0] = f32::NAN;
        let map = map_of(&moving, &indices, &settings());
        assert_eq!(map.validity[0], Validity::NonFinite);
    }

    #[test]
    fn the_ramp_is_blue_below_and_red_above() {
        let map = DeviationMap {
            signed_mm: vec![-0.5, 0.0, 0.5],
            validity: vec![Validity::Measured; 3],
        };
        let colors = deviation_colors(&map, &RampSettings::default());
        assert!(colors[0][2] > colors[0][0], "the negative end must be blue");
        assert!(colors[2][0] > colors[2][2], "the positive end must be red");
        assert!(
            colors[1][1] > colors[1][0] && colors[1][1] > colors[1][2],
            "zero must be green"
        );
    }

    #[test]
    fn banded_mode_quantizes_neighbouring_values_to_one_color() {
        let map = DeviationMap {
            signed_mm: vec![0.11, 0.13],
            validity: vec![Validity::Measured; 2],
        };
        let ramp = RampSettings {
            bands: Some(10),
            ..RampSettings::default()
        };
        let colors = deviation_colors(&map, &ramp);
        assert_eq!(colors[0], colors[1], "one band must be one colour");

        let continuous = deviation_colors(&map, &RampSettings::default());
        assert_ne!(
            continuous[0], continuous[1],
            "the continuous ramp must still separate them"
        );
    }

    #[test]
    fn a_cancelled_run_marks_everything_unmeasured() {
        let (positions, indices) = sheet();
        let (fixed_positions, fixed_indices) = sheet();
        let index = SurfaceIndex::build(Soup {
            positions: &fixed_positions,
            indices: &fixed_indices,
            mask: None,
        })
        .unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();

        let map = deviation(
            Soup {
                positions: &positions,
                indices: &indices,
                mask: None,
            },
            &index,
            Rigid::IDENTITY,
            &settings(),
            &cancel,
        );

        assert!(map
            .validity
            .iter()
            .all(|state| *state != Validity::Measured));
    }
}

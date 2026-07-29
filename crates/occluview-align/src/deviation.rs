//! PROOF: the signed deviation map, its statistics, and the ramp the viewport
//! paints it with.
//!
//! The sign lives in the measured value; the ramp only answers "how far". A
//! vertex with no fixed surface inside the influence radius is **not**
//! measured — it is marked and painted grey, because painting it at full scale
//! would be a lie, and letting it into the statistics would be a worse one.
//!
//! A vertex the operator painted out is treated the same way. Excluding a
//! region from the fit but still counting it in the numbers would report a
//! quality the operator explicitly said not to measure.
//!
//! # What this map measures, and what it does not
//!
//! Every value here is the distance from a moving vertex to the **nearest
//! point on the fixed surface**. That is a distance from a point to a *set*,
//! not the distance between two corresponding pieces of material, and the two
//! agree only where the fixed surface cannot slide onto itself. Two
//! consequences the operator must be told about, both measured on real arch
//! scans and on analytic primitives:
//!
//! 1. **Tangential blindness.** Displace two surfaces along a direction the
//!    fixed surface is smooth in and the nearest point simply slides: the
//!    reported value collapses towards zero while the material has genuinely
//!    moved. A 0.30 mm rigid offset of a real 945k-vertex arch reads as a mean
//!    of 0.14 mm — under half. On a cylinder displaced along its own axis it
//!    reads 0.0075 mm, and on a sphere turned about any diameter, 0.0008 mm.
//!    Measuring the other direction as well does **not** help; the surfaces
//!    really do coincide as point sets. [`crate::observability`] quantifies how
//!    much of a rigid displacement can hide, and is the number that must be
//!    reported next to these statistics.
//! 2. **One-sided blindness.** A moving scan missing a region reports a perfect
//!    fit over it, because the vertices that would have measured it are not
//!    there. Nothing derived from this map can see that, so what the map could
//!    not reach is counted and reported by cause — see [`Unmeasured`] — rather
//!    than folded into a single number that looks like a verdict.

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

/// Largest share of the display range the nominal band may occupy.
///
/// The band is the tolerance, and an operator can set a tolerance wider than
/// the range they are looking at. Left alone that paints the entire scan
/// nominal, which answers every question with "fine".
const NOMINAL_BAND_CEILING: f64 = 0.8;

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
    /// The operator painted this vertex out of the comparison.
    Excluded,
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

/// Why the vertices that carry no measurement do not.
///
/// They all paint the same grey, and that grey has three different meanings
/// behind it. Counted apart because the operator's next move differs for each:
/// the first is what they asked for, the second is anatomy — a bridge on one
/// arch with nothing opposite it has nothing to measure against, and no reach
/// will change that — and the third is a defect in the file itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unmeasured {
    /// Painted out of the comparison by the operator.
    pub excluded: u32,
    /// No fixed surface within the influence radius.
    pub out_of_reach: u32,
    /// The vertex, or the surface under it, is not usable data.
    pub unusable: u32,
}

impl Unmeasured {
    /// Every vertex that carries no measurement.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.excluded
            .saturating_add(self.out_of_reach)
            .saturating_add(self.unusable)
    }
}

/// Summary over the measured vertices only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviationStats {
    /// Vertices that carry a measurement.
    pub measured: u32,
    /// Vertices that do not, and why.
    ///
    /// The plain total used to live here on its own, and it could not tell "there
    /// is no tooth opposite this one" from "this file has broken vertices" —
    /// which is the whole difference between a normal result and a bad scan.
    pub unmeasured: Unmeasured,
    /// The numbers themselves — **absent** when there was not enough measured
    /// surface to characterise, which is a different answer from zero.
    ///
    /// This is an `Option` on purpose. A struct of zeroes returned for a
    /// measurement that never happened reads, in the one field a clinician
    /// looks at, as a perfect fit. Making the numbers unrepresentable in that
    /// case is the only way a reader cannot print one by accident.
    pub summary: Option<DeviationSummary>,
}

/// Fewest measured vertices that can characterise a surface.
///
/// A p95 taken over a handful of vertices is arithmetically true and clinically
/// meaningless. The same floor idea guards the ICP correspondence count and the
/// observability estimate; this is the deviation map's.
pub const MIN_MEASURED: u32 = 32;

/// What a measurement found, once there was enough of it to say anything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviationSummary {
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
    /// Largest absolute deviation, in millimetres — the directed Hausdorff
    /// distance from the measured moving vertices to the fixed surface.
    pub max_abs: f64,
}

/// Which colour scheme the map uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RampMode {
    /// Blue below, green at nominal, red above: the metrology convention, and
    /// the one lab software shows.
    ///
    /// The default, because it puts "the surfaces agree" in the MIDDLE of the
    /// ramp. A registration that worked lands near zero, and on a magnitude
    /// ramp that is the dead end of the scale — every good result comes out one
    /// flat blue, which reads as a broken tool rather than a clean fit.
    #[default]
    Signed,
    /// Blue at nothing, red at the display scale: magnitude only, for when
    /// which side a surface sits on is not the question.
    Magnitude,
}

/// How to turn measurements into colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampSettings {
    /// Deviation mapped to the ramp ends, in millimetres.
    pub scale_mm: f64,
    /// Tolerance band, in millimetres. Everything inside it is painted the
    /// ramp's nominal colour and the ramp only starts moving outside it — see
    /// [`ramp_color`]. It is also the band the statistics report.
    pub tolerance_mm: f64,
    /// Steps per side for a banded ramp; `None` is continuous. A stepped map
    /// shows where a boundary falls far more sharply than a smooth one.
    pub bands: Option<u32>,
    /// Which colour scheme to paint with.
    pub mode: RampMode,
}

/// The display scale that actually shows this measurement's structure.
///
/// A fixed scale is a guess about data nobody has measured yet: too wide and
/// every result is one flat colour at the ramp's centre, too narrow and
/// everything saturates. Derived from the 95th percentile so a handful of
/// outliers cannot stretch the range and flatten everything else, rounded up to
/// a readable step, and floored so a perfect fit still gets a sane bar.
#[must_use]
pub fn suggested_scale_mm(stats: &DeviationStats) -> f64 {
    /// Below this a scale stops meaning anything to an operator.
    const FLOOR_MM: f64 = 0.05;
    /// Readable steps, in millimetres. The range runs well past a clinical
    /// tolerance on purpose: a scan pair straight off a two-point fit really is
    /// millimetres apart, and capping the suggestion at a clinical number
    /// leaves every vertex pinned to an end stop — a saturated mosaic that says
    /// nothing about where the two actually differ.
    const STEPS: [f64; 13] = [
        0.05, 0.1, 0.15, 0.2, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0,
    ];

    let Some(summary) = stats.summary.filter(|summary| summary.p95.is_finite()) else {
        return FLOOR_MM;
    };
    let wanted = summary.p95.max(FLOOR_MM);
    STEPS
        .into_iter()
        .find(|step| *step >= wanted)
        .unwrap_or(10.0)
}

impl Default for RampSettings {
    fn default() -> Self {
        Self {
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            mode: RampMode::Signed,
        }
    }
}

/// Signed stops from `-1` to `+1`. Blue is undersize, green is nominal, red is
/// oversize — the convention every metrology package shares.
///
/// The stops keep a little headroom off the pure channel extremes so shading
/// has something to modulate — a blue whose red channel is exactly zero cannot
/// vary with the light at all, and the surface comes out as a flat coloured
/// silhouette with no readable form.
const SIGNED_RAMP: [(f64, [u8; 3]); 5] = [
    (-1.0, [20, 50, 235]),
    (-0.5, [20, 170, 235]),
    (0.0, [40, 200, 70]),
    (0.5, [250, 205, 20]),
    (1.0, [252, 30, 18]),
];

/// Magnitude stops from `0` to `1`: cool where the surfaces agree, hot where
/// they do not. This is the false-colour bar a lab operator reads without
/// having to work out which side of the surface a colour means.
///
/// Same stops as the signed ramp, folded onto one side — two ramps that
/// disagreed about what "0.2 mm out" looks like would make the mode switch
/// change the reading rather than the question.
const MAGNITUDE_RAMP: [(f64, [u8; 3]); 5] = [
    (0.0, [20, 50, 235]),
    (0.25, [20, 170, 235]),
    (0.5, [40, 200, 70]),
    (0.75, [250, 205, 20]),
    (1.0, [252, 30, 18]),
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
    if moving.is_excluded(vertex) {
        return (0.0, Validity::Excluded);
    }
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
///
/// These are **directed** statistics: moving vertices against fixed surface,
/// and nothing about fixed surface the moving scan never covered. They are also
/// a lower bound on the true displacement — see the module documentation for
/// the measured size of both effects. Report them alongside
/// [`crate::observability`] and the [`Unmeasured`] counts, never alone.
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
    let unmeasured = map
        .validity
        .iter()
        .fold(Unmeasured::default(), |mut count, state| {
            match state {
                Validity::Measured => {}
                Validity::Excluded => count.excluded = count.excluded.saturating_add(1),
                Validity::OutOfReach => count.out_of_reach = count.out_of_reach.saturating_add(1),
                Validity::DegenerateNormal | Validity::NonFinite => {
                    count.unusable = count.unusable.saturating_add(1);
                }
            }
            count
        });
    if measured < MIN_MEASURED {
        return DeviationStats {
            measured,
            unmeasured,
            summary: None,
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
        measured,
        unmeasured,
        summary: Some(DeviationSummary {
            #[allow(clippy::cast_precision_loss)]
            within_tolerance: inside as f64 / count,
            mean_abs,
            rms,
            median: values.get(values.len() / 2).copied().unwrap_or(0.0),
            p95: magnitudes
                .get(p95_slot.clamp(1, magnitudes.len()) - 1)
                .copied()
                .unwrap_or(0.0),
            max_abs: magnitudes.last().copied().unwrap_or(0.0),
        }),
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
///
/// The tolerance is a **flat nominal band**, not just a number in the summary.
/// Everything inside it lands on the ramp's nominal colour and the ramp only
/// starts moving outside it.
///
/// This is the difference between a heat map and a thermal camera. Without the
/// band every value gets its own hue, so scan noise at twenty microns paints a
/// full rainbow and a pair of arches that agree everywhere comes out as
/// speckle — the operator cannot tell "these match" from "these are all over
/// the place". With it, everything that is within tolerance is one colour, and
/// what is left burning is the part that genuinely differs.
#[must_use]
pub fn ramp_color(value_mm: f64, ramp: &RampSettings) -> [u8; 4] {
    let scale = if ramp.scale_mm.is_finite() && ramp.scale_mm > 0.0 {
        ramp.scale_mm
    } else {
        1.0
    };
    // Capped below the scale: a band as wide as the display range would paint
    // the whole scan nominal and answer every question with "fine".
    let band = if ramp.tolerance_mm.is_finite() && ramp.tolerance_mm > 0.0 {
        ramp.tolerance_mm.min(scale * NOMINAL_BAND_CEILING)
    } else {
        0.0
    };
    let span = (scale - band).max(f64::MIN_POSITIVE);
    let beyond = ((value_mm.abs() - band) / span).clamp(0.0, 1.0);
    let (ramp_stops, mut position) = match ramp.mode {
        RampMode::Magnitude => (&MAGNITUDE_RAMP, beyond),
        RampMode::Signed => (&SIGNED_RAMP, beyond.copysign(value_mm)),
    };
    if let Some(bands) = ramp.bands.filter(|count| *count > 0) {
        let quantum = f64::from(bands);
        position = ((position * quantum).floor() / quantum).clamp(-1.0, 1.0);
    }
    let [red, green, blue] = sample_ramp(ramp_stops, position);
    [red, green, blue, 255]
}

/// Linear interpolation through a stop table.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn sample_ramp(stops: &[(f64, [u8; 3]); 5], position: f64) -> [u8; 3] {
    let mut low = stops[0];
    for stop in *stops {
        if position >= stop.0 {
            low = stop;
        }
    }
    let Some(high) = stops.iter().copied().find(|stop| stop.0 > low.0) else {
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
#[path = "deviation_tests.rs"]
mod tests;

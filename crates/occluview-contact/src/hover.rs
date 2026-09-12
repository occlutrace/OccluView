//! Reading a value off the surface: what a hover reports, and how the readout
//! is spelled.
//!
//! A colour map without a number is a picture. The operator points at a mark to
//! find out how deep it is, and the answer has to be in the units a technician
//! writes: micrometres below a millimetre, millimetres above it. `82 µm` reads;
//! `0.08 mm` does not.
//!
//! The value under the pointer is interpolated from the three corners of the
//! triangle it landed on, exactly as the GPU interpolates the field before it
//! looks the colour up. Reading a corner instead would report a number up to
//! one vertex spacing away from the mark the operator is pointing at, and the
//! colour under the pointer and the number beside it would disagree.

use crate::{lut::FIELD_FAR_SENTINEL_MM, PENETRATION_EPS_MM};

/// Which side of touch a reading sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactReadingKind {
    /// The surfaces are apart; there is no load here.
    Gap,
    /// The surfaces overlap; this is where the bite presses.
    Penetration,
}

/// One hover reading: which side of touch, and how far from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactReading {
    /// Gap or penetration.
    pub kind: ContactReadingKind,
    /// Distance from touch, in millimetres, always unsigned.
    pub magnitude_mm: f64,
}

impl ContactReading {
    /// The reading at a signed field value, or `None` when nothing was measured.
    ///
    /// A vertex that found no opposing surface has no reading to report. It is
    /// not rounded to zero and not marked as a deep contact: the operator is
    /// told the number is missing instead of being handed a plausible one.
    pub fn from_signed_mm(signed_mm: f32) -> Option<Self> {
        if !signed_mm.is_finite() {
            return None;
        }
        let magnitude_mm = f64::from(signed_mm.abs());
        if signed_mm < -PENETRATION_EPS_MM {
            Some(Self {
                kind: ContactReadingKind::Penetration,
                magnitude_mm,
            })
        } else {
            // A sub-micron interference reads as touch, the same dead-band the
            // field itself applies — the reading and the pixels agree.
            Some(Self {
                kind: ContactReadingKind::Gap,
                magnitude_mm,
            })
        }
    }
}

/// A distance from touch, spelled the way a technician writes it.
///
/// Micrometres below a millimetre, millimetres above it, and never a bare
/// number: a reading that is not told its unit will be read in one.
pub fn format_contact_value(magnitude_mm: f64) -> String {
    if !magnitude_mm.is_finite() {
        return "-- µm".to_owned();
    }
    if magnitude_mm < 0.9995 {
        format!("{} µm", (magnitude_mm * 1000.0).round())
    } else {
        format!("{magnitude_mm:.2} mm")
    }
}

/// The signed field value at a point inside a triangle, or `None` when nothing
/// there was measured.
///
/// `barycentric` are the three corner weights the ray hit reported, in the same
/// order as the triangle's indices. A corner with no measurement enters the
/// blend as the finite far sentinel rather than as infinity, so one unmeasured
/// corner cannot poison the whole triangle with a NaN: near the edge of a
/// contact the honest answer is a value between "measured here" and "nothing
/// found nearby", and that is what the paint shows too. A triangle whose
/// corners were ALL unmeasured has no reading at all and says so, rather than
/// reporting the sentinel as if it were a distance.
pub fn interpolate_field_at_triangle(
    values: &[f32],
    indices: &[u32],
    triangle: usize,
    barycentric: [f64; 3],
) -> Option<f32> {
    let corners = indices.get(triangle * 3..triangle * 3 + 3)?;
    let mut sum = 0.0_f64;
    let mut total = 0.0_f64;
    let mut measured = false;
    for (corner, weight) in corners.iter().zip(barycentric) {
        if !weight.is_finite() || weight == 0.0 {
            continue;
        }
        let vertex = usize::try_from(*corner).ok()?;
        let Some(&value) = values.get(vertex) else {
            continue;
        };
        let finite = if value.is_finite() {
            measured = true;
            value
        } else {
            FIELD_FAR_SENTINEL_MM
        };
        sum += weight * f64::from(finite);
        total += weight;
    }
    if !measured || total <= 0.0 {
        return None;
    }
    let blended = sum / total;
    if !blended.is_finite() {
        return None;
    }
    Some(to_f32(blended))
}

/// Whether a value is the "no opposing surface" sentinel.
///
/// A NaN counts as no contact too: it is what a hostile or corrupt buffer
/// produces, and it must read as "not measured" rather than as zero. Exposed so
/// a caller never has to compare a float for equality itself; the crate owns
/// the spelling of "nothing here".
pub fn is_no_contact(signed_mm: f32) -> bool {
    !signed_mm.is_finite()
}

/// A blended value narrowed back to the field's own precision.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

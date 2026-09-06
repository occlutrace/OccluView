//! Units of measure.
//!
//! OccluView works in **millimeters** internally. All public APIs use
//! a unit newtype rather than a bare `f32`, so units cannot be silently
//! confused. Conversion from format-native units lives in `occluview-formats`.

use core::fmt;
use core::ops::{Add, Sub};

/// A length expressed in millimeters — OccluView's canonical length unit.
///
/// Arithmetic works on the underlying value; multiplying two lengths to get an
/// area is intentionally not provided (no dimension errors hiding in `f32`).
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub struct Millimeters(pub f32);

impl Millimeters {
    /// The zero length.
    pub const ZERO: Self = Self(0.0);

    /// Construct from a millimeter value.
    #[inline]
    #[must_use]
    pub const fn new(mm: f32) -> Self {
        Self(mm)
    }

    /// Construct from meters (1 m = 1000 mm). glTF declares meters.
    #[inline]
    #[must_use]
    pub fn from_meters(m: f32) -> Self {
        Self(m * 1000.0)
    }

    /// Construct from inches (3MF sometimes declares inches).
    #[inline]
    #[must_use]
    pub fn from_inches(inch: f32) -> Self {
        Self(inch * 25.4)
    }

    /// Return the value in millimeters.
    #[inline]
    #[must_use]
    pub const fn as_mm(self) -> f32 {
        self.0
    }
}

impl Add for Millimeters {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Millimeters {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl fmt::Display for Millimeters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 3 significant decimals in mm is ~micron precision — plenty for dental.
        write!(f, "{:.3} mm", self.0)
    }
}

/// Unit semantics a source format declares (or fails to declare) for its
/// coordinates. OccluView renders everything in [`Millimeters`]; this type
/// records what the file *meant* so the import scale is explicit metadata
/// instead of tribal knowledge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SourceUnit {
    /// Coordinates are already millimeters.
    Millimeters,
    /// Coordinates are meters (glTF 2.0 declares meters).
    Meters,
    /// The format declares no unit (STL, OBJ, PLY, OFF, HPS).
    Unitless,
}

/// How much the [`UnitInterpretation::scale_to_mm`] factor can be trusted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnitConfidence {
    /// The file declares its unit and we honor it.
    Certain,
    /// The format declares nothing; millimeters are assumed (v1 policy for
    /// STL/OBJ/PLY/OFF/HPS — scanner exports in the wild are millimeter
    /// numbers, and guessing otherwise would corrupt real cases).
    AssumedMillimeters,
    /// The declaration and the observed data disagree in practice (glTF
    /// declares meters, but scanner exporters write millimeter numbers), so
    /// no scale is applied and the operator must confirm the interpretation.
    Ambiguous,
}

/// Import-unit metadata carried per layer: what the file declared, what
/// scale brings it to millimeters, and whether that scale is trustworthy.
/// Coordinates are normalized to millimeters exactly once, at import; a
/// factor of `1.0` with anything but `Certain` confidence means "kept as-is,
/// flagged", never "verified".
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UnitInterpretation {
    /// What the source format declares.
    pub declared: SourceUnit,
    /// Multiply file coordinates by this to get millimeters.
    pub scale_to_mm: f32,
    /// Whether the factor above is trustworthy.
    pub confidence: UnitConfidence,
}

impl UnitInterpretation {
    /// Unitless formats read as millimeter numbers (v1 policy).
    #[inline]
    #[must_use]
    pub const fn assumed_millimeters() -> Self {
        Self {
            declared: SourceUnit::Unitless,
            scale_to_mm: 1.0,
            confidence: UnitConfidence::AssumedMillimeters,
        }
    }

    /// glTF declares meters, but scanner GLBs in practice carry millimeter
    /// numbers, so v1 keeps coordinates unchanged and flags the layer
    /// ambiguous instead of silently scaling either way.
    #[inline]
    #[must_use]
    pub const fn ambiguous_gltf() -> Self {
        Self {
            declared: SourceUnit::Meters,
            scale_to_mm: 1.0,
            confidence: UnitConfidence::Ambiguous,
        }
    }
}

impl fmt::Display for UnitInterpretation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.declared, self.confidence) {
            (SourceUnit::Millimeters, UnitConfidence::Certain) => write!(f, "millimeters"),
            (_, UnitConfidence::AssumedMillimeters) => {
                write!(f, "millimeters (assumed; unitless format)")
            }
            (_, UnitConfidence::Ambiguous) => write!(
                f,
                "ambiguous: declares {:?}, coordinates kept as-is",
                self.declared
            ),
            (declared, UnitConfidence::Certain) => {
                write!(f, "{:?} (x{} to mm)", declared, self.scale_to_mm)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_as_mm_roundtrip() {
        assert_eq!(Millimeters::new(12.5).as_mm(), 12.5);
    }

    #[test]
    fn from_meters_converts_correctly() {
        assert!((Millimeters::from_meters(1.0).as_mm() - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn from_inches_converts_correctly() {
        assert!((Millimeters::from_inches(1.0).as_mm() - 25.4).abs() < 1e-3);
    }

    #[test]
    fn add_sub_are_linear() {
        let a = Millimeters::new(10.0);
        let b = Millimeters::new(3.0);
        assert_eq!((a + b).as_mm(), 13.0);
        assert_eq!((a - b).as_mm(), 7.0);
    }

    #[test]
    fn zero_is_identity() {
        assert_eq!(Millimeters::ZERO.as_mm(), 0.0);
    }

    #[test]
    fn display_is_millimetric() {
        assert_eq!(format!("{}", Millimeters::new(0.5)), "0.500 mm");
    }

    #[test]
    fn assumed_millimeters_keeps_coordinates() {
        let policy = UnitInterpretation::assumed_millimeters();
        assert_eq!(policy.scale_to_mm, 1.0);
        assert_eq!(policy.confidence, UnitConfidence::AssumedMillimeters);
        assert_eq!(
            format!("{policy}"),
            "millimeters (assumed; unitless format)"
        );
    }

    #[test]
    fn ambiguous_gltf_keeps_coordinates_and_says_so() {
        let policy = UnitInterpretation::ambiguous_gltf();
        assert_eq!(policy.declared, SourceUnit::Meters);
        assert_eq!(policy.scale_to_mm, 1.0);
        assert_eq!(policy.confidence, UnitConfidence::Ambiguous);
        assert!(format!("{policy}").starts_with("ambiguous:"));
    }
}

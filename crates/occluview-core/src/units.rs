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
}

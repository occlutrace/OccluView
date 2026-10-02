use glam::DVec3;

/// The tip family used by live sculpt strokes and their surface feedback.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipStamp {
    /// Spherical falloff.
    Ball = 0,
    /// Narrow, travel-aligned edge with a blended transverse shoulder.
    Knife = 1,
    /// Flat-ended stamp with a blended rim.
    Cylinder = 2,
}

impl TipStamp {
    /// Decode a wire tag, or return `None` for an unknown value.
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Ball),
            1 => Some(Self::Knife),
            2 => Some(Self::Cylinder),
            _ => None,
        }
    }
}

/// Cylinder plateau as a share of the brush radius.
pub const CYLINDER_PLATEAU: f64 = 0.8;
/// Transverse knife reach relative to its along-stroke radius.
pub const KNIFE_CROSS_RADIUS_SHARE: f64 = 0.55;
/// Minimum stroke-axis length that still defines a knife bearing.
pub const KNIFE_AXIS_MIN_LENGTH: f64 = 1e-12;

/// Spherical stamp falloff: `t^2` for `t = 1 - d/r`.
#[must_use]
pub fn ball_weight(distance_mm: f64, radius_mm: f64) -> f64 {
    if distance_mm >= radius_mm
        || radius_mm <= 0.0
        || !distance_mm.is_finite()
        || !radius_mm.is_finite()
    {
        return 0.0;
    }
    let t = (1.0 - distance_mm / radius_mm).clamp(0.0, 1.0);
    t * t
}

/// Oriented knife edge. The radius is its along-stroke reach.
#[must_use]
pub fn knife_weight(offset: DVec3, axis: Option<DVec3>, radius_mm: f64) -> f64 {
    if !offset.is_finite() {
        return 0.0;
    }
    let cross_share = KNIFE_CROSS_RADIUS_SHARE;
    let minimum_axis_length = KNIFE_AXIS_MIN_LENGTH;
    let Some(axis) = axis.filter(|value| value.is_finite() && value.length() > minimum_axis_length)
    else {
        return ball_weight(offset.length(), radius_mm * cross_share.sqrt());
    };
    let axis_length = axis.length();
    if !axis_length.is_finite() {
        return ball_weight(offset.length(), radius_mm * cross_share.sqrt());
    }
    let unit = axis * (1.0 / axis_length);
    let along = offset.dot(unit);
    let across = (offset - unit * along).length();
    ball_weight(along.hypot(across / cross_share), radius_mm)
}

/// Flat-ended stamp: a plateau, then a smoothstep rim to zero.
#[must_use]
pub fn cylinder_weight(distance_mm: f64, radius_mm: f64) -> f64 {
    if distance_mm >= radius_mm
        || radius_mm <= 0.0
        || !distance_mm.is_finite()
        || !radius_mm.is_finite()
    {
        return 0.0;
    }
    let plateau = radius_mm * CYLINDER_PLATEAU;
    if distance_mm <= plateau {
        return 1.0;
    }
    let s = (1.0 - (distance_mm - plateau) / (radius_mm - plateau)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

/// The shared point field for a sculpt cursor and its dab kernel.
#[must_use]
pub fn stamp_weight(
    tip: TipStamp,
    offset: DVec3,
    distance_mm: f64,
    axis: Option<DVec3>,
    radius_mm: f64,
) -> f64 {
    match tip {
        TipStamp::Ball => ball_weight(distance_mm, radius_mm),
        TipStamp::Knife => knife_weight(offset, axis, radius_mm),
        TipStamp::Cylinder => cylinder_weight(distance_mm, radius_mm),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knife_with_overflowing_axis_length_uses_ball_fallback() {
        let offset = DVec3::X;
        let radius = 4.0;
        let expected = ball_weight(offset.length(), radius * KNIFE_CROSS_RADIUS_SHARE.sqrt());
        let actual = knife_weight(offset, Some(DVec3::splat(1.0e308)), radius);
        assert!(
            (actual - expected).abs() < 1.0e-12,
            "{actual} != {expected}"
        );
    }
}

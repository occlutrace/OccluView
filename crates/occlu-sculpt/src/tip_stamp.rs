//! Brush tip stamps — the shape of a dab, shared by every sculpt session.
//!
//! A tip is geometry, not a falloff curve: it decides which vertices a dab
//! reaches and with what share of the stroke. A mode (add / smooth / flatten)
//! uses one tip (ball / knife / cylinder), so each tip has one shared footprint.
//!
//! The knife follows the supplied stroke bearing; a press with no valid
//! bearing uses a narrow radial footprint instead of producing an empty dab.
//! Sessions own everything around the stamp — selection, protection, undo,
//! safety clamps — and only call into here for the per-vertex share.

use glam::DVec3;

/// The tip family. The discriminants are part of the wire contract: a stored
/// session or a payload names a tip by number, never by declaration order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipStamp {
    /// Spherical falloff: the default shaping stamp.
    Ball = 0,
    /// Narrow, travel-aligned edge with a blended transverse shoulder.
    Knife = 1,
    /// Flat-ended stamp: uniform displacement inside a plateau with a short
    /// blended rim, so a dab levels one face instead of raising a mound.
    Cylinder = 2,
}

impl TipStamp {
    /// The tip a wire payload names, or `None` for an unknown value.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Ball),
            1 => Some(Self::Knife),
            2 => Some(Self::Cylinder),
            _ => None,
        }
    }
}

/// Cylinder plateau as a share of the brush radius. The remaining fifth of the
/// radius forms the blended rim.
pub const CYLINDER_PLATEAU: f64 = 0.8;
/// Transverse knife reach relative to its along-stroke radius.
const KNIFE_CROSS_RADIUS_SHARE: f64 = 0.55;

/// Spherical stamp falloff: `t^2` for `t = 1 - d/r`. It is one at the centre
/// and reaches zero with zero slope at the radius.
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

/// Oriented knife edge. The radius is its along-stroke reach; the transverse
/// reach is wide enough to engage several edges on an ordinary prepared mesh
/// while remaining narrower than the ball. The squared taper holds the rim.
pub fn knife_weight(offset: DVec3, axis: Option<DVec3>, radius_mm: f64) -> f64 {
    if !offset.x.is_finite() || !offset.y.is_finite() || !offset.z.is_finite() {
        return 0.0;
    }
    let press_radius = radius_mm * KNIFE_CROSS_RADIUS_SHARE.sqrt();
    let Some(axis) = axis.filter(|a| a.x.is_finite() && a.y.is_finite() && a.z.is_finite()) else {
        return ball_weight(offset.length(), press_radius);
    };
    let length = axis.length();
    if !length.is_finite() || length <= 1e-12 {
        return ball_weight(offset.length(), press_radius);
    }
    let along = offset.dot(axis * (1.0 / length));
    let across = (offset - (axis * (along / length))).length();
    let elliptical_distance = along.hypot(across / KNIFE_CROSS_RADIUS_SHARE);
    ball_weight(elliptical_distance, radius_mm)
}

/// Flat-ended stamp: uniform inside the plateau, then a smoothstep rim to zero.
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

/// Gauss-Legendre nodes and weights on [-1, 1]. Each stamp is smooth on the
/// pieces the integral below is split into, so five points reproduce it far
/// below any visible dose.
const GAUSS_NODES: [f64; 5] = [
    -0.906_179_845_938_664,
    -0.538_469_310_105_683,
    0.0,
    0.538_469_310_105_683,
    0.906_179_845_938_664,
];
const GAUSS_WEIGHTS: [f64; 5] = [
    0.236_926_885_056_189,
    0.478_628_670_499_366,
    0.568_888_888_888_889,
    0.478_628_670_499_366,
    0.236_926_885_056_189,
];

/// The stamp swept along a straight segment of travel: the integral of the
/// tip's share over the segment, in millimetres of travel. `offset` runs from
/// the segment start to the vertex; `segment` from start to end. The knife
/// takes the segment as its bearing.
///
/// Every tip is a function of the along-travel offset and one fixed cross
/// distance (radial for the ball and the cylinder, elliptical for the knife),
/// so its support on the segment is one chord, found in closed form. The
/// cylinder's plateau edge splits that chord so each piece is smooth.
pub fn segment_stamp_weight(tip: TipStamp, offset: DVec3, segment: DVec3, radius_mm: f64) -> f64 {
    let length = segment.length();
    if !(length > 1e-12) || !(radius_mm > 0.0) || !offset.is_finite() || !length.is_finite() {
        return 0.0;
    }
    let axis = segment * (1.0 / length);
    let along = offset.dot(axis);
    let across = (offset - axis * along).length();
    let cross = match tip {
        TipStamp::Knife => across / KNIFE_CROSS_RADIUS_SHARE,
        TipStamp::Ball | TipStamp::Cylinder => across,
    };
    if !(cross < radius_mm) {
        return 0.0;
    }
    let half = (radius_mm * radius_mm - cross * cross).sqrt();
    let (lo, hi) = ((along - half).max(0.0), (along + half).min(length));
    if !(hi > lo) {
        return 0.0;
    }
    let profile = |t: f64| {
        let distance = (t - along).hypot(cross);
        match tip {
            TipStamp::Ball | TipStamp::Knife => ball_weight(distance, radius_mm),
            TipStamp::Cylinder => cylinder_weight(distance, radius_mm),
        }
    };
    let mut cuts = [lo, hi, hi, hi];
    let mut count = 1;
    if tip == TipStamp::Cylinder {
        let plateau = radius_mm * CYLINDER_PLATEAU;
        if cross < plateau {
            let inner = (plateau * plateau - cross * cross).sqrt();
            for edge in [along - inner, along + inner] {
                if edge > lo && edge < hi {
                    cuts[count] = edge;
                    count += 1;
                }
            }
        }
    }
    cuts[count] = hi;
    count += 1;
    let mut total = 0.0;
    for piece in cuts[..count].windows(2) {
        let (a, b) = (piece[0], piece[1]);
        let (middle, half_width) = (f64::midpoint(a, b), (b - a) * 0.5);
        for (node, weight) in GAUSS_NODES.iter().zip(GAUSS_WEIGHTS) {
            total += weight * profile(middle + half_width * node) * half_width;
        }
    }
    total
}

/// One entry point for sessions: the tip's share of a dab for a vertex at
/// `offset` from the dab centre (`distance_mm` is its length, passed so the
/// radial tips skip a second norm). Only the knife uses the stroke axis.
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

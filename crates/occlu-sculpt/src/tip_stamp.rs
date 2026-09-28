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

use crate::{ball_weight, cylinder_weight, TipStamp, CYLINDER_PLATEAU, KNIFE_CROSS_RADIUS_SHARE};
use glam::DVec3;

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
        TipStamp::Knife => across / f64::from(KNIFE_CROSS_RADIUS_SHARE),
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
        let plateau = radius_mm * f64::from(CYLINDER_PLATEAU);
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

//! Shape-preserving influence band outside a brush selection, shared by sessions.
//!
//! A dab that ends at the selection rim steps: full motion inside, none
//! outside, with a crease along the edge loop. The Ctrl-style answer is a
//! blend band N rings wide where the preserved share of the original shape
//! rises from 0 at the rim to 1 where the surface is fully held. The band is
//! an alpha FIELD, not a mesh edit — the session multiplies its own proposal
//! by (1 − alpha) — so this module measures ring distance and shapes the ramp
//! and owns nothing else.
//!
//! The ramp is a smoothstep, not linear: zero slope at both ends meets full
//! motion and full hold without a crease, the same reason a levelling stamp
//! blends its rim instead of stepping it.

/// Blend width in rings when the operator does not name one. One ring is no
/// blend at all — the ramp has nowhere to go and the crease it exists to
/// remove stays. Past a handful the brush stops answering the hand, because
/// the band reaches further than the dab's mechanical influence. Three spans
/// the fairing's own single held rim with room to breathe on each side.
pub const DEFAULT_PRESERVE_RINGS: usize = 3;

/// Preserved share of the original shape `rings_out` rings outside the
/// selection edge: 0 at the rim (moves with the brush), 1 at `falloff_rings`
/// out and beyond (fully held). Ring 0 names the selected vertices
/// themselves, which always move fully. A zero falloff means no band: outside
/// is held immediately. Integer inputs admit no NaN, so the answer is finite
/// by construction.
pub fn preserve_alpha(rings_out: u32, falloff_rings: usize) -> f64 {
    if rings_out == 0 {
        return 0.0;
    }
    if falloff_rings == 0 {
        return 1.0;
    }
    let t = (rings_out as f64 / falloff_rings as f64).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

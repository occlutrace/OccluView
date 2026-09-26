//! Import-unit policy: which scale brings each format to millimeters.
//!
//! OccluView works in millimeters ([`Millimeters`][occluview_core::units::Millimeters]).
//! Unitless formats (STL, OBJ, PLY, OFF, HPS) are read as millimeter numbers
//! — that matches what scanner exporters write, and guessing otherwise would
//! corrupt real cases. glTF declares meters, but scanner GLBs in practice
//! carry millimeter numbers, so coordinates are kept as-is and the layer is
//! flagged [`UnitConfidence::Ambiguous`][occluview_core::units::UnitConfidence]
//! instead of being silently scaled either way. [`recommend_glb_scale`]
//! suggests an interpretation from the bounding box for operator confirmation;
//! the suggestion is never applied without one — the app surfaces it in the
//! status line when an ambiguous layer loads, which is the confirmation step
//! this paragraph describes.

use crate::probe::FormatKind;
pub use occluview_core::units::UnitInterpretation;

/// Import-unit interpretation for a detected format kind. Pure policy table:
/// unitless formats assume millimeters, glTF stays ambiguous (meters declared,
/// millimeter numbers observed).
#[inline]
#[must_use]
pub fn policy_for(kind: FormatKind) -> UnitInterpretation {
    match kind {
        FormatKind::Stl
        | FormatKind::Ply
        | FormatKind::Obj
        | FormatKind::Off
        | FormatKind::Hps
        | FormatKind::Threemf => UnitInterpretation::assumed_millimeters(),
        FormatKind::Gltf => UnitInterpretation::ambiguous_gltf(),
    }
}

/// Bounding-box-based scale suggestion for an ambiguous GLB layer.
///
/// `max_extent` is the largest bounding-box dimension in raw file units. A
/// full dental arch spans roughly 50–150 mm (0.05–0.15 m); even a single
/// crown is ~10 mm (0.01 m). Anything below one file unit is therefore
/// almost certainly spec-compliant meters, anything above ten is almost
/// certainly millimeter numbers. The middle band — and any degenerate box —
/// stays [`GlbScaleRecommendation::Unclear`]: a wrong silent scale corrupts
/// measurements, ruler, brush physics, and deviation maps, so unclear means
/// "ask the operator", never "guess".
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GlbScaleRecommendation {
    /// Raw units read as meters: multiply coordinates by 1000.
    MetersToMillimeters,
    /// Raw units read as millimeters: keep coordinates.
    KeepAsMillimeters,
    /// Cannot tell from the bounding box: ask the operator.
    Unclear,
}

/// Suggest a GLB scale from the largest bounding-box dimension in raw file
/// units. Advisory only — callers must not apply the result silently.
#[inline]
#[must_use]
pub fn recommend_glb_scale(max_extent: f32) -> GlbScaleRecommendation {
    if !max_extent.is_finite() || max_extent <= 0.0 {
        return GlbScaleRecommendation::Unclear;
    }
    if max_extent < 1.0 {
        GlbScaleRecommendation::MetersToMillimeters
    } else if max_extent > 10.0 {
        GlbScaleRecommendation::KeepAsMillimeters
    } else {
        GlbScaleRecommendation::Unclear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::units::{SourceUnit, UnitConfidence};

    #[test]
    fn unitless_formats_assume_millimeters() {
        for kind in [
            FormatKind::Stl,
            FormatKind::Ply,
            FormatKind::Obj,
            FormatKind::Off,
            FormatKind::Hps,
            FormatKind::Threemf,
        ] {
            let policy = policy_for(kind);
            assert_eq!(policy.declared, SourceUnit::Unitless, "{kind:?}");
            assert_eq!(policy.scale_to_mm, 1.0, "{kind:?}");
            assert_eq!(
                policy.confidence,
                UnitConfidence::AssumedMillimeters,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn gltf_stays_ambiguous_without_silent_scaling() {
        let policy = policy_for(FormatKind::Gltf);
        assert_eq!(policy.declared, SourceUnit::Meters);
        assert_eq!(policy.scale_to_mm, 1.0);
        assert_eq!(policy.confidence, UnitConfidence::Ambiguous);
    }

    #[test]
    fn tiny_boxes_recommend_meters() {
        // Full arch in meters (~0.06–0.15) and a single crown (~0.01).
        assert_eq!(
            recommend_glb_scale(0.12),
            GlbScaleRecommendation::MetersToMillimeters
        );
        assert_eq!(
            recommend_glb_scale(0.01),
            GlbScaleRecommendation::MetersToMillimeters
        );
    }

    #[test]
    fn large_boxes_keep_millimeters() {
        // Full arch in millimeters (~50–150) and a single crown (~10+).
        assert_eq!(
            recommend_glb_scale(80.0),
            GlbScaleRecommendation::KeepAsMillimeters
        );
        assert_eq!(
            recommend_glb_scale(10.01),
            GlbScaleRecommendation::KeepAsMillimeters
        );
    }

    #[test]
    fn middle_band_and_degenerate_boxes_stay_unclear() {
        for extent in [0.0, -5.0, f32::NAN, f32::INFINITY, 1.0, 5.0, 10.0] {
            assert_eq!(
                recommend_glb_scale(extent),
                GlbScaleRecommendation::Unclear,
                "extent {extent}"
            );
        }
    }
}

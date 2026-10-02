//! Scale-bar math for rendered mesh views.

use crate::i18n::catalog::NumberFormat;

/// A screen-space scale bar chosen for a mesh view.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScaleBar {
    /// Physical length represented by the bar, in millimeters.
    pub length_mm: f32,
    /// On-screen bar width, in pixels.
    pub width_px: f32,
}

impl ScaleBar {
    /// Pick a readable millimetre scale bar for a view at this scale.
    ///
    /// Takes the scale itself rather than the scene's size, because the two are
    /// only equal for one instant: right after Fit view. A bar derived from the
    /// mesh's bounding box over the viewport width would keep describing the
    /// load-time framing however far the operator zoomed.
    #[must_use]
    pub fn for_mm_per_px(mm_per_px: f32) -> Option<Self> {
        if !mm_per_px.is_finite() || mm_per_px <= 0.0 {
            return None;
        }

        let length_mm = nice_length_mm(mm_per_px * 120.0);
        let width_px = length_mm / mm_per_px;
        if !width_px.is_finite() || width_px <= 0.0 {
            return None;
        }

        Some(Self {
            length_mm,
            width_px,
        })
    }

    /// Label text for the UI, in the operator's chosen unit.
    #[must_use]
    pub fn label(
        self,
        unit: crate::app_settings::UnitDisplay,
        number_format: NumberFormat,
    ) -> String {
        match unit {
            crate::app_settings::UnitDisplay::Millimeters => {
                let length = f64::from(self.length_mm);
                let mm = number_format.decimal(length, length_precision(length, 0));
                format!("{mm} mm")
            }
            crate::app_settings::UnitDisplay::Inches => {
                let length = f64::from(self.length_mm) / 25.4;
                let inches = number_format.decimal(length, length_precision(length, 2));
                format!("{inches} in")
            }
        }
    }
}

fn length_precision(length: f64, minimum: usize) -> usize {
    let mut digits = minimum;
    let mut scaled = length.abs();
    for _ in 0..minimum {
        scaled *= 10.0;
    }
    while scaled.is_finite() && scaled > 0.0 && scaled < 1.0 {
        digits += 1;
        scaled *= 10.0;
    }
    digits
}

fn nice_length_mm(target_mm: f32) -> f32 {
    let magnitude = 10.0_f32.powf(target_mm.log10().floor());
    let normalized = target_mm / magnitude;
    let nice = if normalized < 1.5 {
        1.0
    } else if normalized < 3.5 {
        2.0
    } else if normalized < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    /// Zooming changes the bar: it follows the view scale, not the framing the
    /// scene had when the file opened.
    #[test]
    fn a_closer_view_puts_fewer_millimetres_in_the_bar() {
        let wide = ScaleBar::for_mm_per_px(80.0 / 512.0).expect("a wide view");
        let close = ScaleBar::for_mm_per_px(8.0 / 512.0).expect("the same view, zoomed in");
        assert!(
            close.length_mm < wide.length_mm,
            "zooming in by ten did not shorten the bar: {} then {}",
            wide.length_mm,
            close.length_mm
        );
        for bar in [wide, close] {
            assert!(
                bar.width_px > 0.0 && bar.width_px.is_finite(),
                "a bar has to be drawable: {bar:?}"
            );
        }
    }

    #[test]
    fn picks_readable_bar_for_typical_arch_width() {
        let bar = ScaleBar::for_mm_per_px(80.0 / 512.0).unwrap();

        assert_eq!(bar.length_mm, 20.0);
        assert!((bar.width_px - 128.0).abs() < 0.01);
        assert_eq!(
            bar.label(
                crate::app_settings::UnitDisplay::Millimeters,
                NumberFormat::for_tag("en"),
            ),
            "20 mm"
        );
        assert_eq!(
            bar.label(
                crate::app_settings::UnitDisplay::Inches,
                NumberFormat::for_tag("en"),
            ),
            "0.79 in"
        );
    }

    #[test]
    fn returns_none_for_invalid_dimensions() {
        assert!(ScaleBar::for_mm_per_px(0.0).is_none());
        assert!(ScaleBar::for_mm_per_px(f32::INFINITY).is_none());
        assert!(ScaleBar::for_mm_per_px(f32::NAN).is_none());
    }

    #[test]
    fn keeps_small_scenes_in_millimeters() {
        let bar = ScaleBar::for_mm_per_px(4.0 / 512.0).unwrap();

        assert_eq!(bar.length_mm, 1.0);
        assert!((bar.width_px - 128.0).abs() < 0.01);
        assert_eq!(
            bar.label(
                crate::app_settings::UnitDisplay::Millimeters,
                NumberFormat::for_tag("en"),
            ),
            "1 mm"
        );
    }

    #[test]
    fn zoomed_scale_bar_keeps_nonzero_lengths_in_both_units_and_locales() {
        let bar = ScaleBar::for_mm_per_px(0.5 / 512.0).expect("close view");
        assert_eq!(bar.length_mm, 0.1);
        for (tag, mm, inches) in [("en", "0.1 mm", "0.004 in"), ("de", "0,1 mm", "0,004 in")] {
            assert_eq!(
                bar.label(
                    crate::app_settings::UnitDisplay::Millimeters,
                    NumberFormat::for_tag(tag)
                ),
                mm
            );
            assert_eq!(
                bar.label(
                    crate::app_settings::UnitDisplay::Inches,
                    NumberFormat::for_tag(tag)
                ),
                inches
            );
        }
    }

    #[test]
    fn rounds_large_scenes_to_nice_lengths() {
        let bar = ScaleBar::for_mm_per_px(500.0 / 512.0).unwrap();

        assert_eq!(bar.length_mm, 100.0);
        assert!((bar.width_px - 102.4).abs() < 0.01);
        assert_eq!(
            bar.label(
                crate::app_settings::UnitDisplay::Millimeters,
                NumberFormat::for_tag("en"),
            ),
            "100 mm"
        );
    }
}

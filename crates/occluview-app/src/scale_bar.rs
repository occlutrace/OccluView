//! Scale-bar math for rendered mesh views.

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
    /// only equal for one instant: right after Fit view. This used to be derived
    /// from the mesh's bounding box over the viewport width, which meant the bar
    /// described the framing the scene had when it loaded and went on describing
    /// it for the rest of the session, however far the operator zoomed.
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
    pub fn label(self, unit: crate::app_settings::UnitDisplay) -> String {
        match unit {
            crate::app_settings::UnitDisplay::Millimeters => format!("{:.0} mm", self.length_mm),
            crate::app_settings::UnitDisplay::Inches => {
                format!("{:.2} in", f64::from(self.length_mm) / 25.4)
            }
        }
    }
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

    /// Zooming changes the bar. It used to be built from the mesh's bounding box
    /// over the viewport width, so it was right for the first frame after a file
    /// opened and then described that framing for the rest of the session.
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
            bar.label(crate::app_settings::UnitDisplay::Millimeters),
            "20 mm"
        );
        assert_eq!(
            bar.label(crate::app_settings::UnitDisplay::Inches),
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
            bar.label(crate::app_settings::UnitDisplay::Millimeters),
            "1 mm"
        );
    }

    #[test]
    fn rounds_large_scenes_to_nice_lengths() {
        let bar = ScaleBar::for_mm_per_px(500.0 / 512.0).unwrap();

        assert_eq!(bar.length_mm, 100.0);
        assert!((bar.width_px - 102.4).abs() < 0.01);
        assert_eq!(
            bar.label(crate::app_settings::UnitDisplay::Millimeters),
            "100 mm"
        );
    }
}

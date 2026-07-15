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
    /// Pick a readable millimeter scale bar for a rendered view.
    #[must_use]
    pub fn for_viewport(scene_width_mm: f32, viewport_width_px: f32) -> Option<Self> {
        if !scene_width_mm.is_finite()
            || !viewport_width_px.is_finite()
            || scene_width_mm <= 0.0
            || viewport_width_px <= 0.0
        {
            return None;
        }

        let mm_per_px = scene_width_mm / viewport_width_px;
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

    /// Label text for the UI.
    #[must_use]
    pub fn label(self) -> String {
        format!("{:.0} mm", self.length_mm)
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
    use super::*;

    #[test]
    fn picks_readable_bar_for_typical_arch_width() {
        let bar = ScaleBar::for_viewport(80.0, 512.0).unwrap();

        assert_eq!(bar.length_mm, 20.0);
        assert!((bar.width_px - 128.0).abs() < 0.01);
        assert_eq!(bar.label(), "20 mm");
    }

    #[test]
    fn returns_none_for_invalid_dimensions() {
        assert!(ScaleBar::for_viewport(0.0, 512.0).is_none());
        assert!(ScaleBar::for_viewport(80.0, 0.0).is_none());
        assert!(ScaleBar::for_viewport(f32::NAN, 512.0).is_none());
    }

    #[test]
    fn keeps_small_scenes_in_millimeters() {
        let bar = ScaleBar::for_viewport(4.0, 512.0).unwrap();

        assert_eq!(bar.length_mm, 1.0);
        assert!((bar.width_px - 128.0).abs() < 0.01);
        assert_eq!(bar.label(), "1 mm");
    }

    #[test]
    fn rounds_large_scenes_to_nice_lengths() {
        let bar = ScaleBar::for_viewport(500.0, 512.0).unwrap();

        assert_eq!(bar.length_mm, 100.0);
        assert!((bar.width_px - 102.4).abs() < 0.01);
        assert_eq!(bar.label(), "100 mm");
    }
}

//! Shared UI tokens for OccluView chrome.

use eframe::egui;

/// Neutral accent for active state, selection, and links.
pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(54, 60, 68);
/// Primary body ink.
pub(crate) const TEXT: egui::Color32 = egui::Color32::from_rgb(26, 32, 44);
/// Secondary ink for labels and metadata.
pub(crate) const TEXT_WEAK: egui::Color32 = egui::Color32::from_rgb(90, 98, 110);
/// Muted ink with a 4.76:1 contrast ratio on white.
pub(crate) const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(100, 116, 139);

/// Errors and rejected results.
pub(crate) const DANGER: egui::Color32 = egui::Color32::from_rgb(198, 64, 48);
/// Semantic warning (unsaved markers, caution).
pub(crate) const WARNING: egui::Color32 = egui::Color32::from_rgb(181, 106, 0);

/// Corner radius for controls (buttons, chips, sliders, rows).
pub(crate) const RADIUS_CONTROL: f32 = 6.0;
/// Corner radius for floating panels and windows.
pub(crate) const RADIUS_PANEL: f32 = 10.0;

// The translucent tokens below carry a saturated color with a low alpha, which
// `from_rgba_premultiplied` (the only const constructor) cannot represent, so
// they are small runtime helpers built from straight (unmultiplied) alpha.

/// Frosted fill for floating viewport panels.
pub(crate) fn panel_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(248, 250, 252, 234)
}
/// Hairline border for floating viewport panels.
pub(crate) fn panel_stroke() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(26, 32, 44, 46)
}
/// Section divider inside panels and menus.
pub(crate) fn hairline() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(26, 32, 44, 30)
}
/// Row background under the pointer.
pub(crate) fn row_hover_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(54, 60, 68, 18)
}
/// Row background for the layer currently open in the mesh editor.
pub(crate) fn row_active_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(54, 60, 68, 34)
}

/// Shadow for floating viewport panels.
pub(crate) fn panel_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: egui::Color32::from_black_alpha(40),
    }
}

/// Shared frame for the floating overlays that sit over the 3D viewport.
pub(crate) fn overlay_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(panel_fill())
        .stroke(egui::Stroke::new(1.0_f32, panel_stroke()))
        .shadow(panel_shadow())
        .corner_radius(RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// Fixed toolbar height.
pub(crate) const MENUBAR_HEIGHT_PX: f32 = 34.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_frame_keeps_the_established_panel_geometry() {
        let frame = overlay_frame();

        assert_eq!(frame.corner_radius, egui::CornerRadius::same(10));
        assert_eq!(frame.inner_margin, egui::Margin::symmetric(10, 8));
        assert_eq!(frame.shadow.offset, [0, 4]);
        assert_eq!(frame.shadow.blur, 16);
        assert_eq!(frame.shadow.spread, 0);
        assert!((frame.stroke.width - 1.0).abs() < f32::EPSILON);
        assert_eq!(frame.stroke.color, panel_stroke());
    }
}

//! Shared UI tokens and drawn glyph primitives for the OccluView chrome.
//!
//! One quiet, dense, precise design language (exocad/CAD register): a single
//! accent, neutral ink, hairline separators, and small vector glyphs instead of
//! emoji. The menubar, the layers overlay, and the About dialog all pull their
//! colors and iconography from here so the app reads as one tool.

use eframe::egui;

/// Primary accent used for active state, selection, and links.
pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(66, 117, 204);
/// Primary body ink.
pub(crate) const TEXT: egui::Color32 = egui::Color32::from_rgb(26, 32, 44);
/// Secondary ink for labels and metadata.
pub(crate) const TEXT_WEAK: egui::Color32 = egui::Color32::from_rgb(90, 98, 110);
/// Muted ink for disabled/hidden affordances (e.g. a closed visibility eye).
pub(crate) const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(154, 161, 172);

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
    egui::Color32::from_rgba_unmultiplied(66, 117, 204, 22)
}
/// Row background for the layer currently open in the mesh editor.
pub(crate) fn row_active_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(66, 117, 204, 46)
}

/// Menubar height: compact, fixed, consistent with a professional tool.
pub(crate) const MENUBAR_HEIGHT_PX: f32 = 30.0;

/// Shared frame for the floating overlays that sit over the 3D viewport.
pub(crate) fn overlay_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(panel_fill())
        .stroke(egui::Stroke::new(1.0, panel_stroke()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
}

/// Number of samples per almond arc when drawing the visibility eye.
const EYE_ARC_SAMPLES: u16 = 12;

/// Draw a small, crisp visibility eye inside `rect`. Open when `visible`,
/// otherwise dimmed with a diagonal slash. Vector-drawn (no emoji, no font
/// glyph) so it stays sharp at the overlay's small control size.
pub(crate) fn paint_visibility_eye(painter: &egui::Painter, rect: egui::Rect, visible: bool) {
    let color = if visible { TEXT } else { TEXT_MUTED };
    let stroke = egui::Stroke::new(1.4, color);
    let center = rect.center();
    let half_w = rect.width() * 0.42;
    let half_h = rect.height() * 0.27;

    // Almond outline sampled as an upper and a lower arc that meet at the corners.
    let samples = f32::from(EYE_ARC_SAMPLES);
    let mut outline = Vec::with_capacity(usize::from(EYE_ARC_SAMPLES) * 2 + 2);
    for i in 0..=EYE_ARC_SAMPLES {
        let t = f32::from(i) / samples;
        let x = center.x - half_w + 2.0 * half_w * t;
        let lid = (std::f32::consts::PI * t).sin();
        outline.push(egui::pos2(x, center.y - half_h * lid));
    }
    for i in (0..=EYE_ARC_SAMPLES).rev() {
        let t = f32::from(i) / samples;
        let x = center.x - half_w + 2.0 * half_w * t;
        let lid = (std::f32::consts::PI * t).sin();
        outline.push(egui::pos2(x, center.y + half_h * lid));
    }
    painter.add(egui::Shape::closed_line(outline, stroke));

    if visible {
        painter.circle_filled(center, half_h * 0.72, color);
    } else {
        painter.line_segment(
            [
                egui::pos2(center.x - half_w, center.y + half_h),
                egui::pos2(center.x + half_w, center.y - half_h),
            ],
            egui::Stroke::new(1.4, color),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn theme_uses_vector_eye_not_emoji_glyph() {
        let source = include_str!("ui_theme.rs");
        assert!(
            source.contains("fn paint_visibility_eye("),
            "visibility toggle should be a drawn vector glyph"
        );
        assert!(
            !source.contains('\u{1f441}') && !source.contains('\u{1f440}'),
            "iconography must not use eye emoji"
        );
    }
}

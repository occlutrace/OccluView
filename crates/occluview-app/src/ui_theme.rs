//! Shared UI tokens for OccluView chrome.
//!
//! Tokens resolve against the active theme — one process-wide value the app
//! sets every frame from the loaded preference. Paint helpers read the token
//! functions instead of threading a theme parameter through every call.

use crate::app_settings::ThemePreference;
use eframe::egui;
use std::sync::atomic::{AtomicU8, Ordering};

const LIGHT: u8 = 0;
const DARK: u8 = 1;

/// The chrome theme the token helpers resolve against. The UI is single
/// threaded, so a process-wide atomic beats threading a parameter through
/// every paint call.
static ACTIVE_THEME: AtomicU8 = AtomicU8::new(LIGHT);

pub(crate) fn set_active(theme: ThemePreference) {
    ACTIVE_THEME.store(
        match theme {
            ThemePreference::Light => LIGHT,
            ThemePreference::Dark => DARK,
        },
        Ordering::Relaxed,
    );
}

fn dark() -> bool {
    ACTIVE_THEME.load(Ordering::Relaxed) == DARK
}

fn pick(light: egui::Color32, dark_value: egui::Color32) -> egui::Color32 {
    if dark() {
        dark_value
    } else {
        light
    }
}

/// Accent for active state, selection, and links: dark slate on light chrome,
/// pale slate on dark chrome (the light value would vanish against it).
pub(crate) fn accent() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(54, 60, 68),
        egui::Color32::from_rgb(150, 164, 182),
    )
}

/// Ink for text sitting on an `accent()` fill. The light accent is dark enough
/// for white text; the pale dark-theme accent needs near-black ink.
pub(crate) fn on_accent() -> egui::Color32 {
    pick(egui::Color32::WHITE, egui::Color32::from_rgb(20, 22, 26))
}

/// Whether the dark chrome theme is active, for the few callers that need a
/// theme branch instead of a token (marker fills painted over the render).
pub(crate) fn is_dark() -> bool {
    dark()
}

/// Primary body ink.
pub(crate) fn text() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(26, 32, 44),
        egui::Color32::from_rgb(226, 230, 236),
    )
}

/// Secondary ink for labels and metadata.
pub(crate) fn text_weak() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(90, 98, 110),
        egui::Color32::from_rgb(168, 176, 188),
    )
}

/// Muted ink with a 4.76:1 contrast ratio on white.
pub(crate) fn text_muted() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(100, 116, 139),
        egui::Color32::from_rgb(136, 146, 160),
    )
}

/// Errors and rejected results.
pub(crate) fn danger() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(198, 64, 48),
        egui::Color32::from_rgb(240, 112, 94),
    )
}

/// Semantic warning (unsaved markers, caution).
pub(crate) fn warning() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(181, 106, 0),
        egui::Color32::from_rgb(232, 178, 86),
    )
}

/// Solid toolbar band fill.
pub(crate) fn toolbar_fill() -> egui::Color32 {
    pick(
        egui::Color32::from_rgb(247, 248, 250),
        egui::Color32::from_rgb(26, 29, 34),
    )
}

/// Ink for the scale bar and axis-gizmo overlays that sit directly on the 3D
/// render (not on chrome panels). Chosen by the *viewport background* — an
/// independent setting — not by the chrome theme: a dark chrome theme over the
/// default gray render still needs dark ink.
pub(crate) fn viewport_ink(viewport_is_dark: bool) -> egui::Color32 {
    if viewport_is_dark {
        egui::Color32::from_rgb(236, 240, 246)
    } else {
        egui::Color32::from_rgb(15, 23, 42)
    }
}

/// Soft halo behind viewport ink so it reads over any geometry.
pub(crate) fn viewport_ink_halo(viewport_is_dark: bool) -> egui::Color32 {
    if viewport_is_dark {
        egui::Color32::from_rgba_unmultiplied(16, 19, 24, 200)
    } else {
        egui::Color32::from_rgba_unmultiplied(248, 250, 252, 190)
    }
}

/// Corner radius for controls (buttons, chips, sliders, rows).
pub(crate) const RADIUS_CONTROL: f32 = 6.0;
/// Corner radius for floating panels and windows.
pub(crate) const RADIUS_PANEL: f32 = 10.0;

// The translucent tokens below carry a saturated color with a low alpha, which
// `from_rgba_premultiplied` (the only const constructor) cannot represent, so
// they are small runtime helpers built from straight (unmultiplied) alpha.

/// Frosted fill for floating viewport panels.
pub(crate) fn panel_fill() -> egui::Color32 {
    pick(
        egui::Color32::from_rgba_unmultiplied(248, 250, 252, 234),
        egui::Color32::from_rgba_unmultiplied(30, 33, 39, 238),
    )
}
/// Hairline border for floating viewport panels.
pub(crate) fn panel_stroke() -> egui::Color32 {
    pick(
        egui::Color32::from_rgba_unmultiplied(26, 32, 44, 46),
        egui::Color32::from_rgba_unmultiplied(226, 232, 240, 42),
    )
}
/// Section divider inside panels and menus.
pub(crate) fn hairline() -> egui::Color32 {
    pick(
        egui::Color32::from_rgba_unmultiplied(26, 32, 44, 30),
        egui::Color32::from_rgba_unmultiplied(226, 232, 240, 28),
    )
}
/// Row background under the pointer.
pub(crate) fn row_hover_fill() -> egui::Color32 {
    pick(
        egui::Color32::from_rgba_unmultiplied(54, 60, 68, 18),
        egui::Color32::from_rgba_unmultiplied(226, 232, 240, 22),
    )
}
/// Row background for the layer currently open in the mesh editor.
pub(crate) fn row_active_fill() -> egui::Color32 {
    pick(
        egui::Color32::from_rgba_unmultiplied(54, 60, 68, 34),
        egui::Color32::from_rgba_unmultiplied(226, 232, 240, 44),
    )
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
    use crate::app_settings::ThemePreference;

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

    #[test]
    fn tokens_flip_between_themes() {
        set_active(ThemePreference::Light);
        let light_text = text();
        let light_fill = panel_fill();

        set_active(ThemePreference::Dark);
        assert_ne!(text(), light_text);
        assert_ne!(panel_fill(), light_fill);

        set_active(ThemePreference::Light);
        assert_eq!(text(), light_text);
    }
}

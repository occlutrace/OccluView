use crate::app_settings::ThemePreference;
use eframe::egui;

pub(crate) fn load_app_logo_color_image() -> Option<egui::ColorImage> {
    let image = image::load_from_memory(include_bytes!("../assets/windows/occluview.png"))
        .ok()?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &rgba,
    ))
}

/// Height of the unobtrusive bottom-left status row.
const STATUS_HEIGHT_PX: f32 = 22.0;
/// Keep transient status text above the scale-bar label without drawing a
/// second surface over the render.
const STATUS_BOTTOM_OFFSET_PX: f32 = 44.0;
const STATUS_MAX_WIDTH_PX: f32 = 360.0;

/// Bottom-left status row. It is deliberately a transparent, compact text
/// target rather than a framed pill: the scale bar keeps the bottom edge and
/// the render remains visible behind transient messages.
pub(crate) fn status_overlay_rect(viewport_rect: egui::Rect) -> egui::Rect {
    let width = (viewport_rect.width() - 28.0).clamp(0.0, STATUS_MAX_WIDTH_PX);
    let row_bottom = viewport_rect.bottom() - STATUS_BOTTOM_OFFSET_PX;
    egui::Rect::from_min_size(
        egui::pos2(viewport_rect.left() + 14.0, row_bottom - STATUS_HEIGHT_PX),
        egui::vec2(width, STATUS_HEIGHT_PX),
    )
}

/// Quiet, precise theme for the whole app: neutral surfaces, hairline borders,
/// a single neutral accent for hover/active/selection, and softly rounded
/// controls. Tuned to read as a professional CAD viewer rather than a demo.
pub(crate) fn viewer_visuals(theme: ThemePreference) -> egui::Visuals {
    use crate::ui_theme::{accent, hairline, text};

    let dark = theme == ThemePreference::Dark;
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let accent = accent();
    if dark {
        visuals.window_fill = egui::Color32::from_rgb(24, 27, 32);
        visuals.panel_fill = egui::Color32::from_rgb(28, 31, 37);
        visuals.faint_bg_color = egui::Color32::from_rgb(36, 40, 47);
        visuals.extreme_bg_color = egui::Color32::from_rgb(20, 22, 26);
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(36, 40, 47);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(36, 40, 47);
    } else {
        visuals.window_fill = egui::Color32::from_rgb(250, 251, 252);
        visuals.panel_fill = egui::Color32::from_rgb(243, 245, 248);
        visuals.faint_bg_color = egui::Color32::from_rgb(236, 239, 243);
        visuals.extreme_bg_color = egui::Color32::from_rgb(255, 255, 255);
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(236, 239, 243);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(236, 239, 243);
    }
    visuals.window_corner_radius = egui::CornerRadius::same(7);
    visuals.menu_corner_radius = egui::CornerRadius::same(7);
    visuals.window_stroke = egui::Stroke::new(1.0_f32, hairline());
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: egui::Color32::from_black_alpha(38),
    };
    visuals.selection.bg_fill = accent.gamma_multiply(0.28);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, accent);
    visuals.hyperlink_color = accent;

    // Static text and disabled chrome.
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text());
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, hairline());

    // Resting interactive controls: flat, hairline outline, rounded.
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, hairline());
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, text());
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);

    // Hover: a light accent wash, accent hairline.
    visuals.widgets.hovered.weak_bg_fill = accent.gamma_multiply(0.12);
    visuals.widgets.hovered.bg_fill = accent.gamma_multiply(0.12);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.55));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, text());
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);

    // Active / pressed: solid accent.
    visuals.widgets.active.weak_bg_fill = accent;
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, accent);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(
        1.0_f32,
        if dark {
            egui::Color32::from_rgb(20, 22, 26)
        } else {
            egui::Color32::WHITE
        },
    );
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);

    // Open menus track the resting palette so dropdowns stay quiet.
    visuals.widgets.open.weak_bg_fill = accent.gamma_multiply(0.12);
    visuals.widgets.open.bg_fill = accent.gamma_multiply(0.12);
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, hairline());
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, text());

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_near(left: f32, right: f32) {
        assert!(
            (left - right).abs() < f32::EPSILON,
            "left={left}, right={right}"
        );
    }

    #[test]
    fn viewer_visuals_keep_the_established_corner_and_shadow_geometry() {
        let visuals = viewer_visuals(ThemePreference::Light);

        assert_eq!(visuals.window_corner_radius, egui::CornerRadius::same(7));
        assert_eq!(visuals.menu_corner_radius, egui::CornerRadius::same(7));
        assert_eq!(visuals.popup_shadow.offset, [0, 3]);
        assert_eq!(visuals.popup_shadow.blur, 12);
        assert_eq!(visuals.popup_shadow.spread, 0);
        for widget in [
            visuals.widgets.inactive,
            visuals.widgets.hovered,
            visuals.widgets.active,
        ] {
            assert_eq!(widget.corner_radius, egui::CornerRadius::same(4));
        }
    }

    #[test]
    fn dark_theme_keeps_the_same_geometry_with_dark_surfaces() {
        let visuals = viewer_visuals(ThemePreference::Dark);

        assert_eq!(visuals.window_corner_radius, egui::CornerRadius::same(7));
        assert_eq!(visuals.popup_shadow.blur, 12);
        assert_ne!(
            visuals.widgets.inactive.weak_bg_fill,
            egui::Color32::from_rgb(236, 239, 243),
            "dark theme must not keep the light resting fill"
        );
    }

    #[test]
    fn status_overlay_is_compact_and_clear_of_the_scale_bar() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));

        let rect = status_overlay_rect(viewport);

        assert_near(rect.left(), 14.0);
        // The transient status is a quiet text row, not a 34 px panel.
        assert_near(rect.bottom(), 756.0);
        assert!(rect.height() <= 22.0);
        assert!(rect.width() <= 360.0);
        assert!(viewport.contains_rect(rect));
    }

    #[test]
    fn status_overlay_clears_the_scale_bar_band() {
        // The status pill and the scale bar are both bottom-left; the pill must
        // sit strictly above the scale bar so it no longer covers the ruler.
        // The highest scale-bar pixel is its mm label top: bar line (bottom - 16)
        // minus the 22 px label offset baked into `app_scale_bar::paint_scale_bar`.
        for size in [
            egui::vec2(1200.0, 800.0),
            egui::vec2(640.0, 480.0),
            egui::vec2(2000.0, 1100.0),
        ] {
            let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size);
            let rect = status_overlay_rect(viewport);
            let scale_bar_label_top = viewport.bottom() - (16.0 + 22.0);
            assert!(
                rect.bottom() <= scale_bar_label_top,
                "status pill (bottom={}) must clear the scale bar label top ({})",
                rect.bottom(),
                scale_bar_label_top
            );
            assert_near(rect.left(), 14.0);
            assert!(viewport.contains_rect(rect));
        }
    }
}

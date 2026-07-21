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

/// Vertical band (px, measured up from the viewport's bottom edge) reserved for
/// the bottom-left scale bar and its mm label. The scale bar line sits 16 px up
/// and its label another 22 px above that (`app_scale_bar::paint_scale_bar`), so
/// the label top — the highest scale-bar pixel — is ~38 px up; 40 px clears it.
const SCALE_BAR_RESERVE_PX: f32 = 40.0;
/// Breathing gap between the status pill and the scale bar band above it.
const STATUS_SCALE_BAR_GAP_PX: f32 = 8.0;
/// Height of the bottom-left status pill.
const STATUS_HEIGHT_PX: f32 = 34.0;

/// Bottom-left status pill rectangle, lifted to sit *above* the scale bar band
/// so the two no longer overlap. The scale bar keeps the very bottom-left corner
/// (line + ticks + mm label); the transient status message stacks just above it.
/// The axis gizmo and (moved) Section panel own the bottom-right corner, so this
/// column is the status pill's alone.
pub(crate) fn status_overlay_rect(viewport_rect: egui::Rect) -> egui::Rect {
    let width = viewport_rect.width().clamp(240.0, 430.0);
    let pill_bottom = viewport_rect.bottom() - SCALE_BAR_RESERVE_PX - STATUS_SCALE_BAR_GAP_PX;
    egui::Rect::from_min_size(
        egui::pos2(viewport_rect.left() + 14.0, pill_bottom - STATUS_HEIGHT_PX),
        egui::vec2(width, STATUS_HEIGHT_PX),
    )
}

/// Quiet, precise light theme for the whole app: neutral surfaces, hairline
/// borders, a single neutral accent for hover/active/selection, and softly rounded
/// controls. Tuned to read as a professional CAD viewer rather than a demo.
pub(crate) fn viewer_visuals() -> egui::Visuals {
    use crate::ui_theme::{hairline, ACCENT, TEXT};

    let mut visuals = egui::Visuals::light();
    visuals.window_fill = egui::Color32::from_rgb(250, 251, 252);
    visuals.panel_fill = egui::Color32::from_rgb(243, 245, 248);
    visuals.faint_bg_color = egui::Color32::from_rgb(236, 239, 243);
    visuals.extreme_bg_color = egui::Color32::from_rgb(255, 255, 255);
    visuals.window_rounding = egui::Rounding::same(7.0);
    visuals.menu_rounding = egui::Rounding::same(7.0);
    visuals.window_stroke = egui::Stroke::new(1.0, hairline());
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 3.0),
        blur: 12.0,
        spread: 0.0,
        color: egui::Color32::from_black_alpha(38),
    };
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.28);
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    // Static text and disabled chrome.
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, hairline());

    // Resting interactive controls: flat, hairline outline, rounded.
    visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(236, 239, 243);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(236, 239, 243);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, hairline());
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.rounding = egui::Rounding::same(4.0);

    // Hover: a light accent wash, accent hairline.
    visuals.widgets.hovered.weak_bg_fill = ACCENT.gamma_multiply(0.12);
    visuals.widgets.hovered.bg_fill = ACCENT.gamma_multiply(0.12);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.55));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.rounding = egui::Rounding::same(4.0);

    // Active / pressed: solid accent.
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.active.rounding = egui::Rounding::same(4.0);

    // Open menus track the resting palette so dropdowns stay quiet.
    visuals.widgets.open.weak_bg_fill = ACCENT.gamma_multiply(0.12);
    visuals.widgets.open.bg_fill = ACCENT.gamma_multiply(0.12);
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, hairline());
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, TEXT);

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
    fn status_overlay_uses_bottom_left_without_resizing_viewport() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));

        let rect = status_overlay_rect(viewport);

        assert_near(rect.left(), 14.0);
        // Lifted above the scale bar band: bottom - 40 (reserve) - 8 (gap) = 752.
        assert_near(rect.bottom(), 752.0);
        assert_near(rect.height(), STATUS_HEIGHT_PX);
        assert!(rect.width() <= 620.0);
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

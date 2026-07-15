use eframe::egui;

pub(super) const LAYER_ROW_HEIGHT_PX: f32 = 28.0;
// Title line + hairline separator + breathing room above the first row.
pub(super) const LAYER_OVERLAY_HEADER_HEIGHT_PX: f32 = 32.0;
pub(super) const LAYER_ROW_GAP_PX: f32 = 8.0;
pub(super) const LAYER_ROW_CONTROL_HEIGHT_PX: f32 = 18.0;
pub(super) const LAYER_ROW_EYE_WIDTH_PX: f32 = 18.0;
pub(super) const LAYER_ROW_SLIDER_WIDTH_PX: f32 = 54.0;
pub(super) const LAYER_ROW_TINT_WIDTH_PX: f32 = 18.0;
pub(super) const LAYER_ROW_REMOVE_WIDTH_PX: f32 = 18.0;
// The destructive remove control gets a wider gap than the ordinary columns.
pub(super) const LAYER_ROW_ACTION_GAP_PX: f32 = 6.0;
pub(super) const LAYER_ROW_CONTROL_WIDTH_PX: f32 = LAYER_ROW_EYE_WIDTH_PX
    + LAYER_ROW_SLIDER_WIDTH_PX
    + LAYER_ROW_TINT_WIDTH_PX
    + LAYER_ROW_REMOVE_WIDTH_PX
    + LAYER_ROW_GAP_PX * 3.0
    + LAYER_ROW_ACTION_GAP_PX;

/// The panel sizes itself to its content: it grows one row per layer until it
/// would cover this fraction of the viewport height, and only then scrolls.
/// No absolute pixel cap — a taller window shows more layers, dynamically.
const LAYER_OVERLAY_MAX_VIEWPORT_FRACTION: f32 = 0.72;

pub(crate) fn layer_overlay_rect(viewport_rect: egui::Rect, layer_count: usize) -> egui::Rect {
    let max_width = (viewport_rect.width() - 28.0).max(180.0);
    let width = (viewport_rect.width() * 0.22)
        .clamp(236.0, 320.0)
        .min(max_width);
    let max_height = (viewport_rect.height() * LAYER_OVERLAY_MAX_VIEWPORT_FRACTION).max(86.0);
    let rows_that_fit = ((max_height - LAYER_OVERLAY_HEADER_HEIGHT_PX) / LAYER_ROW_HEIGHT_PX)
        .floor()
        .max(1.0);
    #[allow(clippy::cast_precision_loss)]
    let requested_rows = layer_count.clamp(1, 4096) as f32;
    let visible_rows = requested_rows.min(rows_that_fit);
    let height = (LAYER_OVERLAY_HEADER_HEIGHT_PX + LAYER_ROW_HEIGHT_PX * visible_rows)
        .clamp(86.0, max_height);
    egui::Rect::from_min_size(
        viewport_rect.min + egui::vec2(14.0, 14.0),
        egui::vec2(width, height),
    )
}

/// Width the layer name label may occupy: everything the fixed control columns
/// leave behind, so the name fills the row and truncates instead of pushing the
/// controls off the edge.
pub(super) fn layer_name_width(row_width: f32) -> f32 {
    (row_width - LAYER_ROW_CONTROL_WIDTH_PX).max(0.0)
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
    fn layer_overlay_stays_inside_viewport_corner() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));

        let rect = layer_overlay_rect(viewport, 4);

        assert_near(rect.left(), 14.0);
        assert_near(rect.top(), 14.0);
        assert!(rect.width() <= 300.0);
        assert!(rect.height() <= 420.0);
        assert!(viewport.contains_rect(rect));
    }

    #[test]
    fn layer_name_width_fills_the_space_left_by_fixed_controls() {
        assert_near(layer_name_width(280.0), 280.0 - LAYER_ROW_CONTROL_WIDTH_PX);
        assert_near(layer_name_width(216.0), 216.0 - LAYER_ROW_CONTROL_WIDTH_PX);
        // A row narrower than the control stack leaves no room for the name
        // rather than overflowing.
        assert_near(layer_name_width(LAYER_ROW_CONTROL_WIDTH_PX - 20.0), 0.0);
    }

    #[test]
    fn layer_row_columns_do_not_overflow_available_width() {
        for row_width in [120.0, 216.0, 260.0, 320.0] {
            let used = LAYER_ROW_CONTROL_WIDTH_PX + layer_name_width(row_width);
            assert!(
                used <= row_width.max(LAYER_ROW_CONTROL_WIDTH_PX) + f32::EPSILON,
                "row columns should stay bounded: row_width={row_width}, used={used}"
            );
        }
    }

    #[test]
    fn layer_row_action_controls_are_symmetric_and_have_breathing_room() {
        let eye_width = std::hint::black_box(LAYER_ROW_EYE_WIDTH_PX);
        let tint_width = std::hint::black_box(LAYER_ROW_TINT_WIDTH_PX);
        let remove_width = std::hint::black_box(LAYER_ROW_REMOVE_WIDTH_PX);
        assert!(
            (tint_width - remove_width).abs() <= f32::EPSILON
                && (tint_width - eye_width).abs() <= f32::EPSILON,
            "eye, tint swatch, and remove action should sit in symmetric fixed columns"
        );
        let row_gap = std::hint::black_box(LAYER_ROW_GAP_PX);
        assert!(
            row_gap >= 6.0,
            "compact rows still need enough horizontal air between controls"
        );
        let action_gap = std::hint::black_box(LAYER_ROW_ACTION_GAP_PX);
        assert!(
            action_gap >= 4.0,
            "remove action needs a distinct gap from the tint swatch"
        );
        assert_near(
            LAYER_ROW_CONTROL_WIDTH_PX,
            LAYER_ROW_EYE_WIDTH_PX
                + LAYER_ROW_SLIDER_WIDTH_PX
                + LAYER_ROW_TINT_WIDTH_PX
                + LAYER_ROW_REMOVE_WIDTH_PX
                + LAYER_ROW_GAP_PX * 3.0
                + LAYER_ROW_ACTION_GAP_PX,
        );
    }

    #[test]
    fn layer_row_controls_use_one_height_contract() {
        let source = include_str!("layout.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\n#[cfg(test)]")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("LAYER_ROW_CONTROL_HEIGHT_PX"),
            "row controls should share one height contract instead of repeating literals"
        );
    }
}

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

/// The overlay's own chrome stacked on top of its rows: the frame's vertical
/// inner margins (2 × 8 px) plus the header block (`LAYER_OVERLAY_HEADER_
/// HEIGHT_PX`) the row list lives under. Both the wanted height and the
/// scroll-area budget derive from it, so the rows a panel was sized for never
/// overflow into a scrollbar.
pub(crate) const LAYER_OVERLAY_CHROME_HEIGHT_PX: f32 = 16.0 + LAYER_OVERLAY_HEADER_HEIGHT_PX;
/// Top offset of the panel inside the viewport (see `layer_overlay_rect`).
pub(crate) const LAYER_OVERLAY_TOP_OFFSET_PX: f32 = 14.0;
/// Height kept clear at the bottom of the viewport when the panel stretches
/// to its maximum: the scale-bar band (40 px), the gap above it, and the
/// status pill — all bottom-left, under the panel's own corner.
pub(crate) const LAYER_OVERLAY_BOTTOM_RESERVE_PX: f32 = 82.0;

/// Full height the panel wants to show `layer_count` rows without scrolling.
/// Derived from the layer count alone — never from the current viewport — so
/// the window-growth hint in the app layer can be computed from it directly.
pub(crate) fn layer_overlay_desired_height(layer_count: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let rows = layer_count.max(1) as f32;
    LAYER_OVERLAY_CHROME_HEIGHT_PX + LAYER_ROW_HEIGHT_PX * rows
}

/// The panel sizes itself to its content: it grows one row per layer until it
/// reaches the space between the viewport's top edge and the bottom overlay
/// band, and only then scrolls. The app grows the OS window past that point.
pub(crate) fn layer_overlay_rect(viewport_rect: egui::Rect, layer_count: usize) -> egui::Rect {
    let max_width = (viewport_rect.width() - 28.0).max(180.0);
    let width = (viewport_rect.width() * 0.22)
        .clamp(236.0, 320.0)
        .min(max_width)
        // The floors above are aspirations; on a tiny window the viewport wins
        // so the panel never pokes past the window edge.
        .min((viewport_rect.width() - LAYER_OVERLAY_TOP_OFFSET_PX).max(0.0));
    let max_height =
        (viewport_rect.height() - LAYER_OVERLAY_TOP_OFFSET_PX - LAYER_OVERLAY_BOTTOM_RESERVE_PX)
            .max(86.0);
    let height = layer_overlay_desired_height(layer_count)
        .clamp(86.0, max_height)
        .min((viewport_rect.height() - LAYER_OVERLAY_TOP_OFFSET_PX).max(0.0));
    egui::Rect::from_min_size(
        viewport_rect.min + egui::vec2(LAYER_OVERLAY_TOP_OFFSET_PX, LAYER_OVERLAY_TOP_OFFSET_PX),
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
    fn the_panel_wants_one_row_per_layer_without_scrolling() {
        assert_near(
            layer_overlay_desired_height(1),
            LAYER_OVERLAY_CHROME_HEIGHT_PX + LAYER_ROW_HEIGHT_PX,
        );
        assert_near(
            layer_overlay_desired_height(0),
            layer_overlay_desired_height(1),
        );
        assert_near(
            layer_overlay_desired_height(30),
            LAYER_OVERLAY_CHROME_HEIGHT_PX + LAYER_ROW_HEIGHT_PX * 30.0,
        );
    }

    #[test]
    fn a_tall_viewport_shows_every_layer_without_a_cap() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 1100.0));

        let rect = layer_overlay_rect(viewport, 30);

        assert_near(rect.height(), layer_overlay_desired_height(30));
        assert!(viewport.contains_rect(rect));
    }

    #[test]
    fn a_short_viewport_caps_the_panel_above_the_bottom_band() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 500.0));

        let rect = layer_overlay_rect(viewport, 30);

        assert_near(
            rect.height(),
            500.0 - LAYER_OVERLAY_TOP_OFFSET_PX - LAYER_OVERLAY_BOTTOM_RESERVE_PX,
        );
        assert!(viewport.contains_rect(rect));
    }

    #[test]
    fn a_tiny_viewport_keeps_the_panel_inside_instead_of_honoring_the_floors() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(120.0, 60.0));

        let rect = layer_overlay_rect(viewport, 30);

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
    fn the_name_column_takes_what_the_controls_leave() {
        // The old form asserted `C + max(w - C, 0) <= max(w, C)`, which is the
        // same expression on both sides and true for any constants at all --
        // the control stack could have been five thousand pixels wide.
        for row_width in [120.0, 216.0, 260.0, 320.0] {
            let name = layer_name_width(row_width);
            let expected = (row_width - LAYER_ROW_CONTROL_WIDTH_PX).max(0.0);
            assert_near(name, expected);
        }
        // The panel is never narrower than the 236 px floor in
        // `layer_overlay_rect`, less its 20 px of frame. The controls have to
        // leave a readable name column at that width.
        let narrowest_row = 236.0 - 20.0;
        assert!(
            layer_name_width(narrowest_row) >= 60.0,
            "the controls take {LAYER_ROW_CONTROL_WIDTH_PX} px and leave \
             {} px for the layer name in the narrowest panel",
            layer_name_width(narrowest_row)
        );
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
    }
}

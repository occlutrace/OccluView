#![allow(clippy::expect_used)]

/// The window has to be draggable like the mesh editor: a panel pinned to a
/// corner covers the very geometry the operator is clicking on.
#[test]
fn align_window_opens_clear_of_layers_at_normal_and_narrow_widths() {
    for width in [600.0, 1024.0, 1600.0] {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 768.0));
        let layers = crate::layers_overlay::layer_overlay_rect(viewport, 2);
        let top_left = super::panel_default_pos(viewport, 2);
        let panel = egui::Rect::from_min_size(top_left, egui::vec2(super::WINDOW_WIDTH, 400.0));
        assert!(
            viewport.contains_rect(panel),
            "{width}: panel leaves viewport"
        );
        assert!(!panel.intersects(layers), "{width}: panel covers Layers");
    }
}

#[test]
fn previously_opened_align_window_reanchors_after_narrowing() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 720.0));
    let old_rect = egui::Rect::from_min_size(egui::pos2(425.0, 50.0), egui::vec2(308.0, 468.0));
    assert!(super::panel_needs_reanchor(Some(old_rect), viewport, 2));

    let new_rect = egui::Rect::from_min_size(
        super::panel_default_pos(viewport, 2),
        egui::vec2(super::WINDOW_WIDTH, 468.0),
    );
    assert!(viewport.contains_rect(new_rect));
    assert!(!new_rect.intersects(crate::layers_overlay::layer_overlay_rect(viewport, 2)));
    assert!(!super::panel_needs_reanchor(Some(new_rect), viewport, 2));
}

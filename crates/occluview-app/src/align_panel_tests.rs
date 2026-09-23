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

/// Every label AccessKit was handed for one render of the window.
///
/// The window is checked through the widgets it actually produced, not through
/// its source: AccessKit is what a screen reader sees, so a control that is
/// drawn but never registered is missing for exactly the operator who cannot
/// see it either.
fn panel_control_labels(
    ctx: &egui::Context,
    tab: super::AlignTab,
    locale: &crate::i18n::LocaleManager,
) -> Vec<String> {
    use crate::align_drag::DragConstraint;
    use crate::align_tool::AlignTool;
    use crate::align_worker::AlignSettings;

    let tool = AlignTool::default();
    let mut settings = AlignSettings::default();
    let mut constraint = DragConstraint::default();
    let mut excluding = false;
    let mut drop_pending = false;
    let mut open_tab = tab;
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
    let raw = egui::RawInput {
        screen_rect: Some(viewport),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        let _ = super::show(
            &ctx,
            viewport,
            super::AlignPanelView {
                tool: &tool,
                layer_count: 2,
                settings: &mut settings,
                constraint: &mut constraint,
                excluding: &mut excluding,
                drop_pending: &mut drop_pending,
                status: None,
                refined_match_ready: false,
                roles: None,
                busy: false,
                worker_failed: false,
                moved: false,
                can_undo: false,
                can_redo: false,
                tab: &mut open_tab,
            },
            locale,
        );
    });
    // The frame is inspected, not painted: its texture deltas are dropped the
    // way the other egui-driven tests here do.
    output.textures_delta.clear();
    output
        .platform_output
        .accesskit_update
        .map(|update| {
            update
                .nodes
                .iter()
                .filter_map(|(_, node)| node.label().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The exclusion brush belongs to the automatic tab and to no other.
///
/// Marking surface out of the match is a question only the tab that runs
/// best-fit matching asks. A brush control offered on the manual pose tab says
/// it applies to a fit that tab cannot run, and the operator who ticks it there
/// gets markings with nothing behind them.
#[test]
fn the_exclusion_brush_is_offered_on_the_automatic_tab_only() {
    let locale = crate::i18n::LocaleManager::for_tests();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let brush = locale.tr("align-exclude");

    for (tab, expected) in [
        (super::AlignTab::Automatically, true),
        (super::AlignTab::Manually, false),
    ] {
        let labels = panel_control_labels(&ctx, tab, &locale);
        assert!(
            !labels.is_empty(),
            "{tab:?}: the window has to render controls before this checks anything"
        );
        assert_eq!(
            labels.contains(&brush),
            expected,
            "{tab:?}: exclusion brush present={expected}, controls={labels:?}"
        );
    }
}

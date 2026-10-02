#![allow(clippy::expect_used, clippy::panic)]

#[test]
fn unavailable_alignment_controls_expose_disabled_responses_for_tooltips() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        for enabled in [false, true, false] {
            let chip = super::chip(ui, 100.0, None, "Back", enabled, false);
            let fit = super::fit_button(
                ui,
                200.0,
                crate::ui::icons::AppIcon::AlignFit,
                "Perform alignment",
                enabled,
                true,
            );
            assert_eq!(chip.enabled(), enabled, "history and brush controls");
            assert_eq!(fit.enabled(), enabled, "fit refusal tooltips");
            if !enabled {
                assert!(!chip.clicked() && !fit.clicked());
            }
        }
    });
    output.textures_delta.clear();
}

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
/// drawn but never registered is missing for a screen-reader user.
fn panel_control_labels(
    ctx: &egui::Context,
    tab: super::AlignTab,
    locale: &crate::i18n::LocaleManager,
) -> Vec<String> {
    panel_controls(ctx, tab, false, locale)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// Every label AccessKit was handed, with whether the control was disabled.
///
/// `busy` is the input that matters for the orientation rule: the job holds the
/// settings snapshot it was submitted with, so the rule has to be disabled
/// while a fit runs rather than let an edit describe a different match.
fn panel_controls(
    ctx: &egui::Context,
    tab: super::AlignTab,
    busy: bool,
    locale: &crate::i18n::LocaleManager,
) -> Vec<(String, bool)> {
    use crate::align::align_drag::DragConstraint;
    use crate::align::align_tool::AlignTool;
    use crate::align::align_worker::AlignSettings;

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
            egui::Id::new("align-panel-test-scene"),
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
                busy,
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
                .filter_map(|(_, node)| {
                    node.label()
                        .map(|label| (label.to_string(), node.is_disabled()))
                })
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
    let brush = locale.tr(crate::i18n::message_id!("align-exclude"));

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

/// The surface-orientation rule is disabled while a fit is running: the job
/// holds the settings snapshot it was submitted with, so an edit made
/// mid-flight would describe a different match than the one that lands.
///
/// Drives the real `facing` control through egui and reads the disabled state
/// back from the produced widget tree. The panel-level AccessKit node for a
/// window does not surface per-control disabled state on this egui version, so
/// the check runs at the control the panel delegates to.
#[test]
fn the_orientation_rule_is_disabled_while_a_fit_runs() {
    use occluview_align::Orientation;

    let locale = crate::i18n::LocaleManager::for_tests();
    let target = locale.tr(crate::i18n::message_id!("align-orientation-match"));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let disabled_for = |enabled: bool| -> bool {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 400.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let mut orientation = Orientation::Match;
            super::super::align_panel_settings::facing(ui, &mut orientation, enabled, &locale);
        });
        out.textures_delta.clear();
        out.platform_output
            .accesskit_update
            .and_then(|update| {
                update.nodes.iter().find_map(|(_, node)| {
                    (node.label() == Some(target.as_str())).then(|| node.is_disabled())
                })
            })
            .unwrap_or_else(|| panic!("the orientation radio {target:?} must be rendered"))
    };

    assert!(
        !disabled_for(true),
        "with no fit running the orientation rule is editable"
    );
    assert!(
        disabled_for(false),
        "a running fit must disable the orientation rule"
    );
}

/// Read the actual coarse-fit widget after its inputs change, including the
/// busy/failure states which used to leave a valid pair looking unavailable.
#[test]
fn the_coarse_fit_button_recovers_after_busy_and_failed_frames() {
    use crate::align::align_tool::{AlignPoint, AlignTool};
    use glam::Vec3;
    use occluview_core::{Mesh, Scene, SceneMesh, Vertex};

    let mut scene = Scene::new();
    for _ in 0..2 {
        let mesh = Mesh::new(
            None,
            vec![
                Vertex::at(Vec3::ZERO),
                Vertex::at(Vec3::X),
                Vertex::at(Vec3::Y),
            ],
            vec![0, 1, 2],
        )
        .expect("triangle mesh");
        scene.add(SceneMesh::new(mesh));
    }
    let moving = scene.meshes()[0].id();
    let fixed = scene.meshes()[1].id();
    let locale = crate::i18n::LocaleManager::for_tests();
    let target = format!(
        "1. {}",
        locale.tr(crate::i18n::message_id!("align-fit-perform"))
    );
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut observed_disabled = None;
    let mut rendered_disabled = |tool: &AlignTool, busy: bool, failed: bool| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 400.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = super::fits(ui, tool, !busy && !failed, busy, &locale);
        });
        output.textures_delta.clear();
        if let Some(update) = output.platform_output.accesskit_update {
            if let Some((_, node)) = update
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(target.as_str()))
            {
                observed_disabled = Some(node.is_disabled());
            }
        }
        observed_disabled.expect("the coarse-fit button must register with AccessKit")
    };

    let mut tool = AlignTool::default();
    tool.arm();
    assert!(rendered_disabled(&tool, false, false));
    for local in [Vec3::ZERO, Vec3::X] {
        tool.click(AlignPoint {
            layer: moving,
            local,
            normal: Vec3::Z,
        });
        tool.click(AlignPoint {
            layer: fixed,
            local,
            normal: Vec3::Z,
        });
        if tool.pairs().len() == 1 {
            assert!(rendered_disabled(&tool, false, false));
        }
    }
    for _ in 0..3 {
        assert!(
            !rendered_disabled(&tool, false, false),
            "two complete pairs enable coarse fit"
        );
        assert!(
            rendered_disabled(&tool, true, false),
            "a running job owns the pair"
        );
        assert!(
            !rendered_disabled(&tool, false, false),
            "completion or cancellation releases the button"
        );
        assert!(
            rendered_disabled(&tool, false, true),
            "a dead worker cannot accept jobs"
        );
        assert!(
            !rendered_disabled(&tool, false, false),
            "worker replacement restores the button"
        );
    }
}

fn fit_frame(
    ctx: &egui::Context,
    tool: &crate::align::align_tool::AlignTool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let _ = super::fits(
                ui,
                tool,
                true,
                false,
                &crate::i18n::LocaleManager::for_tests(),
            );
        },
    );
    output.textures_delta.clear();
    output
}

#[test]
fn a_single_complete_pair_explains_that_two_pairs_are_required() {
    use crate::align::align_tool::{AlignPoint, AlignTool};
    use glam::Vec3;
    use occluview_core::{test_support::simple_triangle_mesh, Scene, SceneMesh};

    let mut scene = Scene::new();
    for _ in 0..2 {
        scene.add(SceneMesh::new(
            simple_triangle_mesh(None).expect("triangle mesh"),
        ));
    }
    let mut tool = AlignTool::default();
    tool.arm();
    for entry in scene.meshes() {
        tool.click(AlignPoint {
            layer: entry.id(),
            local: Vec3::ZERO,
            normal: Vec3::Z,
        });
    }
    assert_eq!(tool.pairs().len(), 1);
    assert!(tool.pending().is_none());
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let output = fit_frame(&ctx, &tool, Vec::new());
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "1. Perform alignment" => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .expect("the coarse-fit button must be rendered");
    let _ = fit_frame(&ctx, &tool, vec![egui::Event::PointerMoved(pos)]);
    let output = fit_frame(&ctx, &tool, Vec::new());
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("needs at least two arrows")
        )),
        "a complete first pair must be told why it cannot fit yet"
    );
}

#![allow(clippy::expect_used, clippy::panic)]

use super::workspace::commands::SplitSide;
use super::workspace::layout::WorkspaceLayout;
use super::{egui, OccluViewApp};
use std::path::PathBuf;

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

// AccessKit reports f64 bounds for a UI that egui positions in f32 coordinates.
#[allow(clippy::cast_possible_truncation)]
fn accessible_center(bounds: egui::accesskit::Rect) -> egui::Pos2 {
    egui::pos2(
        bounds.x0.midpoint(bounds.x1) as f32,
        bounds.y0.midpoint(bounds.y1) as f32,
    )
}

fn frame(
    app: &mut OccluViewApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            app.show_workspace(ui);
            app.apply_workspace_commands(ctx);
        },
    );
    output.textures_delta.clear();
    output
}

fn start_rename(app: &mut OccluViewApp, ctx: &egui::Context) {
    app.workspace.rename = Some((app.workspace.scenes[0].key, String::new()));
    app.workspace.rename_focus_pending = true;
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
}

#[test]
fn rename_enter_commits_after_text_field_surrenders_focus() {
    let ctx = egui::Context::default();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    start_rename(&mut app, &ctx);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("  Upper arch  ".to_owned())],
    );
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.workspace.scenes[0].name, "Upper arch");
    assert!(app.workspace.rename.is_none());
    assert!(!app.ui.workspace_modal_open);
}

#[test]
fn rename_keeps_empty_input_focused_and_escape_wins_over_enter() {
    let ctx = egui::Context::default();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    let original = app.workspace.scenes[0].name.clone();
    start_rename(&mut app, &ctx);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert!(app.workspace.rename.is_some());
    assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("workspace-rename-value"))));
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("Discard this".to_owned())],
    );
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Enter), key(egui::Key::Escape)],
    );
    assert!(app.workspace.rename.is_none());
    assert_eq!(app.workspace.scenes[0].name, original);
}

#[test]
fn rename_limits_unicode_input_without_cutting_a_character() {
    let ctx = egui::Context::default();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    start_rename(&mut app, &ctx);
    frame(&mut app, &ctx, vec![egui::Event::Paste("Ж".repeat(200))]);
    assert_eq!(
        app.workspace
            .rename
            .as_ref()
            .expect("rename remains open")
            .1
            .chars()
            .count(),
        128
    );
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.workspace.scenes[0].name, "Ж".repeat(128));
}

#[test]
fn scene_controls_follow_split_order_even_when_one_view_is_hidden() {
    let ctx = egui::Context::default();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    let original = app.workspace.scenes[0].key;
    app.active_context()
        .expect("active scene")
        .queue_new_scene(SplitSide::Left);
    app.apply_workspace_commands(&ctx);
    let other = app
        .workspace
        .scenes
        .iter()
        .find(|scene| scene.key != original)
        .expect("new scene");
    let other_key = other.key;
    let other_pane = other.pane;
    assert_eq!(
        app.workspace
            .summaries()
            .iter()
            .map(|scene| scene.key)
            .collect::<Vec<_>>(),
        vec![other_key, original]
    );
    app.workspace.saved_split = Some(app.workspace.layout);
    app.workspace.layout = WorkspaceLayout::single(other_pane);
    assert_eq!(
        app.workspace
            .summaries()
            .iter()
            .map(|scene| scene.key)
            .collect::<Vec<_>>(),
        vec![other_key, original]
    );
}

#[test]
fn drop_prompt_keeps_long_names_in_bounds_and_queues_only_the_chosen_scene() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    app.active_context()
        .expect("active scene")
        .queue_new_scene(SplitSide::Left);
    app.apply_workspace_commands(&ctx);
    let chosen = app.workspace.scenes[0].key;
    let long_name = "Upper arch ".repeat(12);
    app.workspace.scenes[0].name.clone_from(&long_name);
    app.workspace.pending_drop = Some(vec![PathBuf::from("queued.stl")]);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let area = ctx
        .memory(|memory| memory.area_rect(egui::Id::new("workspace-drop-target-dialog")))
        .expect("drop dialog");
    assert!(
        ctx.content_rect().contains_rect(area),
        "dialog escaped screen: {area:?}"
    );
    let update = output
        .platform_output
        .accesskit_update
        .expect("accessible drop choices");
    let bounds = update
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.role() == egui::accesskit::Role::Button
                && node.label() == Some(long_name.as_str()))
            .then(|| node.bounds())
            .flatten()
            .filter(|bounds| area.contains(accessible_center(*bounds)))
        })
        .expect("named scene choice");
    let point = accessible_center(bounds);
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(
        app.workspace.pending_drop.is_none(),
        "choice point {point:?}, dialog {area:?}, layer {:?}",
        ctx.layer_id_at(point)
    );
    assert_eq!(app.loader.queued.len(), 1);
    assert_eq!(app.loader.queued[0].scene_key, chosen);
}

#[test]
fn focused_divider_cancels_with_escape_and_defers_f6_until_release() {
    let ctx = egui::Context::default();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    app.active_context()
        .expect("active scene")
        .queue_new_scene(SplitSide::Right);
    app.apply_workspace_commands(&ctx);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let original_layout = app.workspace.layout;
    let original_active = app.workspace.active_id();
    let start = egui::pos2(500.0, 400.0);
    let end = egui::pos2(650.0, 400.0);
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("workspace-scene-divider")));
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(end), key(egui::Key::F6)],
    );
    assert_ne!(app.workspace.layout, original_layout);
    assert_eq!(app.workspace.active_id(), original_active);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(app.workspace.layout, original_layout);
    assert_eq!(app.workspace.active_id(), original_active);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_ne!(app.workspace.active_id(), original_active);
}

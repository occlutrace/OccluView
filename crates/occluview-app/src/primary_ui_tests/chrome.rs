use super::*;

#[test]
fn primary_camera_action_labels_stay_generic() {
    let labels = primary_camera_action_labels();

    assert!(
        labels.is_empty(),
        "primary toolbar should not expose persistent camera action buttons"
    );
    for dental_label in ["Occlusal", "Buccal", "Lingual", "Mesial", "Distal"] {
        assert!(
            !labels.contains(&dental_label),
            "primary camera controls should not expose {dental_label}"
        );
    }
}

#[test]
fn viewport_has_no_persistent_mesh_stats_summary() {
    let app_source = app_module_source();

    assert!(
        !app_source.contains("viewport_status_summary"),
        "viewport should not expose mesh/count/size/color summary text"
    );
    assert!(
        !app_source.contains("has color"),
        "viewport should not advertise color metadata in the corner"
    );
    assert!(
        !app_source.contains("mesh ·"),
        "viewport should not show mesh-count summary copy"
    );
    assert!(
        !app_source.contains("{layer_count} open"),
        "layer overlay should not show a redundant open-count summary"
    );
}

#[test]
fn successful_appends_do_not_leave_status_overlay_copy() {
    let app_source = app_module_source();

    assert!(
        !app_source.contains("Added {loaded_count} layer"),
        "successful appends should not leave persistent bottom-left status copy"
    );
    assert!(
        !app_source.contains("self.status_message = append.then"),
        "append success should clear transient loading status instead of replacing it"
    );
}

#[test]
fn toolbar_and_about_are_operator_focused() {
    let dialogs = app_dialogs_source();
    let toolbar = function_source(dialogs, "pub(super) fn show_toolbar_impl");

    assert!(
        dialogs.contains("egui::TopBottomPanel::top(\"toolbar\")")
            && dialogs.contains(".exact_height(ui_theme::MENUBAR_HEIGHT_PX)"),
        "top toolbar should be a compact fixed-height operator surface"
    );
    assert!(
        !toolbar.contains("ui.menu_button(\"File\"")
            && !toolbar.contains("ui.menu_button(\"View\"")
            && !toolbar.contains("ui.menu_button(\"Help\""),
        "the windows-style File/View/Help menubar is retired: actions are direct \
         toolbar buttons (owner decision)"
    );
    assert!(
        toolbar.contains("toolbar_action(") && toolbar.contains("Cut view"),
        "toolbar should expose the real actions as direct flat buttons"
    );
    assert!(
        !toolbar.contains("Save edits"),
        "edited layers belong to the explicit editor/close guard, not the global header"
    );
    assert!(
        !toolbar.contains("Screenshot"),
        "the screenshot action was removed; the toolbar should not reference it"
    );
    assert!(
        toolbar.contains("Recent files") && toolbar.contains("Clear recent"),
        "recent scans should stay reachable from a slim dropdown beside Open"
    );
    assert!(
        toolbar.contains("consume_shortcut(&open_shortcut)")
            && toolbar.contains("format_shortcut(&open_shortcut)"),
        "shown keyboard hints must be backed by a real, wired shortcut (no dead hints)"
    );
    assert!(
        !toolbar.contains("CARGO_PKG_VERSION") && !toolbar.contains("app_logo_texture(ctx)"),
        "toolbar stays brand-light: no logo, no version stamp"
    );
    assert!(
        !app_chrome_source().contains("fn paint_version_stamp(")
            && !app_render_source().contains("paint_version_stamp("),
        "the viewport carries no version watermark"
    );
    assert!(
        dialogs.contains("About OccluView")
            && function_source(dialogs, "pub(super) fn show_about_window")
                .contains("self.app_logo_texture(ctx)")
            && dialogs.contains("https://occlutrace.ai")
            && dialogs.contains("https://github.com/occlutrace/OccluView"),
        "About shows the product, logo, the occlutrace.ai link, and the GitHub link"
    );
    assert!(
        !dialogs.contains("Dental Cloud"),
        "About must not surface a company name (owner decision), let alone twice"
    );
}

#[test]
fn layer_overlay_does_not_clone_full_scene_each_repaint() {
    let layer_source = repo_source_file("src/layers_overlay/mod.rs");
    let viewport_source = app_viewport_source();
    let layer_edits = app_layer_edits_source();

    assert!(
        layer_source.contains("let mut layer_edits = Vec::new();"),
        "layer overlay should collect lightweight edits while painting"
    );
    assert!(
        viewport_source
            .contains("if changes.context_request.is_none() && changes.layer_edits.is_empty()"),
        "full scene mutation should happen only after a real layer edit"
    );
    assert!(
        viewport_source
            .find("if changes.context_request.is_none() && changes.layer_edits.is_empty()")
            < viewport_source.find("let mut draft = scene.clone();"),
        "layer overlay must return before deep-cloning mesh payloads on repaint-only frames"
    );
    assert!(
        !viewport_source.contains("let mut draft = (*scene).clone();"),
        "layer overlay must not deep-clone mesh payloads before every repaint"
    );
    assert!(
        layer_edits.contains("let entry = scene.meshes().get(request.index)?;")
            && layer_edits.contains("if entry.id() != request.layer_id"),
        "stale layer delete indices should not panic the viewer"
    );
}

#[test]
fn layer_overlay_is_split_by_responsibility_not_single_file() {
    let facade_source = repo_source_file("src/layers_overlay/mod.rs");
    let facade = facade_source
        .split_once("\n#[cfg(test)]")
        .map_or(facade_source.as_str(), |(source, _)| source);
    let row = repo_source_file("src/layers_overlay/row.rs");
    let menu = repo_source_file("src/layers_overlay/menu.rs");
    let layout = repo_source_file("src/layers_overlay/layout.rs");
    let label = repo_source_file("src/layers_overlay/label.rs");

    assert!(
        facade.contains("mod layout;")
            && facade.contains("mod row;")
            && facade.contains("mod menu;")
            && facade.contains("mod label;"),
        "layers overlay should be a private module directory with focused responsibilities"
    );
    assert!(
        facade.contains("pub(crate) fn show(")
            && facade.contains("pub(crate) use label::layer_label;"),
        "layers overlay facade should keep the existing crate API"
    );
    assert!(
        !facade.contains("fn show_layer_row(") && !facade.contains("fn show_layer_context_menu("),
        "layers overlay facade should stay thin instead of owning row/menu implementation"
    );
    assert!(
        row.contains("fn show_layer_row(")
            && menu.contains("fn show_layer_context_menu(")
            && layout.contains("fn layer_overlay_rect(")
            && label.contains("fn layer_label("),
        "layers overlay responsibilities should live in their focused modules"
    );
}

#[test]
fn app_internals_are_split_by_responsibility_not_support_bucket() {
    let app_module = app_module_source();

    assert!(
        app_module.contains("mod app_layer_edits;")
            && app_module.contains("mod app_load_errors;")
            && app_module.contains("mod app_scale_bar;"),
        "app internals should name focused responsibilities instead of a generic support bucket"
    );
    assert!(
        !app_module.contains("mod app_support;"),
        "app_support.rs should not become a mixed helper bucket again"
    );
}

#[test]
fn edit_shortcut_stays_with_viewport_input_not_layer_mutation() {
    let layer_edits = app_layer_edits_source();
    let viewport = app_viewport_source();

    assert!(
        viewport.contains("input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)"),
        "mesh undo shortcut should live with viewport/input handling"
    );
    assert!(
        !layer_edits.contains("input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)"),
        "layer edit module should not own keyboard input plumbing"
    );
}

#[test]
fn empty_state_is_blank_instead_of_showing_drop_copy() {
    let central = function_source(
        app_render_source(),
        "pub(super) fn show_central_panel_impl(&mut self, ctx: &egui::Context) {",
    );

    assert!(
        !central.contains("ui.heading(\"OccluView 3D Viewer\")"),
        "blank viewer startup should not render branded empty-state copy inside the viewport"
    );
    assert!(
        !central.contains("Drop one or more 3D files to open them."),
        "blank viewer startup should stay visually quiet"
    );
    assert!(
        central.contains("self.show_status_overlay(ui, viewport_rect);"),
        "load/error status should still have a place to render over an empty viewport"
    );
}

#[test]
fn app_errors_are_copyable_dialogs_not_only_status_text() {
    let loading_source = app_loading_source();
    let render_source = app_render_source();
    let dialogs_source = app_dialogs_source();

    assert!(
        app_module_source().contains("app_error: Option<AppErrorDialog>"),
        "file/render failures should have a copyable error dialog state"
    );
    assert!(
        loading_source.contains("self.app_error = Some(load_error_dialog"),
        "loader failures must open the error dialog, not only write status text"
    );
    assert!(
        render_source.contains("title: \"Could not render scene\".to_string()"),
        "render failures should open a render-specific error dialog"
    );
    assert!(
        dialogs_source.contains("ui.ctx().copy_text(error.details.clone())"),
        "error dialog needs a Copy Details action"
    );
}

#[test]
fn unsaved_mesh_edits_guard_the_window_close() {
    let dialogs = repo_source_file("src/app/app_dialogs.rs");
    let guard = function_source(&dialogs, "pub(super) fn guard_unsaved_close(");

    assert!(
        guard.contains("close_requested()")
            && guard.contains("ViewportCommand::CancelClose")
            && guard.contains("self.has_unsaved_mesh_edits"),
        "closing with unsaved mesh edits must be intercepted, not silently lost"
    );
    assert!(
        guard.contains("Close without saving") && guard.contains("Cancel"),
        "the close guard must offer an explicit choice"
    );

    // Every mesh-edit apply path must mark the unsaved state.
    for file in [
        "src/app/app_layer_edits/whole_mesh.rs",
        "src/app/app_layer_edits/selection_ops.rs",
        "src/app/app_layer_edits/undo_redo.rs",
    ] {
        assert!(
            repo_source_file(file).contains("app.mark_mesh_edits_unsaved("),
            "{file} must mark unsaved mesh edits per layer so the save flow \
             knows exactly what to export"
        );
    }
}

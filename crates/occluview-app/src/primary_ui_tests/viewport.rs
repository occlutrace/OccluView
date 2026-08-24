use super::*;
use std::path::{Path, PathBuf};

#[test]
fn source_budget_guard_ignores_generated_target_directories() {
    let root = std::env::temp_dir().join(format!("occluview-line-budget-{}", std::process::id()));
    let collected = (|| -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(root.join("target"))
            .map_err(|error| format!("cannot create fixture: {error}"))?;
        std::fs::write(root.join("kept.rs"), "fn kept() {}\n")
            .map_err(|error| format!("cannot write kept fixture: {error}"))?;
        std::fs::write(root.join("target/generated.rs"), "fn generated() {}\n")
            .map_err(|error| format!("cannot write generated fixture: {error}"))?;
        let mut files = Vec::new();
        collect_rust_source_files(&root, &mut files)?;
        Ok(files)
    })();
    let _ = std::fs::remove_dir_all(&root);

    assert!(collected.is_ok(), "source collection failed: {collected:?}");
    let Ok(files) = collected else {
        return;
    };
    assert!(files.iter().any(|path| path.ends_with("kept.rs")));
    assert!(!files.iter().any(|path| path.ends_with("generated.rs")));
}

#[test]
fn rust_source_files_stay_within_the_physical_line_budget() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(Path::parent);
    assert!(
        workspace_root.is_some(),
        "app crate should live under the workspace crates directory"
    );
    let Some(workspace_root) = workspace_root else {
        return;
    };
    let mut source_files = Vec::new();
    let collected = collect_rust_source_files(&workspace_root.join("crates"), &mut source_files);
    assert!(collected.is_ok(), "source audit failed: {collected:?}");

    let oversized: Vec<String> = source_files
        .into_iter()
        .filter_map(|path| {
            let lines = std::fs::read_to_string(&path).ok()?.lines().count();
            (lines > 800).then(|| format!("{} ({lines})", path.display()))
        })
        .collect();

    assert!(
        oversized.is_empty(),
        "Rust source files must stay <= 800 lines:\n{}",
        oversized.join("\n")
    );
}

#[test]
fn mesh_editor_panel_has_no_single_target_selector() {
    let overlay = repo_source_file("src/mesh_editor_overlay.rs");
    let groups = repo_source_file("src/mesh_editor_groups.rs");
    let mesh_editor = repo_source_file("src/app/app_mesh_editor.rs");

    assert!(
        !overlay.contains("SwitchTarget("),
        "multi-layer mesh editing must not expose a single-target action"
    );
    assert!(
        !groups.contains("ComboBox::from_id_") && !groups.contains("section(ui, \"Target\")"),
        "mesh editor panel must not expose a target picker"
    );
    assert!(
        mesh_editor.contains("visible_selected_face_count")
            && mesh_editor.contains("select_all_visible_selections"),
        "panel state and bulk selection should use visible multi-layer summaries"
    );
    assert!(
        !mesh_editor.contains("switch_mesh_editor_target"),
        "app routing must not retain a hidden single-target switch path"
    );
}

#[test]
fn edit_mesh_entry_opens_one_scene_wide_session() {
    let layer_edits = repo_source_file("src/app/app_layer_edits/mod.rs");
    let mesh_editor = repo_source_file("src/app/app_mesh_editor.rs");

    assert!(
        layer_edits.contains("LayerContextAction::EditMesh")
            && layer_edits.contains("begin_face_selection_with_status("),
        "the existing RMB Edit mesh action should keep routing through the mesh-edit entry point"
    );
    assert!(
        layer_edits.contains("app.edit_mode.begin_face_selection(entry, scene)"),
        "RMB Edit mesh should open the shared scene edit session"
    );
    assert!(
        !mesh_editor.contains("switch_mesh_editor_target")
            && mesh_editor.contains("has_active_session()"),
        "the open panel should stay scene-wide instead of switching targets"
    );
}

#[test]
fn multi_layer_session_state_lives_in_its_own_module() {
    // Line budgets are enforced for every crate by
    // `rust_source_files_stay_within_the_physical_line_budget` above, which
    // walks the tree instead of naming sixteen files that can be renamed out
    // from under it. What the walk cannot see is the split itself.
    let edit_mode = repo_source_file("src/edit_mode/mod.rs");
    assert!(
        edit_mode.contains("mod selection_set;"),
        "multi-layer session state should live in a focused selection_set module"
    );
}

#[test]
fn viewport_double_click_focuses_scene_point_not_home_reset() {
    let source = app_viewport_source();
    let input = function_source(source, "pub(super) fn handle_viewport_input(");

    assert!(
        source.contains(
            "response.double_clicked() || response.clicked_by(egui::PointerButton::Middle)"
        ),
        "viewport input should pick a scene point for double-click focus"
    );
    assert!(
        appears_before(
            input,
            "if response.double_clicked() {",
            "if let Some(target) = scene_pick",
        ),
        "double click should focus the picked scene point before any fallback"
    );
    assert!(
        !input.contains(
            "if response.double_clicked() {\n                self.reset_camera_to_home();"
        ),
        "double click should not immediately reset the camera home"
    );
    assert!(
        !input.contains("else {\n                    self.reset_camera_to_home();"),
        "double click on empty viewport space should not surprise-reset the inspection camera"
    );
}

#[test]
fn viewport_face_selection_uses_typed_hit_before_camera_focus() {
    let source = app_viewport_source();
    let input = function_source(source, "pub(super) fn handle_viewport_input(");
    let click = function_source(source, "fn handle_primary_face_selection_click(");

    assert!(
        click.contains("self.edit_mode.has_active_session()"),
        "face selection should be gated by the scene-wide edit session"
    );
    assert!(
        click.contains("pick_scene_hit(&camera, response.rect, pointer, &scene)"),
        "face selection should use typed scene hits without the camera-focus AABB fallback"
    );
    assert!(
        appears_before(
            input,
            "self.handle_primary_face_selection_click(ctx, response)",
            "if response.double_clicked() {",
        ),
        "face selection should run before double-click camera focus consumes the pointer event"
    );
}

#[test]
fn viewport_mesh_edit_drag_selection_tracks_marquee_before_camera_branches() {
    let source = app_viewport_source();
    let input = function_source(source, "pub(super) fn handle_viewport_input(");
    let drag = function_source(source, "fn begin_mesh_selection_drag(");
    let track = function_source(source, "fn track_mesh_selection_drag(");

    assert!(
        drag.contains("response.drag_started_by(egui::PointerButton::Primary)"),
        "mesh edit marquee selection should begin from an explicit primary drag gate"
    );
    assert!(
        drag.contains("self.mesh_selection_drag = Some("),
        "viewport input should keep marquee state in app state while dragging"
    );
    assert!(
        drag.contains("MeshSelectionDrag::Rect"),
        "default primary drag should track a marquee rectangle"
    );
    let lasso = function_source(source, "fn track_polygon_lasso(");
    assert!(
        lasso.contains("button_pressed(egui::PointerButton::Primary)")
            && lasso.contains("lasso_capture::decide(&frame)"),
        "armed lasso must place points on the primary PRESS edge and route the decision \
         through the pure lasso_capture state machine (egui drops moved clicks, so \
         press-based capture is the only way input is never lost)"
    );
    assert!(
        lasso.contains("MeshSelectionDrag::Lasso") && lasso.contains("LassoEvent::Close"),
        "armed lasso must accumulate outline points and close through the state machine"
    );
    let commit = function_source(source, "fn commit_screen_polygon_selection(");
    assert!(
        commit.contains("self.edit_mode.select_faces_in_screen_polygon("),
        "marquee and lasso must commit through the single screen-polygon selection API"
    );
    assert!(
        track.contains("self.commit_screen_polygon_selection(")
            && lasso.contains("self.commit_screen_polygon_selection("),
        "both capture modes must share the polygon commit helper"
    );
    assert!(
        input.contains(
            "self.track_mesh_selection_drag(ctx, response, viewport_rect, pan_drag_active)"
        ),
        "viewport input should dispatch mesh marquee selection through its dedicated helper"
    );
    assert!(
        appears_before(
            input,
            "self.track_mesh_selection_drag(ctx, response, viewport_rect, pan_drag_active)",
            "self.handle_primary_face_selection_click(ctx, response)",
        ),
        "drag-selection commit should run before the single-click face selection branch"
    );
}

#[test]
fn armed_lasso_places_points_on_press_through_pure_state_machine() {
    let source = app_viewport_source();
    let lasso = function_source(source, "fn track_polygon_lasso(");
    let input = function_source(source, "pub(super) fn handle_viewport_input(");

    // Root cause of the "clicks do nothing, only stray angular segments" bug:
    // egui only fires `clicked` when the pointer moved < max_click_dist (6px)
    // between press and release, so a fast hand's clicks were reclassified as
    // drags and dropped. The rebuilt capture reads the primary PRESS edge.
    assert!(
        lasso.contains("button_pressed(egui::PointerButton::Primary)"),
        "armed lasso must add points on the primary press edge, not click-release"
    );
    assert!(
        lasso.contains("input.pointer.button_down(egui::PointerButton::Primary)"),
        "armed lasso must sample freehand points while the primary button is held"
    );
    assert!(
        lasso.contains("lasso_capture::decide(&frame)"),
        "the viewport must delegate capture decisions to the pure state machine"
    );
    // While armed, the lasso owns every primary gesture: the single-click face
    // pick must be suppressed so a lasso click cannot also select a face.
    assert!(
        appears_before(
            input,
            "!self.edit_mode.lasso_armed()",
            "self.handle_primary_face_selection_click(ctx, response)",
        ),
        "face pick must be gated off while the lasso owns primary clicks"
    );

    let module = repo_source_file("src/viewer/lasso_capture.rs");
    assert!(
        module.contains("pub(crate) fn decide(") && module.contains("enum LassoEvent"),
        "the pure lasso state machine must expose decide()/LassoEvent"
    );
    assert!(
        module.contains("#[cfg(test)]") && module.contains("fn fast_press_still_places_a_point"),
        "the pure lasso state machine must be unit-tested headlessly, including the \
         fast-press regression that motivated the redesign"
    );
}

#[test]
fn viewport_right_click_opens_shared_layer_menu_without_breaking_orbit() {
    let source = app_viewport_source();
    let input = function_source(source, "pub(super) fn handle_viewport_input(");
    let secondary_context = function_source(source, "fn handle_viewport_secondary_context_menu(");
    let orbit = function_source(source, "fn update_viewport_orbit_gesture(");
    let menu = function_source(source, "fn handle_viewport_context_menu(");
    let interaction = repo_source_file("src/app/app_layer_interaction.rs");

    assert!(
        menu.contains("response.secondary_clicked()"),
        "the viewport layer menu should open on a stationary right-click"
    );
    assert!(
        interaction.contains("fn discard_lasso_outline")
            && appears_before(
                menu,
                "discard_lasso_outline(&mut self.mesh_selection_drag)",
                "viewport_menu_target_id",
            ),
        "a stationary right-click must drop an in-progress lasso outline before \
         opening the shared menu, so it can safely switch the target in one click"
    );
    assert!(
        menu.contains("layers_overlay::show_layer_context_menu("),
        "viewport right-click must reuse the shared layer context menu, not fork a new one"
    );
    assert!(
        menu.contains("self.apply_layer_overlay_changes("),
        "viewport menu actions must route through the existing layer-apply path"
    );
    assert!(
        input.contains("self.handle_viewport_secondary_context_menu(")
            && secondary_context.contains("self.handle_viewport_context_menu(ctx, response);"),
        "viewport input should dispatch the right-click layer menu"
    );
    assert!(
        orbit.contains("sample.down && response.is_pointer_button_down_on()")
            && orbit.contains("viewport_orbit_drag_active(")
            && orbit.contains("pan_drag_active || orbit_drag_active")
            && input.contains("orbit_delta_from_drag(secondary_pointer.motion"),
        "secondary camera motion must start immediately and suppress the stationary context menu"
    );
}

#[test]
fn update_runs_camera_cleanup_before_render_and_ui_pass() {
    let app_source = app_module_source();
    let update = function_source(
        app_source,
        "fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {",
    );
    let central = function_source(
        app_render_source(),
        "pub(super) fn show_central_panel(&mut self, ctx: &egui::Context) {",
    );
    let render_pending = function_source(
        app_render_source(),
        "pub(super) fn render_pending_frame(&mut self, ctx: &egui::Context) {",
    );

    assert!(
        appears_before(
            update,
            "self.render_pending_frame(ctx);",
            "self.show_central_panel(ctx);",
        ),
        "the pending frame render pass should still happen before the main panel where camera input is collected"
    );
    assert!(
        update.rfind("self.render_pending_frame(ctx);")
            > update.find("self.show_central_panel(ctx);"),
        "a second pending-frame pass must follow viewport input so the live paint callback \
         encodes THIS frame's camera (removes one frame of orbit latency)"
    );
    assert!(
        central.contains("self.show_viewport_overlays(ui, &response, ctx);"),
        "the main viewport panel should hand off to the shared overlay body"
    );
    assert!(
        app_render_source().contains("self.handle_viewport_input(ctx, response, response.rect,"),
        "camera input should still be collected once the overlays have had the pointer"
    );
    assert!(
        render_pending.contains("if self.needs_render {")
            && render_pending.contains("self.sync_live_viewport();")
            && render_pending.contains("self.render_now(ctx);"),
        "pending frame rendering should stay centralized in one helper"
    );
}

#[test]
fn viewport_input_uses_shared_camera_repaint_helper_for_all_camera_mutations() {
    let input = function_source(
        app_viewport_source(),
        "pub(super) fn handle_viewport_input(",
    );
    let repaint_helper = function_source(
        app_module_source(),
        "fn request_camera_repaint(&mut self, ctx: &egui::Context) {",
    );

    assert!(
        count_occurrences(input, "self.request_camera_repaint(ctx);") >= 3,
        "camera mutation paths should route through the shared camera repaint helper"
    );
    assert!(
        count_occurrences(input, "camera.target = target;") == 2,
        "scene targeting should still have two target-setting branches"
    );
    assert!(
        repaint_helper.contains("self.needs_render = true;"),
        "the shared camera repaint helper should still mark the viewport dirty"
    );
    assert!(
        repaint_helper.contains("self.mark_camera_modified();"),
        "the shared camera repaint helper should still track camera mutation"
    );
    assert!(
        repaint_helper.contains("ctx.request_repaint();"),
        "the shared camera repaint helper should still request a repaint"
    );
}

#[test]
fn the_scale_bar_reads_the_camera_and_not_the_scene() {
    let app_render = app_render_source();
    let scale_bar = repo_source_file("src/app/app_scale_bar.rs");

    assert!(
        !app_render.contains("SceneStats"),
        "the cached scene bounding box existed only to size the scale bar; nothing \
         should be caching it now"
    );
    assert!(
        scale_bar.contains("camera.orthographic_height"),
        "an orthographic view puts orthographic_height / viewport_height millimetres \
         in a pixel, and that is the only honest source for the bar's length"
    );
    assert!(
        !scale_bar.contains("bbox_mm"),
        "the scene's own size is the framing it had when it loaded, not the framing \
         on screen after the operator has zoomed"
    );
}

#[test]
fn viewport_orbit_grabs_cursor_while_secondary_dragging() {
    let app_source = app_module_source();
    let viewport_source = app_viewport_source();

    assert!(
        app_source.contains("viewport_orbit_cursor_grabbed: bool"),
        "app state should remember whether viewport orbit currently owns the cursor"
    );
    assert!(
        viewport_source.contains("egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked)")
            && viewport_source
                .contains("egui::ViewportCommand::CursorGrab(egui::CursorGrab::None)"),
        "RMB orbit should lock the cursor during drag and always release it afterwards"
    );
    assert!(
        viewport_source.contains("egui::ViewportCommand::CursorVisible(false)")
            && viewport_source.contains("egui::ViewportCommand::CursorVisible(true)"),
        "cursor should hide only while locked for uninterrupted orbit"
    );
    assert!(
        viewport_source.contains("self.release_viewport_orbit_cursor(ctx);"),
        "update should release cursor capture when the button/focus state no longer allows orbit"
    );
}

#[test]
fn viewport_primary_secondary_drag_pans_scene_before_orbit() {
    let input = function_source(
        app_viewport_source(),
        "pub(super) fn handle_viewport_input(",
    );
    let interaction_source = viewer_interaction_source();

    assert!(
        interaction_source.contains("fn viewport_pan_drag_active("),
        "left+right drag needs a named pan predicate instead of being hidden in orbit code"
    );
    assert!(
        interaction_source.contains("fn viewport_combined_pan_drag_active("),
        "combined primary+secondary panning should be testable without egui Response internals"
    );
    assert!(
        interaction_source.contains("response.is_pointer_button_down_on()")
            && !interaction_source.contains(
                "viewport_combined_pan_drag_active(primary_secondary_down, response.dragged())"
            ),
        "combined-button pan must start on raw sub-threshold motion instead of waiting for egui's drag threshold"
    );
    assert!(
        input.contains("viewport_pan_drag_active(ctx, response)"),
        "viewport input should pan when primary and secondary mouse buttons are held together"
    );
    assert!(
        input.contains("let pan_delta = secondary_pointer.motion;"),
        "active pan must consume raw pointer motion so its first sub-threshold movement is not lost"
    );
    assert!(
        appears_before(
            input,
            "viewport_pan_drag_active(ctx, response)",
            "self.update_viewport_orbit_gesture(",
        ),
        "combined left+right pan must win before the secondary-button orbit branch"
    );
}

#[test]
fn cut_view_wires_clip_plane_into_viewport_and_preview() {
    let cut_tool = include_str!("../cut_tool.rs");
    let live_viewport = include_str!("../live_viewport.rs");
    let app_render = app_render_source();

    assert!(
        cut_tool.contains("pub(super) fn viewport_clip_plane(&self, bbox: Aabb) -> ClipPlane"),
        "cut tool should expose the active viewport clip plane"
    );
    assert!(
        cut_tool.contains("pub(super) fn cut_view_spec(&self, bbox: Aabb) -> Option<CutViewSpec>"),
        "cut tool should expose a separate preview render spec"
    );
    assert!(
        app_render.contains("self.active_viewport_clip_plane(scene.bbox())")
            && app_render.contains("render_prepared_viewport_with_clip_and_overlay("),
        "main viewport should render the active clipping plane, not only the small preview"
    );
    assert!(
        app_render.contains("fn render_section_pixels(")
            && app_render.contains(
                "offscreen.render_prepared_scene_with_clip(prepared, &camera, &plane, spec)"
            ),
        "section previews should reuse prepared GPU scene data instead of re-uploading meshes"
    );
    assert!(
        live_viewport.contains("clip_buffer") && live_viewport.contains("scene.draw_with_clip("),
        "live viewport should bind the same clip plane path as offscreen render"
    );
    // Every viewport-owning tool must gate camera input only while it consumes
    // its pointer gesture: Bridge Split, Cut View, Measure, and Align Scans all
    // coexist with ordinary orbit/pan when idle.
    // Matched flag by flag rather than as one literal: rustfmt wraps a long
    // condition, and a contract that breaks on whitespace protects nothing.
    for flag in [
        "!bridge_ui_consumed",
        "!cut_ui_consumed",
        "!measure_ui_consumed",
        "!align_ui_consumed",
    ] {
        assert!(
            app_render.contains(flag),
            "{flag} must gate the camera, or that tool's pointer leaks into orbit/pan"
        );
    }
    assert!(
        app_render.contains("self.handle_viewport_input(ctx, response, response.rect,"),
        "the camera path must still be reachable when no tool consumed the pointer"
    );
    // One body, called twice. `show_viewport_overlays` has what two copies of
    // it cost.
    assert_eq!(
        count_occurrences(
            app_render,
            "self.show_viewport_overlays(ui, &response, ctx);"
        ),
        2,
        "both viewport branches must call the one overlay body"
    );
    assert_eq!(
        count_occurrences(
            app_render,
            "let cut_ui_consumed = self.show_cut_tool_overlay("
        ),
        1,
        "the overlay orchestration must exist exactly once"
    );
}

#[test]
fn layer_material_edits_do_not_reset_prepared_scene() {
    let viewport_source = app_viewport_source();
    let app_render = app_render_source();

    assert!(
        app_render.contains("pub(super) fn update_scene_materials(&mut self, scene: Scene)"),
        "opacity/tint/visibility edits need a lightweight scene-material update path"
    );
    assert!(
        viewport_source.contains("self.update_scene_materials(draft);"),
        "opacity/tint/visibility edits should not clear uploaded GPU mesh data"
    );
    assert!(
        app_render.contains("self.live_viewport_scene_dirty = self.live_viewport.is_some();"),
        "layer/material edits should mark only the live scene payload dirty, not rebuild on every camera move"
    );
}

#[test]
fn section_panel_owns_close_without_a_separate_hint_strip() {
    let cut_tool = repo_source_file("src/cut_tool.rs");
    let panel = repo_source_file("src/cut_ruler/panel.rs");
    let cut_measure = repo_source_file("src/app/app_cut_measure.rs");

    assert!(!cut_tool.contains("show_strip(") && !cut_tool.contains("strip_rect("));
    assert!(
        panel.contains("SectionPanelCommand::Close") && panel.contains("Close section"),
        "the shared Section header should own one explicit close control"
    );
    assert!(
        cut_measure.contains("panel.thickness_changed && measure_owned"),
        "plain Cut View thickness must not leak into the main measurement overlay"
    );
}

#[test]
fn camera_only_live_viewport_redraw_skips_scene_resync() {
    let app_source = app_module_source();
    let sync = app_render_source();

    assert!(
        app_source.contains("live_viewport_scene_dirty: bool"),
        "app state should track whether GPU scene payloads actually changed"
    );
    assert!(
        sync.contains("viewport.update_view(&gpu_cam, self.render_extent_px, clip_plane);"),
        "camera/viewport changes should update view state without forcing a scene upload"
    );
    assert!(
        sync.contains("if self.live_viewport_scene_dirty {"),
        "scene uploads should be conditional on actual scene changes"
    );
    assert!(
        sync.contains("viewport.sync_scene(&sources, &updates);"),
        "only scene mutations should touch prepared scene synchronization"
    );
}

#[test]
fn camera_only_offscreen_redraw_skips_scene_resync() {
    let app_source = app_module_source();
    let render_pixels = app_render_source();

    assert!(
        app_source.contains("offscreen_scene_dirty: bool"),
        "offscreen fallback should track whether scene GPU state actually changed"
    );
    assert!(
        render_pixels.contains("if self.offscreen_scene_dirty {"),
        "camera-only offscreen redraws should not rewrite layer uniforms every frame"
    );
    assert!(
        render_pixels.contains("self.offscreen_scene_dirty = false;"),
        "offscreen scene sync should clear its dirty bit after the upload/update path runs"
    );
}

#[test]
fn live_window_uses_matching_msaa_for_custom_wgpu_viewport() {
    let source = app_bootstrap_source();
    let live_viewport = include_str!("../live_viewport.rs");
    let start = source.rfind("let native_options = eframe::NativeOptions");
    assert!(start.is_some(), "missing native_options");
    let Some(start) = start else {
        return;
    };
    let end = source[start..].find("eframe::run_native(");
    assert!(end.is_some(), "missing run_native after native_options");
    let Some(end) = end else {
        return;
    };
    let native_options = &source[start..start + end];

    assert!(
        native_options.contains("multisampling: LIVE_VIEWPORT_SAMPLE_COUNT"),
        "eframe MSAA must use the same sample-count constant as the live custom renderer"
    );
    assert!(
        live_viewport.contains("Renderer::with_shared_device_sample_count("),
        "custom live viewport pipelines must be built with the eframe render-pass sample count"
    );
    assert!(
        source.contains("LIVE_VIEWPORT_SAMPLE_COUNT")
            && live_viewport.contains("LIVE_VIEWPORT_SAMPLE_COUNT"),
        "the live viewport sample count should be a single shared constant"
    );
}

#[test]
fn live_window_requests_one_frame_of_swapchain_latency() {
    let source = app_bootstrap_source();
    let start = source.rfind("let native_options = eframe::NativeOptions");
    assert!(start.is_some(), "missing native_options");
    let Some(start) = start else {
        return;
    };
    let end = source[start..].find("eframe::run_native(");
    assert!(end.is_some(), "missing run_native after native_options");
    let Some(end) = end else {
        return;
    };
    let native_options = &source[start..start + end];

    assert!(
        native_options.contains("desired_maximum_frame_latency: Some(1)"),
        "the interactive viewport must not queue multiple already-stale camera frames"
    );
}

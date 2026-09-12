#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

/// Source contract for the destroyed-texture submit crash. The per-frame
/// render paths must update ONE persistent egui texture id in place
/// (`TextureHandle::set` / `CutTool::store_slice`), never allocate a fresh id
/// per render. A fresh id can free a texture after this frame has painted it.
#[test]
fn per_frame_render_paths_reuse_persistent_texture_ids() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"))
        .replace("\r\n", "\n");
    assert!(
        source.contains("frame.texture.set(color_image, egui::TextureOptions::LINEAR)"),
        "render_now must update the viewport texture in place, not reallocate it"
    );
    assert!(
        source.contains("self.tools.cut_view.store_slice(ctx, color_image, slice_cam)"),
        "render_cut_now must route the slice through CutTool::store_slice"
    );
    assert!(
        !source.contains("load_texture(\"occluview-cut\""),
        "the cut slice must not allocate a fresh egui texture id per render"
    );
}

/// A deviation map must reach the screen unlit and in its own measured colors.
#[test]
fn a_deviation_overlay_forces_unlit_vertex_colors() {
    use glam::Vec3;
    use occluview_core::scene::SceneMesh;
    use occluview_core::{Mesh, Vertex};

    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh");

    let mut entry = SceneMesh::new(mesh);
    entry.show_vertex_colors = false;
    entry.show_texture = true;

    let plain = super::scene_mesh_uniform(&entry);
    assert_eq!(plain.measured_map, 0);
    assert_eq!(plain.show_vertex_colors, 0);

    let colors = std::sync::Arc::new(vec![[0u8, 0, 0, 255]; 3]);
    let mapped = super::scene_mesh_uniform(&entry.with_deviation(Some(colors)));
    assert_eq!(mapped.measured_map, 1, "a deviation map must draw unlit");
    assert_eq!(mapped.show_vertex_colors, 1);
    assert_eq!(mapped.show_texture, 0);
}

#[test]
fn the_offscreen_viewport_replays_overlay_vertices_after_scene_upload() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));
    assert!(
        source.contains("fn push_deviation_colors_offscreen"),
        "the fallback viewport needs its own overlay upload path"
    );
    assert!(
        source.contains("prepared.write_entry_vertices(offscreen.renderer()"),
        "offscreen scene uploads must receive the measured colours"
    );
    // The body, not the rest of the file: the upload helper is *defined* below
    // this method, so "everything after the signature" was satisfied by the
    // definition even after the call was gone.
    let body = crate::primary_ui_tests::method_body(source, "pub(super) fn render_scene_pixels");
    assert!(
        !body.is_empty(),
        "render_scene_pixels must exist and end at the impl indentation"
    );
    assert!(
        body.contains("push_deviation_colors_offscreen()"),
        "render_scene_pixels must restore a map after rebuilding its prepared scene"
    );
}

#[test]
fn a_failed_offscreen_frame_cannot_start_a_repaint_storm() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));
    let render_now = crate::primary_ui_tests::method_body(source, "pub(super) fn render_now");
    assert!(
        render_now.contains("self.note_offscreen_failure_anyhow(&e)"),
        "render_now must consume and classify a failed offscreen frame"
    );
    let pending =
        crate::primary_ui_tests::method_body(source, "pub(super) fn render_pending_frame");
    assert!(
        pending.contains("self.offscreen_available()") && pending.contains("consume_redraw()"),
        "the pending-frame path must stop retrying an unavailable offscreen path"
    );
    let note = crate::primary_ui_tests::method_body(source, "fn note_offscreen_failure(&mut self");
    assert!(
        note.contains("terminal_offscreen_render_error(error)"),
        "a failure must be classified before the path is latched or deferred"
    );
}

/// A missed readback deadline is a liveness bound, not a device verdict. It
/// used to latch the whole offscreen path off for the session: the section
/// panel kept showing the previous plane and, with no live viewport, the
/// viewport stopped repainting at all, on hardware that was never shown to be
/// broken and with no control that could clear it.
#[test]
fn a_readback_deadline_defers_the_offscreen_path_instead_of_killing_it() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));

    // The body, not the doc comment: the prose above these functions explains
    // the deadline case and would satisfy a whole-text search on its own.
    let terminal_body = source
        .split_once("fn terminal_offscreen_render_error")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body)
        .unwrap_or_default();
    assert!(
        !terminal_body.contains("ReadbackTimeout"),
        "a deadline must not be classified as a broken graphics stack"
    );
    assert!(
        terminal_body.contains("RenderError::Surface"),
        "a real surface failure still latches the path off"
    );
    let retryable_body = source
        .split_once("fn retryable_offscreen_render_error")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body)
        .unwrap_or_default();
    assert!(
        retryable_body.contains("ReadbackTimeout"),
        "a deadline is the failure the offscreen path must retry"
    );
    assert!(
        !retryable_body.contains("RenderError::Surface"),
        "a broken stack is not retried on a timer"
    );
    assert!(
        source.contains(
            "self.render.offscreen_retry_after = Some(Instant::now() + OFFSCREEN_RETRY_DELAY)"
        ),
        "the retry must be deferred so a loaded machine is not asked to fail on a loop"
    );
}

/// The fault dialog has to offer a way back. Clearing the flag is the only
/// recovery this app documents short of closing the viewer and losing the
/// scene, so the dialog must carry the action and the frame loop must route it
/// to the renderer that latched the fault.
#[test]
fn the_graphics_fault_dialog_offers_the_retry_action() {
    let render = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));
    let poll = crate::primary_ui_tests::method_body(render, "pub(super) fn poll_gpu_errors");
    assert!(!poll.is_empty(), "the GPU error poll must exist");
    assert!(
        poll.contains("action: AppErrorAction::RetryGraphics"),
        "a graphics fault must be reported with an offered recovery"
    );

    let dialogs = crate::primary_ui_tests::production_source(include_str!("app_dialogs.rs"));
    assert!(
        dialogs.contains("error.action == AppErrorAction::RetryGraphics"),
        "the dialog must render the retry button only for an actionable error"
    );
    assert!(
        dialogs.contains("self.retry_gpu_after_fault(ctx)"),
        "the retry button must reach the renderer"
    );

    let retry = crate::primary_ui_tests::method_body(render, "pub(super) fn retry_gpu_after_fault");
    assert!(
        retry.contains("viewport.clear_gpu_fault()"),
        "retrying graphics must clear the latch the paint path obeys"
    );
}

/// The deferred retry has to survive the frames that arrive during its wait.
///
/// `ensure_offscreen` runs on the section path, which the operator drives by
/// dragging the cut plane — the very action that causes a readback to miss its
/// deadline. When it refused with a bare string, the caller could not classify
/// the cause and latched the path off permanently: one drag inside the 750 ms
/// window and the section panel went dead for the session, which is the state
/// the deferral exists to avoid.
#[test]
fn a_frame_during_the_retry_wait_cannot_latch_the_offscreen_path_off() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));
    let ensure = crate::primary_ui_tests::method_body(source, "pub(super) fn ensure_offscreen");
    assert!(!ensure.is_empty(), "ensure_offscreen must exist");
    let deferral = ensure
        .split_once("if !self.offscreen_available()")
        .and_then(|(_, rest)| rest.split_once("if self.render.offscreen_failed"))
        .map(|(body, _)| body)
        .unwrap_or_default();
    assert!(
        deferral.contains("Error::new(RenderError::ReadbackTimeout"),
        "a deferral must carry a typed cause, or the caller latches the path off"
    );
    assert!(
        !deferral.contains("anyhow!("),
        "a bare string cannot be classified by the caller"
    );

    // And the wait has to end by itself.
    let pending =
        crate::primary_ui_tests::method_body(source, "pub(super) fn render_pending_frame");
    assert!(
        pending.contains("request_repaint_after"),
        "the retry must schedule its own wake-up; otherwise it waits for input"
    );
    assert!(
        pending.contains("offscreen_retry_after"),
        "the wake-up must be derived from the retry deadline"
    );
}

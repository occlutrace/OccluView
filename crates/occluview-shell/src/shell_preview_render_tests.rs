//! First-frame and interaction contracts for the Explorer preview handler.

#![allow(clippy::expect_used)]

#[test]
fn do_preview_installs_a_first_paintable_bitmap_before_success() {
    let preview = include_str!("com/preview.rs");
    let do_preview_start = preview.find("fn DoPreview(").expect("DoPreview impl");
    let do_preview_end = preview[do_preview_start..]
        .find("fn Unload(")
        .map(|offset| do_preview_start + offset)
        .expect("Unload follows DoPreview");
    let do_preview = &preview[do_preview_start..do_preview_end];
    let first_frame_start = preview
        .find("fn render_first_preview_frame(")
        .expect("first-frame renderer");
    let first_frame_end = preview[first_frame_start..]
        .find("fn render_preview_now(&self")
        .map(|offset| first_frame_start + offset)
        .expect("interactive render follows first-frame renderer");
    let first_frame = &preview[first_frame_start..first_frame_end];

    assert!(do_preview.contains("self.this.ensure_preview_window().and_then(|hwnd| {"));
    assert!(do_preview.contains("self.this.render_first_preview_frame("));
    assert!(do_preview.contains("RenderDeadline::after(PREVIEW_FIRST_FRAME_TIMEOUT)"));
    assert!(first_frame.contains("self.replace_preview_bitmap(hbmp);"));
    assert!(first_frame.contains("InvalidateRect(Some(hwnd), None, false)"));
    assert!(
        !first_frame.contains("RDW_UPDATENOW"),
        "first-frame rendering installs pixels but lets the normal window paint them"
    );
}

#[test]
fn shell_stream_copies_consume_the_callers_original_deadline() {
    let thumbnail = include_str!("com/thumbnail_provider.rs");
    let preview = include_str!("com/preview.rs");

    assert!(thumbnail.contains("fn read_stream_until("));
    assert!(thumbnail.contains("deadline: Instant,"));
    assert!(thumbnail.contains("read_capped_stream_until("));
    assert!(thumbnail.contains("self.ensure_stream_bytes(request)"));
    assert!(thumbnail.contains("read_stream_until(&stream, request.response_deadline())"));
    assert!(preview.contains("read_stream_until(&stream, deadline.expires_at())"));
    assert!(
        !thumbnail.contains("ThumbnailProvider::read_stream(&stream)"),
        "Shell stream reads must not start an independent timeout policy"
    );
}

#[test]
fn low_integrity_preview_forwards_unhandled_accelerators_to_the_host_frame() {
    let com = include_str!("com.rs");
    let preview = include_str!("com/preview.rs");

    assert!(com.contains("IPreviewHandlerFrame"));
    let handler_start = preview
        .find("fn TranslateAccelerator(")
        .expect("TranslateAccelerator implementation");
    let handler_end = preview[handler_start..]
        .find("impl IOleWindow_Impl")
        .map(|offset| handler_start + offset)
        .expect("IOleWindow follows IPreviewHandler");
    let handler = &preview[handler_start..handler_end];

    assert!(handler.contains("pmsg.is_null()"));
    assert!(handler.contains("site.cast::<IPreviewHandlerFrame>()"));
    assert!(handler.contains("Interface::vtable(&frame).TranslateAccelerator"));
    assert!(handler.contains("Interface::as_raw(&frame)"));
    assert!(handler.contains("if hresult == S_OK"));
    assert!(handler.contains("return Err(s_false());"));
    assert!(
        !handler.contains("fn TranslateAccelerator(&self, _pmsg"),
        "the preview handler must not discard all host keyboard accelerators"
    );
}

#[test]
fn preview_reuses_one_process_shared_offscreen_renderer() {
    let factory = include_str!("offscreen_factory.rs");
    let load = include_str!("preview_scene/load.rs");
    let render = include_str!("preview_scene/render.rs");

    assert!(factory.contains("fn shared_shell_offscreen("));
    assert!(factory.contains("deadline: RenderDeadline"));
    assert!(factory.contains("fn discard_shared_shell_offscreen("));
    assert!(
        !factory.contains("fn prewarm_shared_shell_offscreen()"),
        "preview renderer initialization must have exactly one request owner, not a competing activation prewarm"
    );
    assert!(load.contains("let offscreen = shared_shell_offscreen(deadline)?;"));
    assert!(load.contains("fn from_file_with_deadline("));
    assert!(render.contains("fn render_rgba_with_background_with_deadline("));
    assert!(
        !factory.contains("RenderDeadline::after(PREVIEW_RENDERER_SETUP_TIMEOUT)"),
        "preview loads must not create a per-file wgpu device"
    );
    assert!(render.contains("discard_shared_shell_offscreen(&self.offscreen)"));
}

#[test]
fn preview_callbacks_refresh_through_the_explicit_frame_helper() {
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");

    let set_rect_start = preview.find("fn SetRect(").expect("SetRect impl");
    let set_rect_end = preview[set_rect_start..]
        .find("fn DoPreview(")
        .map(|offset| set_rect_start + offset)
        .expect("DoPreview follows SetRect");
    let set_window_start = preview.find("fn SetWindow(").expect("SetWindow impl");
    let set_window_end = preview[set_window_start..]
        .find("fn SetRect(")
        .map(|offset| set_window_start + offset)
        .expect("SetRect follows SetWindow");

    assert!(
        !preview[set_rect_start..set_rect_end].contains("render_scheduled_preview"),
        "SetRect must not use a deferred render callback"
    );
    assert!(
        !preview[set_window_start..set_window_end].contains("render_scheduled_preview"),
        "SetWindow must not use a deferred render callback"
    );
    let wm_size = window.find("WM_SIZE =>").expect("WM_SIZE arm");
    assert!(
        window[wm_size..].contains("handler.render_preview_now("),
        "WM_SIZE must refresh through the explicit bounded frame helper"
    );
    let do_preview_start = preview.find("fn DoPreview(").expect("DoPreview impl");
    let do_preview_end = preview[do_preview_start..]
        .find("fn Unload(")
        .map(|offset| do_preview_start + offset)
        .expect("Unload follows DoPreview");
    assert!(
        preview[do_preview_start..do_preview_end].contains("render_first_preview_frame"),
        "DoPreview must load and render its first frame before returning success"
    );
}

#[test]
fn preview_refreshes_use_one_deadline_for_load_and_render() {
    let preview = include_str!("com/preview.rs");
    let context_menu = include_str!("com/preview/context_menu.rs");

    assert!(preview.contains("fn render_preview_now(&self, deadline: RenderDeadline)"));
    assert!(
        !preview.contains("fn render_preview_now(&self)"),
        "interactive work must not create a second render budget after scene loading"
    );
    let drag_start = preview.find("fn update_drag(").expect("drag update");
    let drag_end = preview[drag_start..]
        .find("fn end_drag(")
        .map(|offset| drag_start + offset)
        .expect("drag end");
    let drag = &preview[drag_start..drag_end];
    assert!(
        drag.contains("let deadline = RenderDeadline::after(PREVIEW_INTERACTION_FRAME_TIMEOUT);")
    );
    assert!(drag.contains("self.ensure_preview_scene_loaded(deadline)"));
    assert!(drag.contains("self.render_preview_now(deadline)?;"));

    assert!(context_menu.contains("let deadline = RenderDeadline::after("));
    assert!(context_menu.contains("self.ensure_preview_scene_loaded(deadline)"));
    assert!(context_menu.contains("self.render_preview_now(deadline)?;"));
    assert!(context_menu.contains("theme.canvas_rgba(),\n                deadline,"));
}

#[test]
fn preview_has_no_deferred_message_delivery_protocol() {
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");

    for forbidden in [
        "WM_OCCLUVIEW_RENDER_PREVIEW",
        "pending_render_token",
        "schedule_preview_render",
        "render_scheduled_preview",
        "PostMessageW",
        "NEXT_PREVIEW_RENDER_TOKEN",
    ] {
        assert!(
            !preview.contains(forbidden) && !window.contains(forbidden),
            "preview delivery must not depend on the retired deferred protocol: {forbidden}"
        );
    }
}

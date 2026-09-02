#[test]
fn preview_renderer_keeps_the_reference_hardware_then_fallback_policy() {
    let factory = include_str!("../offscreen_factory.rs");
    assert!(factory.contains("Offscreen::new_prefer_hardware()"));
    assert!(factory.contains("offscreen.can_draw()"));
    assert!(factory.contains("Offscreen::new()"));
}

#[test]
fn com_thumbnail_provider_creates_one_request_before_selecting_its_source() {
    let com = include_str!("../com/thumbnail_provider.rs");
    let start = com
        .find("fn render_attempt(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_attempt");
    let body = &com[start..];

    let request = body
        .find("let request = ThumbnailRenderRequest::new(DEFAULT_THUMBNAIL_TIMEOUT);")
        .expect("one Shell request object");
    let file_render = body
        .find("try_render_thumbnail_file_with_request(&path, spec, request)")
        .expect("file render receives the original request");
    let stream_reservation = body
        .find("reserve_thumbnail_stream_job_for_request(request)")
        .expect("stream reservation receives the original request");

    assert!(request < file_render);
    assert!(request < stream_reservation);
    assert!(body.contains("self.ensure_stream_bytes(request)"));
    assert!(body.contains("read_stream_until(&stream, request.response_deadline())"));
}

use super::*;

#[test]
fn thumbnail_render_path_uses_parallel_renderer_pool_for_shell_bursts() {
    let render_path = [mod_source(), concurrency_source(), rendering_source()].join("\n");
    assert!(render_path.contains("static THUMBNAIL_RENDERER_POOL"));
    assert!(rendering_source().contains("ThumbnailRendererPool::shared()"));
    assert!(concurrency_source().contains("Condvar"));
    assert!(concurrency_source()
        .contains("Self::with_create(max_renderers, create_thumbnail_offscreen)"));
    // The pool claims its slot before the device exists, and waits for one it
    // does not own. Both need a way out: an unwind out of device creation must
    // give the slot back, and the wait must expire rather than park the decode
    // lane it is holding until the surrogate dies.
    assert!(concurrency_source().contains("panic::catch_unwind(AssertUnwindSafe(|| {"));
    assert!(concurrency_source().contains("(self.create)(deadline, adapter_policy)"));
    assert!(concurrency_source().contains(".wait_timeout(state, remaining)"));
    let legacy_single_renderer_gate = ["Mutex", "<Option<Offscreen>>"].concat();
    assert!(!render_path.contains(&legacy_single_renderer_gate));
    let factory = offscreen_factory_source();
    // Production Shell requests verified hardware first, while deterministic
    // test fixtures select the fallback explicitly. The offscreen policy owns
    // the triangle probe and fallback -- no process-global mode can silently
    // change a request after it has entered Explorer's cache path.
    assert!(factory.contains("fn shell_adapter_policy() -> AdapterPolicy"));
    assert!(factory.contains("AdapterPolicy::HardwareThenFallback"));
    assert!(factory.contains("fn test_adapter_policy() -> AdapterPolicy"));
    assert!(factory.contains("AdapterPolicy::FallbackOnly"));
    assert!(factory.contains("Offscreen::new_with_adapter_policy("));
    assert!(!factory.contains("SOFTWARE_RENDERER_ONLY"));
    assert!(render_path.contains("pub fn prewarm_thumbnail_renderer()"));
    assert!(render_path.contains("THUMBNAIL_RENDERER_PREWARM_TIMEOUT"));
    assert!(concurrency_source().contains("impl Drop for ThumbnailRendererPool"));
    assert!(concurrency_source().contains("std::mem::forget(offscreen)"));
}

#[test]
fn timeout_thumbnail_workers_join_render_test_guard() {
    let rendering = rendering_source();
    assert!(cache_source().contains("ThumbnailFileCacheKey"));
    assert!(loading_source().contains("prepare_stream_thumbnail_render"));
    assert!(
        rendering.contains("#[cfg(test)]")
            && rendering.contains("let _guard = crate::acquire_render_test_guard();")
    );
}

#[test]
fn thumbnail_preflight_carries_no_restartable_timeout_state() {
    assert!(
        !cache_source().contains("wait_timeout"),
        "the request object, not a preflight plan, owns the one end-to-end deadline"
    );
}

#[test]
fn thumbnail_renderer_factory_receives_the_request_deadline() {
    let factory = offscreen_factory_source();
    let concurrency = concurrency_source();
    assert!(
        factory.contains("fn create_thumbnail_offscreen(\n    deadline: RenderDeadline,"),
        "adapter/device creation must be given the request deadline"
    );
    assert!(
        !factory.contains("RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT)"),
        "the factory must not restart a six-second renderer budget"
    );
    assert!(
        !concurrency.contains("RENDERER_WAIT_CEILING"),
        "renderer checkout must use the caller deadline, not a hidden thirty-second ceiling"
    );
}

#[test]
fn shell_thumbnail_supersampling_stops_at_256px_to_control_burst_latency() {
    let small = rendering_source();
    assert!(small.contains("MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX: u16 = 256"));
}

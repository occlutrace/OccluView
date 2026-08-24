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
    assert!(concurrency_source().contains("panic::catch_unwind(AssertUnwindSafe(self.create))"));
    assert!(concurrency_source().contains(".wait_timeout(state, remaining)"));
    let legacy_single_renderer_gate = ["Mutex", "<Option<Offscreen>>"].concat();
    assert!(!render_path.contains(&legacy_single_renderer_gate));
    let factory = offscreen_factory_source();
    // Hardware where there is hardware, the software rasteriser where there is
    // not -- and a device that accepts commands while drawing nothing counts as
    // the second case, which is why the preference is checked and not assumed.
    assert!(factory.contains("!cfg!(test)"));
    assert!(factory.contains("Offscreen::new_prefer_hardware()"));
    assert!(factory.contains("pollster::block_on(offscreen.can_draw())"));
    assert!(factory.contains("Offscreen::new()"));
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
fn shell_thumbnail_supersampling_stops_at_256px_to_control_burst_latency() {
    let small = rendering_source();
    assert!(small.contains("MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX: u16 = 256"));
}

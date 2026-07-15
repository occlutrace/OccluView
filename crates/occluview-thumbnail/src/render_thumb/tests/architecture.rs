use super::*;

#[test]
fn thumbnail_render_path_uses_parallel_renderer_pool_for_shell_bursts() {
    let render_path = [mod_source(), concurrency_source(), rendering_source()].join("\n");
    assert!(render_path.contains("static THUMBNAIL_RENDERER_POOL"));
    assert!(rendering_source().contains("ThumbnailRendererPool::shared()"));
    assert!(concurrency_source().contains("Condvar"));
    assert!(concurrency_source().contains("create_thumbnail_offscreen()"));
    assert!(concurrency_source().contains("create_thumbnail_offscreen()"));
    let legacy_single_renderer_gate = ["Mutex", "<Option<Offscreen>>"].concat();
    assert!(!render_path.contains(&legacy_single_renderer_gate));
    let factory = offscreen_factory_source();
    assert!(factory.contains("cfg!(all(windows, not(test)))"));
    assert!(factory.contains("Offscreen::new_prefer_hardware()"));
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

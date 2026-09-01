//! Renderer-selection contracts kept apart from installer/package assertions.

#[test]
fn explorer_preview_requests_a_verified_hardware_renderer_with_fallback() {
    let factory = include_str!("offscreen_factory.rs");

    assert!(
        factory.contains("AdapterPolicy::HardwareThenFallback"),
        "the preview handler must request verified hardware before the fallback adapter"
    );
    assert!(factory.contains("Offscreen::new_with_adapter_policy("));
}

#[test]
fn class_activation_does_not_set_a_global_thumbnail_adapter_mode() {
    let com = include_str!("com.rs");

    assert!(
        !com.contains("use_software_renderer_only") && !com.contains("spawn_renderer_prewarm"),
        "adapter selection belongs to the rendering request, not COM class activation"
    );
}

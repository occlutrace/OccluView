#[test]
fn mesh_shader_uses_camera_relative_inspection_lighting() {
    let shader = include_str!("../shaders/mesh.wgsl");

    assert!(
        shader.contains("let camera_fill = normalize(view_dir * 0.72 - key * 0.20)"),
        "fill light should follow the camera so details remain readable while orbiting"
    );
    assert!(
        shader.contains("let rim_lit = pow(fresnel, 1.45)"),
        "rim cue should be view-relative instead of a fixed world-space direction"
    );
    assert!(
        shader.contains("0.50 + 0.36 * wrapped_key + 0.095 * fill_lit + 0.018 * rim_lit")
            && shader.contains("0.48,\n        1.05,")
            && shader.contains("let form_contrast = 0.96 + 0.055 * view_form + 0.018 * fresnel"),
        "form-giving studio light should keep a lit floor (no cast shadow) yet a full key/fill/rim swing so side walls read with depth"
    );
    assert!(
        shader.contains("let textured = mesh_uniform.has_texture != 0u")
            && shader.contains("mesh_uniform.show_texture != 0u")
            && shader.contains("let texture_glaze = select(0.0, 1.0, textured)")
            && shader.contains("let glaze_highlight ="),
        "textured scans should get a restrained glaze highlight without making untextured STL glossy, and the neutral-material toggle should suppress it too"
    );
    assert!(
        shader.contains("@builtin(front_facing) front_facing: bool")
            && shader.contains("BACKFACE_INSPECTION_TINT")
            && shader.contains("let backface_mix = select(0.0, 0.14, !front_facing)"),
        "mesh shader should give back-facing triangles only a faint cool tint (not a dark grey) so a flipped surface stays distinguishable without a half-shadow"
    );
    assert!(
        !shader.contains("back_falloff")
            && !shader.contains("- 0.018")
            && !shader.contains("normal_faces_away"),
        "even dental light must not add a moving back-falloff or grazing grey-wash half-shadow over dental surfaces"
    );
}

#[test]
fn gpu_error_latch_records_and_drains_once() {
    // The device error handler records into this latch; the app drains it each
    // frame. wgpu's default handler panics instead — a hard abort in release.
    let latch: super::GpuErrorLatch = std::sync::Arc::new(std::sync::Mutex::new(None));
    assert!(
        super::drain_gpu_error(&latch).is_none(),
        "fresh latch is empty"
    );

    super::record_gpu_error(&latch, "validation error: bad draw".to_string());
    assert_eq!(
        super::drain_gpu_error(&latch).as_deref(),
        Some("validation error: bad draw"),
        "a recorded error is surfaced once"
    );
    assert!(
        super::drain_gpu_error(&latch).is_none(),
        "draining clears the latch so the same fault is not reported forever"
    );
}

#[test]
// Poisoning a mutex requires a deliberate panic while a guard is held. (This
// can only happen in an unwinding build; the shipping binary is `panic = abort`
// where poison never occurs — the guard still keeps the poll crash-proof.)
#[allow(clippy::expect_used, clippy::panic)]
fn gpu_error_latch_poison_is_ignored_not_fatal() {
    // A worker that panics mid-record poisons the mutex. Draining a poisoned
    // latch must return None, never panic — the UI poll must not crash.
    let latch: super::GpuErrorLatch = std::sync::Arc::new(std::sync::Mutex::new(None));
    let poisoned = std::sync::Arc::clone(&latch);
    let _ = std::thread::spawn(move || {
        let _guard = poisoned.lock().expect("lock");
        panic!("poison the latch");
    })
    .join();
    assert!(
        super::drain_gpu_error(&latch).is_none(),
        "poisoned latch drains to None instead of panicking"
    );
}

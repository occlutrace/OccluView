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
    // Three separate assertions, not one `&&`: the golden scene is a flat
    // triangle facing the camera, so fresnel, rim, wrap and backface are all
    // near zero in it and this text is the only guard these terms have. A
    // combined assert reported "studio light and side walls" without saying
    // which of the three had moved.
    assert!(
        shader.contains("0.50 + 0.36 * wrapped_key + 0.095 * fill_lit + 0.018 * rim_lit"),
        "the studio light's key/fill/rim mix should keep its lit floor and its full swing"
    );
    assert!(
        shader.contains("0.48,\n        1.05,"),
        "the studio light's ambient floor and key gain should stay as tuned"
    );
    assert!(
        shader.contains("let form_contrast = 0.96 + 0.055 * view_form + 0.018 * fresnel"),
        "form contrast should stay view-relative so side walls read with depth"
    );
    for (fragment, why) in [
        (
            "let textured = mesh_uniform.has_texture != 0u",
            "the glaze must key off whether the mesh has a texture at all",
        ),
        (
            "mesh_uniform.show_texture != 0u",
            "the neutral-material toggle must suppress the glaze too",
        ),
        (
            "let texture_glaze = select(0.0, 1.0, textured)",
            "an untextured STL must not be made glossy by the glaze",
        ),
        (
            "let glaze_highlight =",
            "the glaze highlight itself must exist",
        ),
    ] {
        assert!(shader.contains(fragment), "{why}");
    }
    for (fragment, why) in [
        (
            "@builtin(front_facing) front_facing: bool",
            "the shader needs the facing flag to tint a back face at all",
        ),
        (
            "BACKFACE_INSPECTION_TINT",
            "the back-face tint constant must stay named",
        ),
        (
            "let backface_mix = select(0.0, 0.14, !front_facing)",
            "the back-face mix should stay a restrained inspection cue",
        ),
    ] {
        assert!(shader.contains(fragment), "{why}");
    }
    // A back-facing triangle gets a faint cool tint, never a dark grey: a
    // flipped surface has to stay distinguishable without looking half-shadowed.
    for forbidden in ["back_falloff", "- 0.018", "normal_faces_away"] {
        assert!(
            !shader.contains(forbidden),
            "dental light must not grow a moving back-falloff or a grazing \
             grey-wash half-shadow: found `{forbidden}`"
        );
    }
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

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
    // Matched with whitespace collapsed: the previous form pinned the exact
    // indentation of a `clamp()` argument list, so reflowing it onto one line
    // -- byte-identical semantics, golden images unchanged -- failed this test.
    // Nothing formats WGSL in CI.
    let collapsed: String = shader.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        collapsed.contains("0.48, 1.05,"),
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

/// A GPU fault must not come back as pixels.
///
/// The device's error handler records rather than panics, because a panic
/// inside the shell surrogate is a crash. Nothing on the offscreen path used
/// to ask what it recorded, so a refused buffer allocation -- a scan of three
/// million triangles against the 256 MiB floor the limits used to request --
/// produced a frame of zeroes that travelled on as a valid transparent
/// thumbnail, and Explorer cached it against the file's timestamp.
#[test]
#[allow(clippy::expect_used)]
fn a_recorded_gpu_fault_fails_the_readback_instead_of_returning_a_blank_frame() {
    use crate::{GpuCamera, Offscreen, RenderDeadline, ThumbnailSpec};
    use glam::{Mat4, Vec3};
    use occluview_core::{MeshBuilder, Vertex};
    use std::time::Duration;

    // Like the crate's other GPU suites, this one needs an adapter -- the
    // software fallback counts, and CI provides one.
    let offscreen = pollster::block_on(Offscreen::new()).expect("an offscreen adapter");

    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(Vertex::at(Vec3::new(-0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let b = builder.push_vertex(Vertex::at(Vec3::new(0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let c = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.5, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(a, b, c);
    let mesh = builder.build().expect("a triangle is a mesh");
    let camera = GpuCamera::new(
        Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y),
        Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, 3.0),
    );
    let spec = ThumbnailSpec {
        size_px: 32,
        ..ThumbnailSpec::default()
    };

    let clean = pollster::block_on(offscreen.render_with_deadline(
        &mesh,
        &camera,
        spec,
        RenderDeadline::after(Duration::from_secs(2)),
    ));
    assert!(clean.is_ok(), "a triangle renders: {clean:?}");

    super::record_gpu_error(
        &offscreen.renderer().gpu_error,
        "buffer allocation refused".to_string(),
    );
    // Map the pixels away before asserting: a failure here must print the
    // reason, not thirty-two rows of RGBA.
    let faulted = pollster::block_on(offscreen.render_with_deadline(
        &mesh,
        &camera,
        spec,
        RenderDeadline::after(Duration::from_secs(2)),
    ))
    .map(|pixels| format!("{} pixels", pixels.len()));
    let error = faulted.expect_err("a recorded fault must not return pixels");
    assert!(
        format!("{error}").contains("buffer allocation refused"),
        "the fault the driver reported must be the one the caller sees: {error}"
    );
}

/// The buffer ceiling has to come from the adapter, not from the floor.
///
/// `using_resolution` copies the three texture dimensions and nothing else, so
/// a request built from `downlevel_defaults` keeps that profile's 256 MiB
/// `max_buffer_size` however capable the adapter is. Three million triangles
/// need 309 MiB of vertex buffer -- an ordinary full-arch export with texture
/// coordinates -- and the allocation was refused.
#[test]
fn the_device_request_takes_its_buffer_ceiling_from_the_adapter() {
    let source = include_str!("pipeline_init.rs");
    assert!(
        source.contains("max_buffer_size: adapter.limits().max_buffer_size,"),
        "the headless device must ask for the adapter's buffer ceiling"
    );
}

/// Draw Sculpt's display-only volume into a pass shaped exactly like the live
/// one: this renderer's depth format and sample count.
#[allow(clippy::expect_used)]
fn draw_sculpt_into_a_live_shaped_pass(renderer: &crate::Renderer) {
    use crate::{GpuCamera, SculptToolShape, SculptToolUniform};
    use glam::Mat4;

    let device = renderer.device();
    let size = wgpu::Extent3d {
        width: 32,
        height: 24,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sculpt live compatibility color"),
        size,
        mip_level_count: 1,
        sample_count: renderer.sample_count(),
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sculpt live compatibility depth"),
        size,
        mip_level_count: 1,
        sample_count: renderer.sample_count(),
        dimension: wgpu::TextureDimension::D2,
        format: renderer.depth_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

    renderer.set_camera(&GpuCamera::new(
        Mat4::IDENTITY,
        Mat4::IDENTITY,
        glam::Vec3::Z,
        glam::Vec3::ZERO,
    ));
    renderer.set_sculpt_tool(&SculptToolUniform {
        model: Mat4::IDENTITY.to_cols_array(),
        color: [0.2, 0.8, 1.0, 1.0],
        opacity: 0.5,
        shape: SculptToolShape::Cone as u32,
        visible: 1,
        padding: 0,
    });
    let camera_bg = renderer.camera_bind_group();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sculpt live compatibility encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sculpt live compatibility pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.draw_sculpt_tool(&mut pass, &camera_bg, renderer.disabled_clip_bind_group());
    }
    renderer.queue().submit(std::iter::once(encoder.finish()));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    assert_eq!(
        renderer.take_gpu_error(),
        None,
        "Sculpt's live volume draw must not submit an incompatible depth pipeline"
    );
}

/// The live egui viewport always has a depth/stencil attachment, and eframe
/// derives it from the app's `depth_buffer: 24` / `stencil_buffer: 8`.
/// Declaring the format here, instead of reading it back from the renderer
/// under test, is what makes this a contract: a pass format and a pipeline
/// format that drifted together used to keep this suite green.
#[test]
#[allow(clippy::expect_used)]
fn sculpt_tool_pipeline_is_compatible_with_the_live_depth_pass() {
    let renderer = pollster::block_on(crate::Renderer::new_headless(
        wgpu::TextureFormat::Rgba8Unorm,
    ))
    .expect("a headless renderer");

    assert_eq!(
        renderer.depth_format(),
        wgpu::TextureFormat::Depth24PlusStencil8,
        "the live pass is Depth24PlusStencil8, so every pipeline in it must declare the same"
    );
    draw_sculpt_into_a_live_shaped_pass(&renderer);
}

/// And at the multisampled profile the app selects whenever the adapter can
/// create the multisampled live targets, which is the configuration the window
/// actually runs there.
#[test]
#[allow(clippy::expect_used)]
fn sculpt_tool_pipeline_is_compatible_with_a_multisampled_live_pass() {
    let single = pollster::block_on(crate::Renderer::new_headless(
        wgpu::TextureFormat::Rgba8Unorm,
    ))
    .expect("a headless renderer");
    let device = std::sync::Arc::clone(&single.device);
    let queue = std::sync::Arc::clone(&single.queue);

    // Probe before building the pipeline. The application turns multisampling
    // on only when BOTH the colour target and the depth attachment support 4x
    // (`adapter_supports_prefill_msaa_4`), so an adapter missing either one runs
    // the single-sample profile and must not be reported as a pipeline defect.
    // Probe both, and say which one was missing rather than returning silently.
    let size = wgpu::Extent3d {
        width: 32,
        height: 24,
        depth_or_array_layers: 1,
    };
    let probe = |label: &'static str, format: wgpu::TextureFormat| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        texture
    };
    let color_probe = probe(
        "live multisample color probe",
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let depth_probe = probe("live multisample depth probe", crate::live_depth_format());
    drop((color_probe, depth_probe));
    if single.take_gpu_error().is_some() {
        // This adapter never selects the multisampled profile in the
        // application, so the single-sample pass is the configuration that has
        // to be proven here - which the test above already does.
        return;
    }

    let multisampled = crate::Renderer::with_shared_device_sample_count(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        4,
    )
    .expect("a multisampled renderer");
    assert_eq!(multisampled.sample_count(), 4);
    draw_sculpt_into_a_live_shaped_pass(&multisampled);
}

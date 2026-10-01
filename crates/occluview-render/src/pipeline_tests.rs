#![allow(clippy::print_stderr)]

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
fn gpu_fault_stays_fail_closed_after_its_message_is_drained() {
    let latch: super::GpuErrorLatch = std::sync::Arc::new(std::sync::Mutex::new(None));
    let faulted = std::sync::atomic::AtomicBool::new(false);

    super::record_gpu_fault(&latch, &faulted, "device lost".to_string());
    assert!(
        faulted.load(std::sync::atomic::Ordering::Acquire),
        "draining the message must not make a failed device look healthy"
    );
    assert_eq!(
        super::drain_gpu_error(&latch).as_deref(),
        Some("device lost")
    );
    assert!(
        faulted.load(std::sync::atomic::Ordering::Acquire),
        "the paint callback needs a persistent stop signal after UI polling"
    );
}

/// Two live panes use one device submission, so all `Queue::write_buffer`
/// operations happen before either pane's command buffer is executed. This
/// catches the camera/clip uniform alias that a sequential render-and-wait
/// test would miss.
#[test]
#[allow(clippy::expect_used)]
#[allow(clippy::too_many_lines)]
fn renderer_peers_keep_camera_and_clip_state_independent_in_one_submission() {
    use crate::{
        ClipPlane, GpuCamera, GpuMeshUniform, GpuTexture, PreparedScene, PreparedSceneSource,
        Renderer,
    };
    use glam::Vec3;
    use occluview_core::{MeshBuilder, Vertex};
    use std::sync::Arc;

    let left = pollster::block_on(Renderer::new_headless(wgpu::TextureFormat::Rgba8Unorm))
        .expect("a headless renderer");
    let right = left.new_peer().expect("a peer renderer");
    assert!(
        std::ptr::eq(left.device(), right.device()),
        "peers must share the same wgpu device"
    );
    assert!(
        std::ptr::eq(left.queue(), right.queue()),
        "peers must use one ordered queue"
    );
    assert!(
        Arc::ptr_eq(&left.gpu_error, &right.gpu_error)
            && Arc::ptr_eq(&left.gpu_faulted, &right.gpu_faulted),
        "one physical device must have one error and fault owner"
    );

    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(Vertex::at(Vec3::new(-0.7, -0.6, 0.0)).with_normal(Vec3::Z));
    let b = builder.push_vertex(Vertex::at(Vec3::new(0.7, -0.6, 0.0)).with_normal(Vec3::Z));
    let c = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.7, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(a, b, c);
    let mesh = builder.build().expect("the isolation triangle is valid");
    let prepared = PreparedScene::prepare(
        &left,
        &[PreparedSceneSource {
            mesh: &mesh,
            uniform: GpuMeshUniform::identity(),
            visible: true,
            wireframe: false,
            contact: None,
        }],
    );
    let fallback = GpuTexture::fallback(&left, left.device(), left.queue());

    let pane_width = 64;
    let height = 64;
    let target_width = pane_width * 2;
    left.set_point_splat_viewport(pane_width, height);
    right.set_point_splat_viewport(pane_width, height);
    let left_eye = Vec3::new(-0.25, 0.0, 3.0);
    let right_eye = Vec3::new(0.25, 0.0, 3.0);
    let make_camera = |eye: Vec3| {
        GpuCamera::new(
            glam::camera::rh::view::look_at_mat4(eye, Vec3::new(eye.x, 0.0, 0.0), Vec3::Y),
            glam::camera::rh::proj::directx::orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0),
            Vec3::Z,
            eye,
        )
    };
    // These writes are intentionally queued before the same submission. If a
    // peer reused the source camera buffer, both passes would see the last
    // camera value rather than their own.
    left.set_camera(&make_camera(left_eye));
    right.set_camera(&make_camera(right_eye));

    let left_clip_buffer = left.clip_uniform_buffer();
    let right_clip_buffer = right.clip_uniform_buffer();
    left.queue().write_buffer(
        &left_clip_buffer,
        0,
        bytemuck::bytes_of(&ClipPlane::new([1.0, 0.0, 0.0], 0.0)),
    );
    right.queue().write_buffer(
        &right_clip_buffer,
        0,
        bytemuck::bytes_of(&ClipPlane::disabled()),
    );
    let left_clip = left.clip_bind_group(&left_clip_buffer);
    let right_clip = right.clip_bind_group(&right_clip_buffer);
    let target = PeerIsolationTarget::new(&left, target_width, height);

    let device = left.device();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("two viewport isolation encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("two peer isolation viewports"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
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
        pass.set_viewport(0.0, 0.0, pane_width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, pane_width, height);
        prepared.draw_with_clip(
            &left,
            &mut pass,
            &left.camera_bind_group(),
            &fallback.bind_group,
            &left_clip,
        );
        pass.set_viewport(
            pane_width as f32,
            0.0,
            pane_width as f32,
            height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(pane_width, 0, pane_width, height);
        prepared.draw_with_clip(
            &right,
            &mut pass,
            &right.camera_bind_group(),
            &fallback.bind_group,
            &right_clip,
        );
    }
    target.copy_to_buffer(&mut encoder);
    left.queue().submit(std::iter::once(encoder.finish()));

    let pixels = target.read(device);
    let left_stats = visible_pixel_stats(&pixels, target_width, pane_width, 0);
    let right_stats = visible_pixel_stats(&pixels, target_width, pane_width, pane_width);
    assert!(left_stats.count > 0, "the clipped left pane still draws");
    assert!(
        right_stats.count > left_stats.count * 3 / 2,
        "the right pane has no clipping, left={left_stats:?}, right={right_stats:?}"
    );
    assert!(
        left_stats.mean_x > f64::from(pane_width) * 0.56,
        "the left camera and +X clip plane should leave the right half: {left_stats:?}"
    );
    assert!(
        right_stats.mean_x < f64::from(pane_width) * 0.44,
        "the right camera should independently pan the full triangle left: {right_stats:?}"
    );
    assert_eq!(left.take_gpu_error(), None, "the peer draw is valid");
}

#[derive(Debug)]
struct VisiblePixelStats {
    count: usize,
    mean_x: f64,
}

fn visible_pixel_stats(
    pixels: &[[u8; 4]],
    image_width: u32,
    pane_width: u32,
    pane_x: u32,
) -> VisiblePixelStats {
    let mut count = 0usize;
    let mut sum_x = 0u64;
    let pane_x = pane_x as usize;
    let pane_width = pane_width as usize;
    for row in pixels.chunks_exact(image_width as usize) {
        for (local_x, pixel) in row.iter().skip(pane_x).take(pane_width).enumerate() {
            if pixel[0] > 12 || pixel[1] > 12 || pixel[2] > 12 {
                count += 1;
                sum_x += local_x as u64;
            }
        }
    }
    VisiblePixelStats {
        count,
        mean_x: if count == 0 {
            0.0
        } else {
            sum_x as f64 / count as f64
        },
    }
}

struct PeerIsolationTarget {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
}

impl PeerIsolationTarget {
    #[allow(clippy::expect_used)]
    fn new(renderer: &crate::Renderer, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("peer isolation color"),
            size,
            mip_level_count: 1,
            sample_count: renderer.sample_count(),
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("peer isolation depth"),
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
        let readback = renderer.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("peer isolation readback"),
            size: u64::from(width) * u64::from(height) * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            color,
            color_view,
            depth_view,
            readback,
            width,
            height,
        }
    }

    fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    #[allow(clippy::expect_used)]
    fn read(&self, device: &wgpu::Device) -> Vec<[u8; 4]> {
        use std::sync::mpsc;

        let slice = self.readback.slice(..);
        let (map_tx, map_rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = map_tx.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("wait for peer isolation readback");
        map_rx
            .recv()
            .expect("peer isolation readback callback")
            .expect("peer isolation readback maps");
        let mapped = slice
            .get_mapped_range()
            .expect("mapped peer isolation pixels");
        let pixels = bytemuck::cast_slice(&mapped).to_vec();
        drop(mapped);
        self.readback.unmap();
        pixels
    }
}

/// The fault flag stops the frame loop from feeding a broken device, but it is
/// not a verdict that the device is gone: a driver reset, a recovered eGPU, or
/// a rebuilt offscreen device can leave it set on a working renderer. The
/// operator's retry has to be able to clear it, and a *new* fault has to be
/// able to set it again afterwards.
#[test]
fn an_acknowledged_fault_can_be_cleared_and_re_raised() {
    let latch: super::GpuErrorLatch = std::sync::Arc::new(std::sync::Mutex::new(None));
    let faulted = std::sync::atomic::AtomicBool::new(false);

    super::record_gpu_fault(&latch, &faulted, "device reset".to_string());
    assert!(super::drain_gpu_error(&latch).is_some());
    assert!(faulted.load(std::sync::atomic::Ordering::Acquire));

    // What the retry button does.
    faulted.store(false, std::sync::atomic::Ordering::Release);
    assert!(
        !faulted.load(std::sync::atomic::Ordering::Acquire),
        "acknowledging must let the next frame try to draw again"
    );

    // A renderer whose device is genuinely gone raises it again immediately.
    super::record_gpu_fault(&latch, &faulted, "device lost again".to_string());
    assert!(
        faulted.load(std::sync::atomic::Ordering::Acquire),
        "a later fault must re-arm the stop signal"
    );
    assert_eq!(
        super::drain_gpu_error(&latch).as_deref(),
        Some("device lost again"),
        "the retried fault reaches the operator as a new message"
    );
}

#[test]
// Poisoning a mutex requires a deliberate panic while a guard is held. (This
// can only happen in an unwinding build; the default release profile is
// `panic = abort` where poison never occurs — the guard still keeps the poll
// crash-proof.)
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
/// inside the shell surrogate is a crash. The offscreen path must ask what it
/// recorded: otherwise a refused buffer allocation -- such as a scan of three
/// million triangles against a 256 MiB buffer limit -- produces a frame of
/// zeroes that travels on as a valid transparent thumbnail, and Explorer
/// caches it against the file's timestamp.
#[test]
#[allow(clippy::expect_used)]
fn a_recorded_gpu_fault_fails_the_readback_instead_of_returning_a_blank_frame() {
    use crate::{GpuCamera, Offscreen, RenderDeadline, ThumbnailSpec};
    use glam::Vec3;
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
        glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y),
        glam::camera::rh::proj::directx::orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0),
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

    super::record_gpu_fault(
        &offscreen.renderer().gpu_error,
        &offscreen.renderer().gpu_faulted,
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
        action: [0.0; 2],
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

#[test]
#[allow(clippy::expect_used)]
fn sculpt_tool_actions_render_distinct_body_profiles() {
    let renderer = pollster::block_on(crate::Renderer::new_headless(
        wgpu::TextureFormat::Rgba8Unorm,
    ))
    .expect("a headless renderer");
    let add = render_sculpt_tool_profile(&renderer, [0.0, 0.0]);
    let remove = render_sculpt_tool_profile(&renderer, [1.0, 0.0]);
    let iron = render_sculpt_tool_profile(&renderer, [0.0, 1.0]);

    assert!(
        different_pixels(&add, &remove) > 16,
        "Add and Remove bodies match"
    );
    assert!(
        different_pixels(&add, &iron) > 16,
        "Add and iron bodies match"
    );
    assert!(
        different_pixels(&remove, &iron) > 16,
        "Remove and iron bodies match"
    );
}

#[allow(clippy::expect_used)]
fn render_sculpt_tool_profile(renderer: &crate::Renderer, action: [f32; 2]) -> Vec<[u8; 4]> {
    use crate::{GpuCamera, SculptToolShape, SculptToolUniform};
    use glam::{Mat4, Vec3};
    let target = SculptProfileTarget::new(renderer);

    let device = renderer.device();
    let eye = Vec3::new(3.0, 0.0, 0.4);
    let camera = GpuCamera::new(
        glam::camera::rh::view::look_at_mat4(eye, Vec3::new(0.0, 0.0, 0.4), Vec3::Y),
        glam::camera::rh::proj::directx::orthographic(-1.2, 1.2, -1.2, 1.2, 0.1, 10.0),
        Vec3::new(1.0, 1.0, 2.0),
        eye,
    );
    renderer.set_camera(&camera);
    renderer.set_sculpt_tool(&SculptToolUniform {
        model: Mat4::from_scale(Vec3::new(0.55, 0.55, 0.8)).to_cols_array(),
        color: [0.9, 0.9, 0.9, 1.0],
        opacity: 0.8,
        shape: SculptToolShape::Cone as u32,
        action,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sculpt profile readback encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sculpt profile readback pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
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
        renderer.draw_sculpt_tool(
            &mut pass,
            &renderer.camera_bind_group(),
            renderer.disabled_clip_bind_group(),
        );
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SCULPT_PROFILE_BYTES_PER_ROW),
                rows_per_image: Some(SCULPT_PROFILE_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: SCULPT_PROFILE_WIDTH,
            height: SCULPT_PROFILE_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit(std::iter::once(encoder.finish()));
    read_sculpt_profile_pixels(device, &target.readback)
}

#[test]
#[allow(clippy::expect_used)]
fn sculpt_footprint_tints_light_and_dark_surfaces_without_changing_alpha() {
    let renderer = pollster::block_on(crate::Renderer::new_headless(
        wgpu::TextureFormat::Rgba8Unorm,
    ))
    .expect("a headless renderer");
    let center =
        (SCULPT_PROFILE_HEIGHT / 2 * SCULPT_PROFILE_WIDTH + SCULPT_PROFILE_WIDTH / 2) as usize;
    for light_ground in [false, true] {
        let baseline = render_sculpt_feedback_ground(&renderer, light_ground, false);
        let marked = render_sculpt_feedback_ground(&renderer, light_ground, true);
        for (index, (pixel, original)) in marked.iter().zip(&baseline).enumerate() {
            let x = u32::try_from(index % SCULPT_PROFILE_WIDTH as usize).expect("pixel column");
            let y = u32::try_from(index / SCULPT_PROFILE_WIDTH as usize).expect("pixel row");
            let x = (f64::from(x) + 0.5) * 2.0 / f64::from(SCULPT_PROFILE_WIDTH) - 1.0;
            let y = (f64::from(y) + 0.5) * 2.0 / f64::from(SCULPT_PROFILE_HEIGHT) - 1.0;
            // The half-unit stamp and its antialiased hairline fit well
            // within this radius; every pixel farther out stays untouched.
            if x * x + y * y > 0.65 * 0.65 {
                assert_eq!(pixel, original, "pixel {index} lies outside the brush");
            }
        }
        assert!(
            marked.iter().zip(&baseline).all(|(a, b)| a[3] == b[3]),
            "the material's alpha must survive the feedback pass"
        );
        if light_ground {
            assert!(
                marked[center][..3].iter().all(|channel| *channel < 240),
                "a dark Smooth footprint must remain visible on white: {:?}",
                marked[center]
            );
        } else {
            assert!(
                marked[center][..3].iter().all(|channel| *channel > 0),
                "the same footprint must remain visible on black"
            );
        }
    }
    assert_eq!(renderer.take_gpu_error(), None);
}

#[allow(clippy::expect_used)]
fn render_sculpt_feedback_ground(
    renderer: &crate::Renderer,
    light_ground: bool,
    visible: bool,
) -> Vec<[u8; 4]> {
    let target = SculptProfileTarget::new(renderer);
    let device = renderer.device();
    let (gpu_mesh, mesh_bg) = prepare_sculpt_feedback_ground(renderer, visible);
    let camera_bg = renderer.camera_bind_group();
    let ground = if light_ground { 1.0 } else { 0.0 };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sculpt feedback ground readback"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sculpt feedback on a rendered ground"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: ground,
                        g: ground,
                        b: ground,
                        a: 0.4,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth_view,
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
        renderer.draw_sculpt_surface_feedback(
            &mut pass,
            super::SculptSurfaceFeedbackBindings {
                camera_bg: &camera_bg,
                mesh_bg: &mesh_bg,
                clip_bg: renderer.disabled_clip_bind_group(),
                mesh: &gpu_mesh,
            },
        );
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SCULPT_PROFILE_BYTES_PER_ROW),
                rows_per_image: Some(SCULPT_PROFILE_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: SCULPT_PROFILE_WIDTH,
            height: SCULPT_PROFILE_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue().submit(std::iter::once(encoder.finish()));
    read_sculpt_profile_pixels(device, &target.readback)
}

#[allow(clippy::expect_used)]
fn prepare_sculpt_feedback_ground(
    renderer: &crate::Renderer,
    visible: bool,
) -> (crate::GpuMesh, wgpu::BindGroup) {
    use crate::{GpuCamera, GpuMesh, SculptBrushUniform};
    use glam::{Mat4, Vec3};
    use occluview_core::{Mesh, Vertex};

    let device = renderer.device();
    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.5)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.5)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.5)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.5)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("a plane carrying a surface footprint");
    let gpu_mesh = GpuMesh::upload(device, renderer.queue(), &mesh);
    let uniform_buffer = renderer.mesh_uniform_buffer();
    renderer.queue().write_buffer(
        &uniform_buffer,
        0,
        bytemuck::bytes_of(&crate::mesh_uniform::GpuMeshUniform::identity()),
    );
    let mesh_bg = renderer.mesh_bind_group(&uniform_buffer);
    renderer.set_camera(&GpuCamera::new(
        Mat4::IDENTITY,
        Mat4::IDENTITY,
        Vec3::Z,
        Vec3::Z * 2.0,
    ));
    let mut brush = SculptBrushUniform::hidden();
    brush.visible = u32::from(visible);
    brush.center = [0.0, 0.0, 0.5];
    brush.radius = 0.5;
    brush.color = [0.1, 0.1, 0.1, 1.0];
    brush.intensity = 0.035;
    renderer.set_sculpt_brush(&brush);
    (gpu_mesh, mesh_bg)
}

const SCULPT_PROFILE_WIDTH: u32 = 64;
const SCULPT_PROFILE_HEIGHT: u32 = 64;
const SCULPT_PROFILE_BYTES_PER_ROW: u32 = SCULPT_PROFILE_WIDTH * 4;

struct SculptProfileTarget {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl SculptProfileTarget {
    fn new(renderer: &crate::Renderer) -> Self {
        let device = renderer.device();
        let size = wgpu::Extent3d {
            width: SCULPT_PROFILE_WIDTH,
            height: SCULPT_PROFILE_HEIGHT,
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sculpt profile readback target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sculpt profile readback depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.depth_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sculpt profile pixels"),
            size: u64::from(SCULPT_PROFILE_BYTES_PER_ROW) * u64::from(SCULPT_PROFILE_HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            color,
            color_view,
            depth_view,
            readback,
        }
    }
}

#[allow(clippy::expect_used)]
fn read_sculpt_profile_pixels(device: &wgpu::Device, readback: &wgpu::Buffer) -> Vec<[u8; 4]> {
    use std::sync::mpsc;

    let slice = readback.slice(..);
    let (map_tx, map_rx) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = map_tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for sculpt profile pixels");
    map_rx
        .recv()
        .expect("sculpt profile readback callback")
        .expect("sculpt profile readback maps");
    let mapped = slice
        .get_mapped_range()
        .expect("mapped sculpt profile pixels");
    let pixels = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    pixels
}

fn different_pixels(left: &[[u8; 4]], right: &[[u8; 4]]) -> usize {
    left.iter()
        .zip(right)
        .filter(|(a, b)| a[..3] != b[..3])
        .count()
}

/// The live egui viewport always has a depth/stencil attachment, and eframe
/// derives it from the app's `depth_buffer: 24` / `stencil_buffer: 8`.
/// Declaring the format here, instead of reading it back from the renderer
/// under test, is what makes this a contract: a pass format and a pipeline
/// format that drift together would otherwise keep this suite green.
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
    // on only when both the colour target and the depth attachment support 4x
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
    if let Some(error) = single.take_gpu_error() {
        // This adapter never selects the multisampled profile in the
        // application, so the single-sample pass is the configuration that has
        // to be proven here, which the test above already does. Adapter lanes
        // that require GPU coverage fail instead of treating this as a pass.
        assert!(
            std::env::var_os("OCCLUVIEW_REQUIRE_GPU_TESTS").is_none_or(|value| value == "0"),
            "OCCLUVIEW_REQUIRE_GPU_TESTS is set, but 4x live targets are unavailable: {error}"
        );
        eprintln!("skipped: the selected adapter cannot create the 4x live targets: {error}");
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

//! Prepared-scene render-pass regression tests.

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{
    GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, PreparedScene, PreparedSceneSource,
    PreparedSceneTopology, PreparedSceneUpdate, RenderDeadline, Renderer, ViewportSpec,
};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

const SHARED_WIDTH: u32 = 32;
const SHARED_HEIGHT: u32 = 24;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const SHARED_PADDED_BYTES_PER_ROW: u32 = 256;

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("prepared-scene GPU test lock is not poisoned")
}

fn test_render_deadline() -> RenderDeadline {
    RenderDeadline::after(Duration::from_secs(5))
}

fn triangle_mesh() -> Mesh {
    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(Vertex::at(Vec3::new(-0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let b = builder.push_vertex(Vertex::at(Vec3::new(0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let c = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.5, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(a, b, c);
    builder.build().expect("valid triangle mesh")
}

fn opposite_normal_triangles() -> Mesh {
    let mut builder = MeshBuilder::new();
    let front_left =
        builder.push_vertex(Vertex::at(Vec3::new(-0.75, -0.45, 0.0)).with_normal(Vec3::Z));
    let front_right =
        builder.push_vertex(Vertex::at(Vec3::new(-0.15, -0.45, 0.0)).with_normal(Vec3::Z));
    let front_top =
        builder.push_vertex(Vertex::at(Vec3::new(-0.45, 0.45, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(front_left, front_right, front_top);

    let back_left =
        builder.push_vertex(Vertex::at(Vec3::new(0.15, -0.45, 0.0)).with_normal(-Vec3::Z));
    let back_right =
        builder.push_vertex(Vertex::at(Vec3::new(0.75, -0.45, 0.0)).with_normal(-Vec3::Z));
    let back_top =
        builder.push_vertex(Vertex::at(Vec3::new(0.45, 0.45, 0.0)).with_normal(-Vec3::Z));
    builder.push_triangle(back_left, back_right, back_top);
    builder.build().expect("valid opposite-normal mesh")
}

fn reversed_winding_triangle() -> Mesh {
    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(Vertex::at(Vec3::new(-0.5, -0.45, 0.0)).with_normal(Vec3::Z));
    let b = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.45, 0.0)).with_normal(Vec3::Z));
    let c = builder.push_vertex(Vertex::at(Vec3::new(0.5, -0.45, 0.0)).with_normal(Vec3::Z));
    builder.push_triangle(a, b, c);
    builder.build().expect("valid reversed-winding mesh")
}

fn point_cloud_mesh() -> Mesh {
    let mut builder = MeshBuilder::new();
    for (x, y) in [
        (-0.5, -0.5),
        (0.5, -0.5),
        (0.0, 0.5),
        (-0.3, 0.0),
        (0.3, 0.0),
    ] {
        builder.push_vertex(Vertex::at(Vec3::new(x, y, 0.0)).with_normal(Vec3::Z));
    }
    builder.as_point_cloud().build().expect("valid point cloud")
}

fn camera_looking_at_origin() -> GpuCamera {
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 4.0 / 3.0, 0.1, 100.0);
    GpuCamera::new(
        view,
        proj,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, 2.0),
    )
}

fn identity_uniform() -> GpuMeshUniform {
    GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
        tint: [0.9, 0.95, 1.0, 1.0],
        opacity: 1.0,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
        show_texture: 1,
        measured_map: 0,
        padding: [0; 2],
    }
}

fn pixel_luma(pixel: &[u8]) -> i32 {
    i32::from(pixel[0]) + i32::from(pixel[1]) + i32::from(pixel[2])
}

fn pixel_at(pixels: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    let start = (y * width + x) * 4;
    &pixels[start..start + 4]
}

fn pixel_delta_sum(left: &[u8], right: &[u8]) -> u64 {
    left.iter()
        .zip(right)
        .map(|(lhs, rhs)| u64::from(lhs.abs_diff(*rhs)))
        .sum()
}

#[test]
fn prepared_scene_rejects_same_length_different_mesh_topology() {
    let _gpu = gpu_test_lock();
    let original = triangle_mesh();
    let replacement = triangle_mesh();
    assert_eq!(original.vertices().len(), replacement.vertices().len());
    assert_eq!(original.indices().len(), replacement.indices().len());

    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let mut prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &original,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);

    let updated = prepared.update(
        offscreen.renderer(),
        &[PreparedSceneUpdate {
            topology: PreparedSceneTopology::from_mesh(&replacement),
            uniform: identity_uniform(),
            visible: true,
            wireframe: false,
        }],
    );

    assert!(
        !updated,
        "same layer count is not enough: changed mesh topology must rebuild GPU buffers"
    );
}

#[test]
fn prepared_scene_draws_into_existing_render_pass() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let renderer = offscreen.renderer();
    let device = renderer.device();
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("prepared scene live test color"),
        size: wgpu::Extent3d {
            width: 32,
            height: 24,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("prepared scene live test depth"),
        size: wgpu::Extent3d {
            width: 32,
            height: 24,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.depth_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let fallback = GpuTexture::fallback(renderer, device, renderer.queue());

    renderer.set_camera(&cam);
    let camera_bg = renderer.camera_bind_group();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("prepared scene live test encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("prepared scene live test pass"),
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
        prepared.draw(renderer, &mut pass, &camera_bg, &fallback.bind_group);
    }
    renderer.queue().submit(std::iter::once(encoder.finish()));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

fn renderer_from_shared_handles(owner: &Renderer) -> Renderer {
    let device = Arc::new(owner.device().clone());
    let queue = Arc::new(owner.queue().clone());
    Renderer::with_shared_device_sample_count(
        Arc::clone(&device),
        Arc::clone(&queue),
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )
    .expect("renderer from shared device and queue")
}

fn expected_backend(name: &str) -> Option<wgpu::Backend> {
    match name.trim().to_ascii_lowercase().as_str() {
        "noop" => Some(wgpu::Backend::Noop),
        "vulkan" => Some(wgpu::Backend::Vulkan),
        "metal" => Some(wgpu::Backend::Metal),
        "dx12" => Some(wgpu::Backend::Dx12),
        "gl" => Some(wgpu::Backend::Gl),
        "webgpu" => Some(wgpu::Backend::BrowserWebGpu),
        _ => None,
    }
}

fn assert_adapter_matches_test_environment(device: &wgpu::Device) {
    let info = device.adapter_info();
    if let Ok(expected) = std::env::var("WGPU_BACKEND") {
        if !expected.trim().is_empty() {
            let expected_backend = expected_backend(&expected);
            assert!(
                expected_backend.is_some(),
                "unsupported WGPU_BACKEND test expectation {expected:?}; supported names: noop, vulkan, metal, dx12, gl, webgpu"
            );
            let Some(expected_backend) = expected_backend else {
                return;
            };
            assert_eq!(
                info.backend, expected_backend,
                "WGPU_BACKEND={expected:?} is a test expectation, but the production renderer selected backend {:?} on adapter {:?}",
                info.backend, info.name
            );
        }
    }
    if let Ok(expected) = std::env::var("WGPU_ADAPTER_NAME") {
        if !expected.trim().is_empty() {
            assert!(
                info.name
                    .to_ascii_lowercase()
                    .contains(&expected.trim().to_ascii_lowercase()),
                "WGPU_ADAPTER_NAME={expected:?} is a test expectation, but the production renderer selected adapter {:?} on backend {:?}",
                info.name,
                info.backend
            );
        }
    }
}

fn shared_color_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shared-device prepared scene color"),
        size: wgpu::Extent3d {
            width: SHARED_WIDTH,
            height: SHARED_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    (color, color_view)
}

fn shared_depth_target(renderer: &Renderer) -> (wgpu::Texture, wgpu::TextureView) {
    let depth = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("shared-device prepared scene depth"),
        size: wgpu::Extent3d {
            width: SHARED_WIDTH,
            height: SHARED_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.depth_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    (depth, depth_view)
}

fn shared_readback_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shared-device prepared scene readback"),
        size: u64::from(SHARED_PADDED_BYTES_PER_ROW) * u64::from(SHARED_HEIGHT),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn submit_shared_prepared_scene(
    renderer: &Renderer,
    prepared: &PreparedScene,
    camera: &GpuCamera,
) -> wgpu::Buffer {
    let device = renderer.device();
    let queue = renderer.queue();
    let (color, color_view) = shared_color_target(device);
    let (_depth, depth_view) = shared_depth_target(renderer);
    let readback = shared_readback_buffer(device);
    let fallback = GpuTexture::fallback(renderer, device, queue);

    renderer.set_camera(camera);
    let camera_bg = renderer.camera_bind_group();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("shared-device prepared scene encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shared-device prepared scene pass"),
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
        prepared.draw(renderer, &mut pass, &camera_bg, &fallback.bind_group);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SHARED_PADDED_BYTES_PER_ROW),
                rows_per_image: Some(SHARED_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: SHARED_WIDTH,
            height: SHARED_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    readback
}

fn count_lit_shared_pixels(renderer: &Renderer, readback: &wgpu::Buffer) -> usize {
    let slice = readback.slice(..);
    let (map_tx, map_rx) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = map_tx.send(result);
    });
    renderer
        .device()
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for shared-device readback");
    map_rx
        .recv()
        .expect("shared-device map callback")
        .expect("shared-device map succeeds");
    let mapped = slice
        .get_mapped_range()
        .expect("shared-device mapped range");
    let row_bytes = SHARED_WIDTH as usize * RGBA_BYTES_PER_PIXEL;
    let lit_pixels = (0..SHARED_HEIGHT as usize)
        .flat_map(|row| {
            let start = row * SHARED_PADDED_BYTES_PER_ROW as usize;
            mapped[start..start + row_bytes]
                .as_chunks::<RGBA_BYTES_PER_PIXEL>()
                .0
                .iter()
        })
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count();
    drop(mapped);
    readback.unmap();
    lit_pixels
}

#[test]
fn shared_device_renderer_submits_and_reads_back_a_prepared_scene() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let owner = pollster::block_on(Renderer::new_headless(wgpu::TextureFormat::Rgba8Unorm))
        .expect("headless renderer init");
    assert_adapter_matches_test_environment(owner.device());
    let renderer = renderer_from_shared_handles(&owner);
    let prepared = PreparedScene::prepare(
        &renderer,
        &[PreparedSceneSource {
            mesh: &mesh,
            uniform: identity_uniform(),
            visible: true,
            wireframe: false,
        }],
    );
    let readback = submit_shared_prepared_scene(&renderer, &prepared, &camera_looking_at_origin());
    let lit_pixels = count_lit_shared_pixels(&renderer, &readback);

    assert!(
        lit_pixels > 16,
        "shared-device prepared scene should produce readable pixels, got {lit_pixels}"
    );
    assert!(
        renderer.take_gpu_error().is_none(),
        "shared device and queue must not trigger a wgpu validation mismatch"
    );
}

#[test]
fn prepared_viewport_can_draw_selection_overlay_after_base_scene() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let overlay = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let base = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let overlay_scene = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &overlay,
        uniform: GpuMeshUniform {
            tint: [1.0, 0.58, 0.06, 1.0],
            opacity: 0.45,
            ..identity_uniform()
        },
        visible: true,
        wireframe: true,
    }]);
    let spec = ViewportSpec {
        size_px: [96, 64],
        background: [0.78, 0.80, 0.82, 1.0],
    };

    let base_pixels = pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        &base,
        &cam,
        spec,
        test_render_deadline(),
    ))
    .expect("render base scene");
    let overlay_pixels = pollster::block_on(
        offscreen.render_prepared_viewport_with_overlay_with_deadline(
            &base,
            Some(&overlay_scene),
            &cam,
            spec,
            test_render_deadline(),
        ),
    )
    .expect("render scene with overlay");

    assert!(
        pixel_delta_sum(&base_pixels, &overlay_pixels) > 1_000,
        "selection overlay should visibly affect the rendered viewport"
    );
}

#[test]
fn studio_material_lights_opposite_normals_evenly() {
    let _gpu = gpu_test_lock();
    let mesh = opposite_normal_triangles();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let pixels = pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.78, 0.80, 0.82, 1.0],
        },
        test_render_deadline(),
    ))
    .expect("render opposite normals");

    let front = pixel_at(&pixels, 96, 29, 36);
    let back = pixel_at(&pixels, 96, 66, 36);
    // Back-facing normals use the same bright inspection light as front-facing
    // normals; both surfaces must remain equally visible.
    assert!(
        pixel_luma(front) > 520 && pixel_luma(back) > 520,
        "opposite-normal triangles must both stay brightly, evenly lit: front={front:?} back={back:?}"
    );
    assert!(
        (pixel_luma(front) - pixel_luma(back)).abs() < 24,
        "opposite normals must light evenly with no half-shadow tint: front={front:?} back={back:?}"
    );
}

#[test]
fn studio_material_draws_reversed_winding_meshes() {
    let _gpu = gpu_test_lock();
    let mesh = reversed_winding_triangle();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let pixels = pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.78, 0.80, 0.82, 1.0],
        },
        test_render_deadline(),
    ))
    .expect("render reversed winding");

    let center = pixel_at(&pixels, 96, 48, 36);
    assert!(
        pixel_luma(center) > 450 && center[2] > center[0],
        "reversed-winding mesh should remain visible with a cool inspection tint, center={center:?}"
    );
}

#[test]
fn prepared_scene_point_cloud_uses_readable_splats() {
    let _gpu = gpu_test_lock();
    let mesh = point_cloud_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let pixels = pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.039, 0.039, 0.039, 1.0],
        },
        test_render_deadline(),
    ))
    .expect("render prepared point cloud");

    let non_bg = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(
        non_bg > 80,
        "prepared point cloud stayed sparse ({non_bg} non-bg pixels); expected readable splats"
    );
    assert!(
        non_bg < 500,
        "prepared point cloud splats grew too large ({non_bg} non-bg pixels)"
    );
}

//! Prepared-scene render-pass regression tests.

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{
    GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, PreparedSceneSource, PreparedSceneTopology,
    PreparedSceneUpdate, ViewportSpec,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("prepared-scene GPU test lock is not poisoned")
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
        padding: [0; 3],
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
        });
        prepared.draw(renderer, &mut pass, &camera_bg, &fallback.bind_group);
    }
    renderer.queue().submit(std::iter::once(encoder.finish()));
    let _ = device.poll(wgpu::Maintain::Wait);
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

    let base_pixels = pollster::block_on(offscreen.render_prepared_viewport(&base, &cam, spec))
        .expect("render base scene");
    let overlay_pixels = pollster::block_on(offscreen.render_prepared_viewport_with_overlay(
        &base,
        Some(&overlay_scene),
        &cam,
        spec,
    ))
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
    let pixels = pollster::block_on(offscreen.render_prepared_viewport(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.78, 0.80, 0.82, 1.0],
        },
    ))
    .expect("render opposite normals");

    let front = pixel_at(&pixels, 96, 29, 36);
    let back = pixel_at(&pixels, 96, 66, 36);
    // Even dental light (owner rule): a front-facing triangle whose VERTEX
    // normal points away is flipped toward the viewer and lit exactly like a
    // correctly-oriented one — no grazing grey half-shadow that would darken a
    // whole inverted-normal surface. Both must stay bright AND read the same.
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
    let pixels = pollster::block_on(offscreen.render_prepared_viewport(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.78, 0.80, 0.82, 1.0],
        },
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
    let pixels = pollster::block_on(offscreen.render_prepared_viewport(
        &prepared,
        &cam,
        ViewportSpec {
            size_px: [96, 64],
            background: [0.039, 0.039, 0.039, 1.0],
        },
    ))
    .expect("render prepared point cloud");

    let non_bg = pixels
        .chunks_exact(4)
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

//! DIAGNOSTIC (ignored): render a light-clay sphere and a box edge-on to eyeball
//! the `fs_main` grazing/edge shading. Used to verify the "even dental light, no
//! shadows" change removes the dark corners/faces without flattening curvature.
//!
//! `cargo test -p occluview-render --test shading_diag -- --ignored --nocapture`

#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::print_stderr
)]

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{
    ClipPlane, GpuCamera, GpuMeshUniform, Offscreen, PreparedSceneSource, ThumbnailSpec,
};

const BG: [f64; 4] = [0.886, 0.902, 0.918, 1.0]; // matches the app viewport grey
const SIZE: u16 = 360;
const CLAY: [u8; 4] = [214, 205, 192, 255];

fn identity_uniform() -> GpuMeshUniform {
    GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        tint: [1.0, 1.0, 1.0, 1.0],
        opacity: 1.0,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
        show_texture: 1,
        padding: [0; 3],
    }
}

fn sphere(radius: f32) -> Mesh {
    let (stacks, slices) = (48usize, 64usize);
    let mut b = MeshBuilder::new();
    let mut grid = vec![vec![0u32; slices + 1]; stacks + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let phi = (i as f32 / stacks as f32) * std::f32::consts::PI;
        for (j, cell) in row.iter_mut().enumerate() {
            let theta = (j as f32 / slices as f32) * std::f32::consts::TAU;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            *cell = b.push_vertex(Vertex::at(n * radius).with_normal(n).with_color(CLAY));
        }
    }
    for i in 0..stacks {
        for j in 0..slices {
            let (a, c, d, e) = (
                grid[i][j],
                grid[i + 1][j],
                grid[i][j + 1],
                grid[i + 1][j + 1],
            );
            b.push_triangle(a, c, d);
            b.push_triangle(d, c, e);
        }
    }
    b.build().expect("sphere")
}

fn box_mesh(half: f32) -> Mesh {
    let mut b = MeshBuilder::new();
    // Six faces, each with an outward face normal (flat-shaded cube => hard
    // edges are exactly where a grazing/edge darkening would show).
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::NEG_Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::Z, Vec3::X),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (normal, up, right) in faces {
        let center = normal * half;
        let corners = [
            center + (right + up) * half,
            center + (right - up) * half,
            center + (-right - up) * half,
            center + (-right + up) * half,
        ];
        let base = corners
            .iter()
            .map(|&p| b.push_vertex(Vertex::at(p).with_normal(normal).with_color(CLAY)))
            .collect::<Vec<_>>();
        b.push_triangle(base[0], base[1], base[2]);
        b.push_triangle(base[0], base[2], base[3]);
    }
    b.build().expect("box")
}

fn ortho_camera(eye: Vec3, half_extent: f32) -> GpuCamera {
    let up = if eye.normalize_or_zero().dot(Vec3::Y).abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, up);
    let proj = Mat4::orthographic_rh(
        -half_extent,
        half_extent,
        -half_extent,
        half_extent,
        0.1,
        eye.length() * 2.0 + half_extent,
    );
    // Fixed studio key light (camera-relative fill is derived in the shader).
    GpuCamera::new(view, proj, Vec3::new(0.35, 0.72, 0.60), eye)
}

fn save_png(path: &str, pixels: &[u8]) {
    let img = image::RgbaImage::from_raw(u32::from(SIZE), u32::from(SIZE), pixels.to_vec())
        .expect("rgba");
    img.save(path).expect("write png");
}

#[test]
#[ignore = "writes PNGs to the scratchpad for manual inspection"]
fn shading_edge_dump() {
    let tag = std::env::var("SHADE_TAG").unwrap_or_else(|_| "after".to_string());
    let out_dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/cutview-r3"
    );
    std::fs::create_dir_all(out_dir).expect("dump dir");
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen");

    let sphere_mesh = sphere(1.0);
    let box_mesh = box_mesh(0.85);
    let spec = ThumbnailSpec {
        size_px: SIZE,
        background: BG,
    };

    // Sphere from straight-on: the silhouette rim is the grazing test.
    let sphere_scene = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &sphere_mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let cam = ortho_camera(Vec3::new(0.0, 0.0, 6.0), 1.15);
    let px = pollster::block_on(offscreen.render_prepared_scene_with_clip(
        &sphere_scene,
        &cam,
        &ClipPlane::disabled(),
        spec,
    ))
    .expect("sphere render");
    save_png(&format!("{out_dir}/shade_{tag}_sphere.png"), &px);

    // Box viewed toward a corner (edge-on): three faces + a near vertical edge
    // and a corner — exactly where an edge half-shadow would darken.
    let box_scene = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &box_mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }]);
    let cam = ortho_camera(Vec3::new(3.2, 2.4, 3.2), 1.35);
    let px = pollster::block_on(offscreen.render_prepared_scene_with_clip(
        &box_scene,
        &cam,
        &ClipPlane::disabled(),
        spec,
    ))
    .expect("box render");
    save_png(&format!("{out_dir}/shade_{tag}_box.png"), &px);

    eprintln!("shading_diag PNGs ({tag}) written to {out_dir}");
}

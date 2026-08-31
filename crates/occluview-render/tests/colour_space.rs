//! What colour a value ends up as on screen.
//!
//! The render target is `Rgba8Unorm` and nothing encodes on the way out of the
//! shader, so whatever the shader returns is treated as sRGB. Every source of
//! colour -- a texture, a vertex, a layer tint -- has to arrive in that same
//! space. The two tests here say so: one nominal value must reach the screen
//! the same way through a texture and through a vertex, and a layer tint must
//! reach it as the number it holds.
//!
//! Each test isolates one color path and its expected channel values.

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, MeshTexture, Vertex};
use occluview_render::{GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, ThumbnailSpec};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SIZE: u16 = 64;
const DARK_TEST_BACKGROUND: [f64; 4] = [0.039, 0.039, 0.039, 1.0];

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("colour-space GPU test lock is not poisoned")
}

fn camera_looking_at_origin() -> GpuCamera {
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 100.0);
    GpuCamera::new(
        view,
        proj,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, 2.0),
    )
}

fn dark_thumbnail_spec() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: SIZE,
        background: DARK_TEST_BACKGROUND,
    }
}

/// A textured-triangle golden test: validates the full texture pipeline
/// (Vertex.uv -> WGSL sampler -> tint -> lighting) end-to-end on the software rasterizer. Uses
/// a synthetic 2x2 checkerboard texture so the output is deterministic.
fn textured_triangle_mesh() -> Mesh {
    // UV-mapped triangle covering UV space [0,0]-[1,1].
    let mut b = MeshBuilder::new();
    let a = b.push_vertex(
        Vertex::at(Vec3::new(-0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([0.0, 0.0]),
    );
    let c = b.push_vertex(
        Vertex::at(Vec3::new(0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([1.0, 0.0]),
    );
    let d = b.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_uv([0.5, 1.0]),
    );
    b.push_triangle(a, c, d);
    b.build().expect("valid textured mesh")
}

/// A uniform 1x1 texture of a known RGBA color, for channel-order assertions.
fn uniform_texture(rgba: [u8; 4]) -> MeshTexture {
    MeshTexture::new(1, 1, rgba.to_vec())
}

/// Render the textured triangle with `texture` and return the RGBA8 pixels.
fn render_uniform_textured(texture: &MeshTexture) -> Vec<u8> {
    let _gpu = gpu_test_lock();
    let mesh = textured_triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let device = offscreen.renderer().device();
    let queue = offscreen.renderer().queue();
    let gpu_tex = GpuTexture::upload(offscreen.renderer(), device, queue, texture);
    let uniform = GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
        tint: [1.0, 1.0, 1.0, 1.0],
        opacity: 1.0,
        has_texture: 1,
        show_orientation: 0,
        show_vertex_colors: 1,
        show_texture: 1,
        measured_map: 0,
        padding: [0; 2],
    };
    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: Some(&gpu_tex),
    }];
    pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
        .expect("render scene")
}

/// The render path preserves channel order for warm-white and blue textures.
/// Upload uses `Rgba8UnormSrgb`, sampling uses `tex.rgb`, and readback only
/// flips vertically.
#[test]
fn textured_render_preserves_channel_order() {
    // Warm white, the canonical dental enamel color (R >= G > B).
    let warm = render_uniform_textured(&uniform_texture([250, 240, 225, 255]));
    let mut warm_lit = 0usize;
    let mut warm_ok = 0usize;
    for px in warm.as_chunks::<4>().0 {
        if px[0] < 12 && px[1] < 12 && px[2] < 12 {
            continue; // background
        }
        warm_lit += 1;
        if px[0] > px[2] {
            warm_ok += 1;
        }
    }
    assert!(warm_lit > 50, "warm-white triangle rendered almost nothing");
    assert_eq!(
        warm_ok, warm_lit,
        "warm-white texture rendered with B>=R on {} of {warm_lit} pixels — a channel swap in the GPU path",
        warm_lit - warm_ok
    );

    // Pure blue must stay blue (B > R), never flip to red.
    let blue = render_uniform_textured(&uniform_texture([0, 0, 255, 255]));
    let mut blue_lit = 0usize;
    let mut blue_ok = 0usize;
    for px in blue.as_chunks::<4>().0 {
        if px[0] < 12 && px[1] < 12 && px[2] < 12 {
            continue;
        }
        blue_lit += 1;
        if px[2] > px[0] {
            blue_ok += 1;
        }
    }
    assert!(blue_lit > 50, "blue triangle rendered almost nothing");
    assert_eq!(
        blue_ok, blue_lit,
        "pure-blue texture rendered with R>=B on {} of {blue_lit} pixels — a channel swap in the GPU path",
        blue_lit - blue_ok
    );
}

/// The same triangle, coloured through the vertex path instead of a texture.
fn render_uniform_vertex_colored(rgba: [u8; 4]) -> Vec<u8> {
    let _gpu = gpu_test_lock();
    let mut builder = MeshBuilder::new();
    let a = builder.push_vertex(
        Vertex::at(Vec3::new(-0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(rgba),
    );
    let c = builder.push_vertex(
        Vertex::at(Vec3::new(0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(rgba),
    );
    let d = builder.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(rgba),
    );
    builder.push_triangle(a, c, d);
    let mesh = builder.build().expect("valid vertex-coloured mesh");
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let uniform = GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
        tint: [1.0, 1.0, 1.0, 1.0],
        opacity: 1.0,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
        show_texture: 0,
        measured_map: 0,
        padding: [0; 2],
    };
    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: None,
    }];
    pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
        .expect("render scene")
}

fn brightest_lit_pixel(pixels: &[u8]) -> Option<[u8; 4]> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] >= 12 || px[1] >= 12 || px[2] >= 12)
        .max_by_key(|px| u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2]))
        .map(|px| [px[0], px[1], px[2], px[3]])
}

/// The tolerance is wide enough to ignore the lighting model's own difference
/// between the two branches (about a dozen levels) and far too tight for a
/// colour-space error, which is forty to eighty.
/// The same nominal colour must reach the screen the same way whether it
/// arrives in a texture or in a vertex.
///
/// The render target is `Rgba8Unorm` and nothing encodes on the way out, so
/// whatever the shader returns is treated as sRGB. Vertex colours arrive as
/// `byte / 255`, already in that space. Typing the texture as sRGB made
/// `textureSample` decode to linear, and that value was then written out as if
/// it were sRGB: measured on this exact triangle, sRGB 128 came out as 70
/// through a texture against 129 through a vertex, and sRGB 200 as 159 against
/// 198. That is the flagship formats (HPS, GLB -- colour in a texture) and the
/// open ones (PLY, OBJ -- colour in vertices) disagreeing about the same
/// physical colour, in a viewer that is used to judge colour.
#[test]
fn a_colour_reaches_the_screen_the_same_way_through_a_texture_or_a_vertex() {
    const TOLERANCE: i32 = 20;
    for value in [200_u8, 128, 250] {
        let textured = render_uniform_textured(&uniform_texture([value, value, value, 255]));
        let vertex = render_uniform_vertex_colored([value, value, value, 255]);
        let textured = brightest_lit_pixel(&textured);
        let vertex = brightest_lit_pixel(&vertex);
        assert!(
            textured.is_some() && vertex.is_some(),
            "both paths should render something for {value}"
        );
        let (Some(textured), Some(vertex)) = (textured, vertex) else {
            return;
        };
        for channel in 0..3 {
            let difference = i32::from(textured[channel]) - i32::from(vertex[channel]);
            assert!(
                difference.abs() <= TOLERANCE,
                "nominal {value} rendered as {textured:?} through a texture and \
                 {vertex:?} through a vertex colour: the two paths are not in \
                 the same colour space"
            );
        }
    }
}

/// Render a white vertex-coloured triangle under `tint` and report the
/// brightest lit pixel.
fn render_tinted_white(tint: [f32; 4]) -> Vec<u8> {
    let _gpu = gpu_test_lock();
    let mut builder = MeshBuilder::new();
    let white = [255_u8, 255, 255, 255];
    let a = builder.push_vertex(
        Vertex::at(Vec3::new(-0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(white),
    );
    let c = builder.push_vertex(
        Vertex::at(Vec3::new(0.5, -0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(white),
    );
    let d = builder.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.5, 0.0))
            .with_normal(Vec3::Z)
            .with_color(white),
    );
    builder.push_triangle(a, c, d);
    let mesh = builder.build().expect("valid vertex-coloured mesh");
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let uniform = GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
        tint,
        opacity: 1.0,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
        show_texture: 0,
        measured_map: 0,
        padding: [0; 2],
    };
    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: None,
    }];
    pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
        .expect("render scene")
}

/// A layer tint reaches the screen as the number it is, not as a number in
/// some other colour space.
///
/// The layer list draws a swatch beside the tint's name, and that swatch is
/// the only place the colour is ever labelled. If the swatch applies a
/// transfer function the renderer does not, the two disagree -- and they did,
/// by seventy levels on the first preset in the palette.
#[test]
fn a_layer_tint_reaches_the_screen_as_the_value_it_holds() {
    const TOLERANCE: f32 = 20.0;
    for tint in [
        [0.03_f32, 0.15, 0.79, 1.0],
        [0.95_f32, 0.53, 0.0, 1.0],
        [0.5_f32, 0.5, 0.5, 1.0],
    ] {
        let rendered = brightest_lit_pixel(&render_tinted_white(tint));
        assert!(rendered.is_some(), "the tinted triangle should render");
        let Some(rendered) = rendered else {
            return;
        };
        for channel in 0..3 {
            let expected = (tint[channel] * 255.0).round();
            let difference = f32::from(rendered[channel]) - expected;
            assert!(
                difference.abs() <= TOLERANCE,
                "tint {tint:?} rendered as {rendered:?}: channel {channel} \
                 expected about {expected}"
            );
        }
    }
}

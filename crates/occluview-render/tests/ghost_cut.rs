//! Ghost cut-view regression tests.
//!
//! OWNER rule: the cut view must NOT remove geometry from the main viewport.
//! The kept side draws opaque (unchanged) and the cut-away side is re-drawn as
//! a faint translucent ghost. These tests render the main-viewport clip path
//! (`render_prepared_viewport_with_clip_and_overlay`, which now runs the ghost
//! pass) and the small-slice hard-clip path (`render_prepared_scene_with_clip`)
//! and assert:
//!   * the kept side is pixel-for-pixel the same whether or not the ghost runs,
//!   * the cut-away side is faint-but-present in the ghost render, and
//!   * the cut-away side is background (fully removed) in the hard-clip render.
//!
//! `ghost_cut_view_visual_dump` (ignored) writes PNGs at several disc poses on
//! a sphere and an arch fixture for human review:
//!   `cargo test -p occluview-render --test ghost_cut -- --ignored`

// Pixel-grid math and fixture generation use casts and short vertex names;
// `eprintln!` reports the dump path. All test-only, allowed crate-wide here.
#![allow(
    clippy::expect_used,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::too_many_arguments,
    clippy::print_stderr
)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, MeshTexture, Vertex};
use occluview_render::{
    ClipPlane, GpuCamera, GpuMeshUniform, Offscreen, PreparedScene, PreparedSceneSource,
    ThumbnailSpec, ViewportSpec,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SIZE: u16 = 128;
const DARK_BG: [f64; 4] = [0.04, 0.04, 0.04, 1.0];

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("ghost-cut GPU test lock is not poisoned")
}

fn identity_uniform() -> GpuMeshUniform {
    GpuMeshUniform {
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
    }
}

/// A UV sphere of the given radius, warm off-white so the ghost's cool
/// desaturation is visible.
fn uv_sphere(radius: f32, stacks: usize, slices: usize) -> Mesh {
    let mut b = MeshBuilder::new();
    let color = [214, 202, 188, 255];
    let mut grid = vec![vec![0u32; slices + 1]; stacks + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let phi = (i as f32 / stacks as f32) * std::f32::consts::PI;
        for (j, cell) in row.iter_mut().enumerate() {
            let theta = (j as f32 / slices as f32) * std::f32::consts::TAU;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            *cell = b.push_vertex(Vertex::at(n * radius).with_normal(n).with_color(color));
        }
    }
    for i in 0..stacks {
        for j in 0..slices {
            let a = grid[i][j];
            let c = grid[i + 1][j];
            let d = grid[i][j + 1];
            let e = grid[i + 1][j + 1];
            b.push_triangle(a, c, d);
            b.push_triangle(d, c, e);
        }
    }
    b.build().expect("valid sphere mesh")
}

/// A torus standing in for a curved dental arch: a plane cuts curved geometry
/// on both sides so the ghost half is clearly a bent solid, not a flat disc.
fn torus(major: f32, minor: f32, seg_major: usize, seg_minor: usize) -> Mesh {
    let mut b = MeshBuilder::new();
    let color = [206, 198, 214, 255];
    let mut grid = vec![vec![0u32; seg_minor + 1]; seg_major + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let u = (i as f32 / seg_major as f32) * std::f32::consts::TAU;
        let (su, cu) = u.sin_cos();
        for (j, cell) in row.iter_mut().enumerate() {
            let v = (j as f32 / seg_minor as f32) * std::f32::consts::TAU;
            let (sv, cv) = v.sin_cos();
            let n = Vec3::new(cu * cv, sv, su * cv);
            let pos = Vec3::new(
                cu * (major + minor * cv),
                minor * sv,
                su * (major + minor * cv),
            );
            *cell = b.push_vertex(Vertex::at(pos).with_normal(n).with_color(color));
        }
    }
    for i in 0..seg_major {
        for j in 0..seg_minor {
            let a = grid[i][j];
            let c = grid[i + 1][j];
            let d = grid[i][j + 1];
            let e = grid[i + 1][j + 1];
            b.push_triangle(a, c, d);
            b.push_triangle(d, c, e);
        }
    }
    b.build().expect("valid torus mesh")
}

/// A UV sphere with texture coordinates and a WHITE per-vertex color — the
/// shape a HPS dental scan takes: the real color lives in the texture, the
/// vertex color is neutral white. The ghost pass must sample the texture, not
/// the (white) vertex color, or a textured scan ghosts as a flat cool-white
/// "normals" shell — the reported scissors bug.
fn uv_sphere_textured(radius: f32, stacks: usize, slices: usize) -> Mesh {
    let mut b = MeshBuilder::new();
    let white = [255, 255, 255, 255];
    let mut grid = vec![vec![0u32; slices + 1]; stacks + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let phi = (i as f32 / stacks as f32) * std::f32::consts::PI;
        for (j, cell) in row.iter_mut().enumerate() {
            let theta = (j as f32 / slices as f32) * std::f32::consts::TAU;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            *cell = b.push_vertex(
                Vertex::at(n * radius)
                    .with_normal(n)
                    .with_color(white)
                    .with_uv([j as f32 / slices as f32, i as f32 / stacks as f32]),
            );
        }
    }
    for i in 0..stacks {
        for j in 0..slices {
            let a = grid[i][j];
            let c = grid[i + 1][j];
            let d = grid[i][j + 1];
            let e = grid[i + 1][j + 1];
            b.push_triangle(a, c, d);
            b.push_triangle(d, c, e);
        }
    }
    b.build().expect("valid textured sphere mesh")
}

/// A 2x2 solid-color texture.
fn solid_texture(rgba: [u8; 4]) -> MeshTexture {
    MeshTexture::new(2, 2, rgba.repeat(4))
}

fn textured_uniform() -> GpuMeshUniform {
    GpuMeshUniform {
        has_texture: 1,
        ..identity_uniform()
    }
}

fn camera(eye_z: f32) -> GpuCamera {
    let eye = Vec3::new(0.0, 0.0, eye_z);
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 100.0);
    GpuCamera::new(view, proj, Vec3::new(-0.3, 0.4, 1.0).normalize(), eye)
}

fn prepared(offscreen: &Offscreen, mesh: &Mesh) -> PreparedScene {
    offscreen.prepare_scene(&[PreparedSceneSource {
        mesh,
        uniform: identity_uniform(),
        visible: true,
        wireframe: false,
    }])
}

fn prepared_textured(offscreen: &Offscreen, mesh: &Mesh) -> PreparedScene {
    offscreen.prepare_scene(&[PreparedSceneSource {
        mesh,
        uniform: textured_uniform(),
        visible: true,
        wireframe: false,
    }])
}

/// Mean per-channel color (0..255) of a normalized rectangular patch.
fn region_mean_rgb(pixels: &[u8], size: u16, x0: f32, x1: f32, y0: f32, y1: f32) -> [f32; 3] {
    let size = usize::from(size);
    let (xa, xb) = ((x0 * size as f32) as usize, (x1 * size as f32) as usize);
    let (ya, yb) = ((y0 * size as f32) as usize, (y1 * size as f32) as usize);
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for y in ya..yb {
        for x in xa..xb {
            let i = (y * size + x) * 4;
            sum[0] += u64::from(pixels[i]);
            sum[1] += u64::from(pixels[i + 1]);
            sum[2] += u64::from(pixels[i + 2]);
            count += 1;
        }
    }
    if count == 0 {
        return [0.0; 3];
    }
    [
        sum[0] as f32 / count as f32,
        sum[1] as f32 / count as f32,
        sum[2] as f32 / count as f32,
    ]
}

/// Mean luma (0..765) of a normalized rectangular patch of an RGBA8 image.
fn region_mean_luma(pixels: &[u8], size: u16, x0: f32, x1: f32, y0: f32, y1: f32) -> f32 {
    let size = usize::from(size);
    let (xa, xb) = ((x0 * size as f32) as usize, (x1 * size as f32) as usize);
    let (ya, yb) = ((y0 * size as f32) as usize, (y1 * size as f32) as usize);
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in ya..yb {
        for x in xa..xb {
            let i = (y * size + x) * 4;
            sum += u64::from(pixels[i]) + u64::from(pixels[i + 1]) + u64::from(pixels[i + 2]);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        sum as f32 / count as f32
    }
}

#[test]
fn ghost_cut_view_fades_removed_side() {
    let _gpu = gpu_test_lock();
    let mesh = uv_sphere(1.0, 48, 64);
    let cam = camera(4.0);
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let scene = prepared(&offscreen, &mesh);

    // Vertical cut: normal +X, distance 0 keeps the right half (x >= 0) opaque
    // and ghosts the left half (x < 0). Both halves project into the frame.
    let clip = ClipPlane::new([1.0, 0.0, 0.0], 0.0);

    let ghost = pollster::block_on(offscreen.render_prepared_viewport_with_clip_and_overlay(
        &scene,
        None,
        &cam,
        &clip,
        ViewportSpec {
            size_px: [SIZE, SIZE],
            background: DARK_BG,
        },
    ))
    .expect("ghost render");

    let hard = pollster::block_on(offscreen.render_prepared_scene_with_clip(
        &scene,
        &cam,
        &clip,
        ThumbnailSpec {
            size_px: SIZE,
            background: DARK_BG,
        },
    ))
    .expect("hard-clip render");

    // Left third = cut-away side, right third = kept side, vertically centered.
    let cutaway_ghost = region_mean_luma(&ghost, SIZE, 0.22, 0.40, 0.40, 0.60);
    let cutaway_hard = region_mean_luma(&hard, SIZE, 0.22, 0.40, 0.40, 0.60);
    let kept_ghost = region_mean_luma(&ghost, SIZE, 0.60, 0.78, 0.40, 0.60);
    let kept_hard = region_mean_luma(&hard, SIZE, 0.60, 0.78, 0.40, 0.60);
    let background = region_mean_luma(&ghost, SIZE, 0.02, 0.12, 0.02, 0.12);

    // Kept side must be untouched by the ghost pass: same pixels either way.
    assert!(
        (kept_ghost - kept_hard).abs() < 6.0,
        "kept side changed with ghost pass: ghost={kept_ghost} hard={kept_hard}"
    );
    // Kept side is solid geometry, clearly brighter than the dark background.
    assert!(
        kept_ghost > background + 120.0,
        "kept side not solid: kept={kept_ghost} bg={background}"
    );
    // Hard-clip render removes the cut-away side entirely -> ~background.
    assert!(
        cutaway_hard < background + 12.0,
        "hard clip left geometry on the removed side: cutaway={cutaway_hard} bg={background}"
    );
    // Ghost render keeps the cut-away side visible but faint: brighter than the
    // hard-clip (which is background), yet dimmer than the opaque kept side.
    assert!(
        cutaway_ghost > cutaway_hard + 10.0,
        "ghost did not fill the removed side: ghost={cutaway_ghost} hard={cutaway_hard}"
    );
    assert!(
        cutaway_ghost + 40.0 < kept_ghost,
        "ghost side not clearly fainter than kept side: ghost={cutaway_ghost} kept={kept_ghost}"
    );
}

#[test]
fn disabled_clip_skips_ghost_and_matches_plain_render() {
    let _gpu = gpu_test_lock();
    let mesh = uv_sphere(1.0, 32, 48);
    let cam = camera(4.0);
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let scene = prepared(&offscreen, &mesh);
    let spec = ViewportSpec {
        size_px: [SIZE, SIZE],
        background: DARK_BG,
    };

    // A disabled clip plane through the ghost-capable path must reproduce the
    // plain (no-clip) render byte-for-byte: fs_ghost draws nothing when off.
    let disabled = ClipPlane::disabled();
    let ghost_path = pollster::block_on(
        offscreen
            .render_prepared_viewport_with_clip_and_overlay(&scene, None, &cam, &disabled, spec),
    )
    .expect("ghost-path render");
    let plain = pollster::block_on(offscreen.render_prepared_viewport(&scene, &cam, spec))
        .expect("plain render");

    assert_eq!(
        ghost_path, plain,
        "disabled clip must render identically with and without the ghost path"
    );
}

/// The reported "scissors paints normals" bug: on a TEXTURED mesh the ghost
/// pass must fade the real (textured) surface, not a flat cool-white shell.
///
/// Precise pin: render the same geometry + same cut with two very different
/// solid textures and compare the *cut-away (ghost) side*. If the ghost samples
/// the texture, the two renders differ there; if it ignores the texture (the
/// bug — it read the WHITE vertex color), both ghost sides are the identical
/// flat cool-white shell and this delta collapses to ~0.
#[test]
fn textured_ghost_tracks_texture_not_flat_shell() {
    let _gpu = gpu_test_lock();
    let mesh = uv_sphere_textured(1.0, 48, 64);
    let cam = camera(4.0);
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let spec = ViewportSpec {
        size_px: [SIZE, SIZE],
        background: DARK_BG,
    };
    // Vertical cut: keep the right half (x >= 0) opaque, ghost the left half.
    let clip = ClipPlane::new([1.0, 0.0, 0.0], 0.0);

    let render = |rgba: [u8; 4]| {
        let mut m = mesh.clone();
        m.set_texture(solid_texture(rgba));
        let scene = prepared_textured(&offscreen, &m);
        pollster::block_on(
            offscreen
                .render_prepared_viewport_with_clip_and_overlay(&scene, None, &cam, &clip, spec),
        )
        .expect("textured ghost render")
    };

    let dark = render([28, 20, 18, 255]);
    let light = render([232, 236, 242, 255]);

    // Cut-away (ghost) side = left third; kept side = right third.
    let ghost_dark = region_mean_rgb(&dark, SIZE, 0.22, 0.40, 0.40, 0.60);
    let ghost_light = region_mean_rgb(&light, SIZE, 0.22, 0.40, 0.40, 0.60);
    let ghost_dark_luma = ghost_dark[0] + ghost_dark[1] + ghost_dark[2];
    let ghost_light_luma = ghost_light[0] + ghost_light[1] + ghost_light[2];

    // The bright texture must ghost clearly brighter than the dark one: the
    // ghost is a faded version of the REAL surface, not a texture-blind shell.
    assert!(
        ghost_light_luma > ghost_dark_luma + 40.0,
        "textured ghost ignores the texture (flat shell): light={ghost_light_luma} dark={ghost_dark_luma}"
    );
    // Sanity: the kept side (opaque, fs_main) obviously tracks the texture too.
    let kept_dark = region_mean_rgb(&dark, SIZE, 0.60, 0.78, 0.40, 0.60);
    let kept_light = region_mean_rgb(&light, SIZE, 0.60, 0.78, 0.40, 0.60);
    let kept_dark_luma = kept_dark[0] + kept_dark[1] + kept_dark[2];
    let kept_light_luma = kept_light[0] + kept_light[1] + kept_light[2];
    assert!(
        kept_light_luma > kept_dark_luma + 60.0,
        "kept side should track the texture: light={kept_light_luma} dark={kept_dark_luma}"
    );
}

/// The ghost pass must never touch the KEPT side of a textured mesh: whether or
/// not the ghost runs, the opaque half is pixel-for-pixel the same.
#[test]
fn textured_cut_keeps_kept_side_identical() {
    let _gpu = gpu_test_lock();
    let mut mesh = uv_sphere_textured(1.0, 48, 64);
    mesh.set_texture(solid_texture([210, 176, 150, 255]));
    let cam = camera(4.0);
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let scene = prepared_textured(&offscreen, &mesh);
    let clip = ClipPlane::new([1.0, 0.0, 0.0], 0.0);

    let ghost = pollster::block_on(offscreen.render_prepared_viewport_with_clip_and_overlay(
        &scene,
        None,
        &cam,
        &clip,
        ViewportSpec {
            size_px: [SIZE, SIZE],
            background: DARK_BG,
        },
    ))
    .expect("ghost render");
    let hard = pollster::block_on(offscreen.render_prepared_scene_with_clip(
        &scene,
        &cam,
        &clip,
        ThumbnailSpec {
            size_px: SIZE,
            background: DARK_BG,
        },
    ))
    .expect("hard-clip render");

    let kept_ghost = region_mean_luma(&ghost, SIZE, 0.60, 0.78, 0.40, 0.60);
    let kept_hard = region_mean_luma(&hard, SIZE, 0.60, 0.78, 0.40, 0.60);
    let background = region_mean_luma(&ghost, SIZE, 0.02, 0.12, 0.02, 0.12);
    assert!(
        (kept_ghost - kept_hard).abs() < 6.0,
        "textured kept side changed with ghost pass: ghost={kept_ghost} hard={kept_hard}"
    );
    assert!(
        kept_ghost > background + 100.0,
        "textured kept side not solid: kept={kept_ghost} bg={background}"
    );
}

/// A disabled clip through the ghost-capable path on a TEXTURED mesh must
/// reproduce the plain render byte-for-byte — the armed-but-no-pose / cut-off
/// state renders identically (`fs_ghost` draws nothing, `fs_main`'s textured
/// path is untouched by the ghost fix).
#[test]
fn disabled_clip_on_textured_matches_plain() {
    let _gpu = gpu_test_lock();
    let mut mesh = uv_sphere_textured(1.0, 32, 48);
    mesh.set_texture(solid_texture([214, 176, 150, 255]));
    let cam = camera(4.0);
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let scene = prepared_textured(&offscreen, &mesh);
    let spec = ViewportSpec {
        size_px: [SIZE, SIZE],
        background: DARK_BG,
    };

    let disabled = ClipPlane::disabled();
    let ghost_path = pollster::block_on(
        offscreen
            .render_prepared_viewport_with_clip_and_overlay(&scene, None, &cam, &disabled, spec),
    )
    .expect("ghost-path render");
    let plain = pollster::block_on(offscreen.render_prepared_viewport(&scene, &cam, spec))
        .expect("plain render");

    assert_eq!(
        ghost_path, plain,
        "disabled clip on a textured mesh must render identically with the ghost path"
    );
}

/// Visual dump for human review — writes ghost + hard-clip PNGs for several
/// disc poses on a sphere and an arch (torus). Ignored by default.
#[test]
#[ignore = "writes PNGs to the scratchpad for manual inspection"]
fn ghost_cut_view_visual_dump() {
    let _gpu = gpu_test_lock();
    let out_dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/ghost-verify"
    );
    std::fs::create_dir_all(out_dir).expect("create dump dir");
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let dump = 256u16;

    let poses = [
        ("x", ClipPlane::new([1.0, 0.0, 0.0], 0.0)),
        ("y", ClipPlane::new([0.0, 1.0, 0.0], 0.0)),
        ("oblique", ClipPlane::new([1.0, 0.0, 1.0], 0.3)),
    ];

    let fixtures: [(&str, Mesh, f32); 2] = [
        ("sphere", uv_sphere(1.0, 64, 96), 4.0),
        ("arch", torus(2.0, 0.6, 96, 32), 7.0),
    ];

    for (fixture_name, mesh, eye_z) in &fixtures {
        let scene = prepared(&offscreen, mesh);
        let cam = camera(*eye_z);
        for (pose_name, clip) in &poses {
            let ghost =
                pollster::block_on(offscreen.render_prepared_viewport_with_clip_and_overlay(
                    &scene,
                    None,
                    &cam,
                    clip,
                    ViewportSpec {
                        size_px: [dump, dump],
                        background: DARK_BG,
                    },
                ))
                .expect("ghost render");
            let hard = pollster::block_on(offscreen.render_prepared_scene_with_clip(
                &scene,
                &cam,
                clip,
                ThumbnailSpec {
                    size_px: dump,
                    background: DARK_BG,
                },
            ))
            .expect("hard render");
            save_png(
                &format!("{out_dir}/{fixture_name}_{pose_name}_ghost.png"),
                &ghost,
                dump,
            );
            save_png(
                &format!("{out_dir}/{fixture_name}_{pose_name}_hard.png"),
                &hard,
                dump,
            );
        }
    }
    eprintln!("ghost-verify PNGs written to {out_dir}");
}

fn save_png(path: &str, pixels: &[u8], size: u16) {
    let img = image::RgbaImage::from_raw(u32::from(size), u32::from(size), pixels.to_vec())
        .expect("rgba buffer matches dimensions");
    img.save(path).expect("write png");
}

//! Golden-image regression test for the offscreen renderer.
//!
//! Renders a fixed scene (one triangle) at 64x64 through the Offscreen path
//! (WARP software rasterizer on Linux CI, real GPU on Windows), compares the
//! RGBA8 output to a stored PNG baseline within a tolerance.
//!
//! Baselines live in `tests/golden/baselines/<name>.png`. To regenerate after
//! an intentional shader change, delete the baseline and re-run; commit the
//! new PNG with a clear visual justification.

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, MeshTexture, Vertex};
use occluview_render::{
    ClipPlane, GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, PreparedSceneSource,
    ThumbnailSpec, ViewportSpec,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SIZE: u16 = 64;
const TOLERANCE: u8 = 8; // per-channel diff allowed
const DARK_TEST_BACKGROUND: [f64; 4] = [0.039, 0.039, 0.039, 1.0];
const TRANSPARENT_THUMBNAIL_BACKGROUND: [f64; 4] = [0.0, 0.0, 0.0, 0.0];

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("golden-image GPU test lock is not poisoned")
}

fn triangle_mesh() -> Mesh {
    let mut b = MeshBuilder::new();
    let a = b.push_vertex(Vertex::at(Vec3::new(-0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let c = b.push_vertex(Vertex::at(Vec3::new(0.5, -0.5, 0.0)).with_normal(Vec3::Z));
    let d = b.push_vertex(Vertex::at(Vec3::new(0.0, 0.5, 0.0)).with_normal(Vec3::Z));
    b.push_triangle(a, c, d);
    b.build().expect("valid triangle mesh")
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

fn render_to_pixels() -> Vec<u8> {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    pollster::block_on(offscreen.render(&mesh, &cam, dark_thumbnail_spec())).expect("render")
}

fn dark_thumbnail_spec() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: SIZE,
        background: DARK_TEST_BACKGROUND,
    }
}

fn identity_uniform(tint: [f32; 4], opacity: f32) -> GpuMeshUniform {
    GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ],
        tint,
        opacity,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
    }
}

fn pixel_at(pixels: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    let start = (y * width + x) * 4;
    &pixels[start..start + 4]
}

#[test]
fn default_thumbnail_background_is_transparent() {
    let spec = ThumbnailSpec::default();
    for (actual, expected) in spec
        .background
        .into_iter()
        .zip(TRANSPARENT_THUMBNAIL_BACKGROUND)
    {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}

/// Rewrites `tests/golden/baselines/triangle.png` from the current renderer.
/// Run deliberately after an intentional shader change:
/// `cargo test -p occluview-render --test golden_image regenerate_golden_triangle -- --ignored`
#[test]
#[ignore = "regenerates the committed golden baseline; run only after an intentional shader change"]
fn regenerate_golden_triangle() {
    let pixels = render_to_pixels();
    let baseline_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/baselines");
    let baseline_path = format!("{baseline_dir}/triangle.png");
    let img =
        image::RgbaImage::from_raw(u32::from(SIZE), u32::from(SIZE), pixels).expect("rgba buffer");
    img.save(&baseline_path).expect("write golden baseline");
}

#[test]
fn golden_triangle_matches_baseline() {
    let pixels = render_to_pixels();
    let baseline_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/baselines");
    let baseline_path = format!("{baseline_dir}/triangle.png");
    let baseline_bytes = std::fs::read(&baseline_path).expect("golden baseline is committed");
    let baseline = image::load_from_memory(&baseline_bytes)
        .expect("baseline PNG decodes")
        .to_rgba8()
        .to_vec();

    assert_eq!(
        pixels.len(),
        baseline.len(),
        "rendered size differs from baseline"
    );
    let mut max_diff = 0u8;
    let mut diffs_above = 0usize;
    for (a, b) in pixels.iter().zip(baseline.iter()) {
        let d = a.abs_diff(*b);
        if d > TOLERANCE {
            diffs_above += 1;
        }
        if d > max_diff {
            max_diff = d;
        }
    }
    // Allow a small fraction of pixels to exceed tolerance (antialiasing edges,
    // rasterization differences between GPU vendors).
    let total_pixels = usize::from(SIZE) * usize::from(SIZE);
    let diff_basis_points = diffs_above * 10_000 / total_pixels;
    assert!(
        diffs_above * 20 < total_pixels,
        "golden mismatch: {diffs_above}/{total_pixels} pixels ({}.{:02}%) exceed tolerance {TOLERANCE}, max_diff={max_diff}",
        diff_basis_points / 100,
        diff_basis_points % 100
    );
}

#[test]
fn prepared_viewport_renders_rectangular_extent() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let uniform = identity_uniform([1.0, 1.0, 1.0, 1.0], 1.0);
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let prepared = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform,
        visible: true,
        wireframe: false,
    }]);
    let spec = ViewportSpec {
        size_px: [96, 48],
        background: [0.78, 0.80, 0.82, 1.0],
    };

    let pixels = pollster::block_on(offscreen.render_prepared_viewport(&prepared, &cam, spec))
        .expect("render prepared viewport");

    assert_eq!(pixels.len(), 96 * 48 * 4);
}

#[test]
fn prepared_scene_opacity_blends_with_background() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let spec = ViewportSpec {
        size_px: [SIZE, SIZE],
        background: [0.0, 0.0, 0.0, 1.0],
    };

    let opaque_uniform = identity_uniform([1.0, 0.0, 0.0, 1.0], 1.0);
    let opaque = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: opaque_uniform,
        visible: true,
        wireframe: false,
    }]);
    let opaque_pixels = pollster::block_on(offscreen.render_prepared_viewport(&opaque, &cam, spec))
        .expect("render opaque");

    let transparent_uniform = identity_uniform([1.0, 0.0, 0.0, 1.0], 0.5);
    let transparent = offscreen.prepare_scene(&[PreparedSceneSource {
        mesh: &mesh,
        uniform: transparent_uniform,
        visible: true,
        wireframe: false,
    }]);
    let transparent_pixels =
        pollster::block_on(offscreen.render_prepared_viewport(&transparent, &cam, spec))
            .expect("render transparent");

    let opaque_center = pixel_at(&opaque_pixels, usize::from(SIZE), 32, 32);
    let transparent_center = pixel_at(&transparent_pixels, usize::from(SIZE), 32, 32);

    assert!(
        transparent_center[0] > 16,
        "transparent triangle did not render: {transparent_center:?}"
    );
    assert!(
        transparent_center[0] < opaque_center[0],
        "opacity did not reduce red channel: transparent={transparent_center:?} opaque={opaque_center:?}"
    );
}

/// A small point cloud: 5 points spread across the view.
fn point_cloud_mesh() -> Mesh {
    use occluview_core::MeshKind;
    let mut b = MeshBuilder::new();
    for (x, y) in [
        (-0.5, -0.5),
        (0.5, -0.5),
        (0.0, 0.5),
        (-0.3, 0.0),
        (0.3, 0.0),
    ] {
        b.push_vertex(Vertex::at(Vec3::new(x, y, 0.0)).with_normal(Vec3::Z));
    }
    let _ = MeshKind::PointCloud; // document intent
    b.as_point_cloud().build().expect("valid point cloud")
}

/// A textured-triangle golden test: validates the full texture pipeline
/// (Vertex.uv -> WGSL sampler -> tint -> lighting) end-to-end on WARP. Uses
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

/// A 2x2 checkerboard: top-left + bottom-right red, other two green.
fn checkerboard_texture() -> MeshTexture {
    MeshTexture::new(
        2,
        2,
        vec![
            255, 0, 0, 255, // (0,0) red
            0, 255, 0, 255, // (1,0) green
            0, 255, 0, 255, // (0,1) green
            255, 0, 0, 255, // (1,1) red
        ],
    )
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
    };
    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: Some(&gpu_tex),
    }];
    pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
        .expect("render scene")
}

/// The render/GPU path must preserve channel order: a warm-white dental texture
/// (R > B) must render warm, and a pure-blue texture must render blue. This is
/// the counterpart to the HPS decode fix — it proves the R<->B swap that turns
/// scans blue is NOT in the GPU upload/sampler/shader/readback path (uploaded as
/// `Rgba8UnormSrgb`, sampled `tex.rgb`, read back with only a vertical flip).
#[test]
fn textured_render_preserves_channel_order() {
    // Warm white, the canonical dental enamel color (R >= G > B).
    let warm = render_uniform_textured(&uniform_texture([250, 240, 225, 255]));
    let mut warm_lit = 0usize;
    let mut warm_ok = 0usize;
    for px in warm.chunks_exact(4) {
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
    for px in blue.chunks_exact(4) {
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

#[test]
fn textured_triangle_renders_checkerboard() {
    let _gpu = gpu_test_lock();
    let mesh = textured_triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let device = offscreen.renderer().device();
    let queue = offscreen.renderer().queue();

    // Upload the checkerboard texture.
    let gpu_tex = GpuTexture::upload(offscreen.renderer(), device, queue, &checkerboard_texture());

    // Per-mesh uniform: identity model, white tint, full opacity, has_texture=1.
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
    };

    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: Some(&gpu_tex),
    }];
    let spec = dark_thumbnail_spec();
    let pixels =
        pollster::block_on(offscreen.render_scene(&entries, &cam, spec)).expect("render scene");

    // The triangle covers the center of the frame. With a 2x2 checker and
    // linear filtering, sampled colors range between red and green. Assert:
    // (1) there are visible pixels (not all background),
    // (2) both red-dominant and green-dominant pixels appear (the checkerboard
    //     is actually being sampled, not a flat color).
    let bg = [10, 10, 10, 255];
    let mut non_bg = 0usize;
    let mut red_dominant = 0usize;
    let mut green_dominant = 0usize;
    for px in pixels.chunks_exact(4) {
        if px[0] == bg[0] && px[1] == bg[1] && px[2] == bg[2] {
            continue;
        }
        non_bg += 1;
        let (r, g) = (i32::from(px[0]), i32::from(px[1]));
        if r > g + 20 {
            red_dominant += 1;
        } else if g > r + 20 {
            green_dominant += 1;
        }
    }
    assert!(non_bg > 50, "textured triangle rendered almost nothing");
    assert!(
        red_dominant > 5 && green_dominant > 5,
        "checkerboard not visible: red={red_dominant} green={green_dominant} \
         (expected both > 5 — texture sampling may be broken)"
    );
}

/// Validates the clip-plane discard (Approach A, "hollow cut") on WARP. A
/// triangle centered at the origin is clipped by a plane at `distance = 0`
/// with normal `+Z` pointing toward the camera — the back half is discarded,
/// leaving fewer visible pixels than the unclipped triangle. Verifies the
/// WGSL `discard` branch and the `ClipPlane` uniform binding work end-to-end.
#[test]
fn cut_triangle_discard_removes_pixels() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");

    // Render unclipped first to count the baseline pixels.
    let spec = dark_thumbnail_spec();
    let full_pixels = pollster::block_on(offscreen.render(&mesh, &cam, spec)).expect("full render");
    let full_visible = full_pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();

    // Render clipped: plane normal +Y, distance 0 — discards the top half
    // of the triangle (where world Y > 0).
    let clip = ClipPlane::new([0.0, 1.0, 0.0], 0.0);
    let cut_pixels =
        pollster::block_on(offscreen.render_clipped(&mesh, &cam, &clip, spec)).expect("cut render");
    let cut_visible = cut_pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();

    // The cut must remove a meaningful fraction of pixels (the top half),
    // but leave some (the bottom half). Use a loose bound to tolerate
    // rasterization edge effects.
    assert!(
        cut_visible < full_visible * 3 / 4,
        "clip did not remove pixels: full={full_visible} cut={cut_visible}"
    );
    assert!(
        cut_visible > full_visible / 8,
        "clip removed too much (expected roughly half): full={full_visible} cut={cut_visible}"
    );

    // A disabled clip plane must reproduce the full render.
    let disabled = ClipPlane::disabled();
    let identity_pixels =
        pollster::block_on(offscreen.render_clipped(&mesh, &cam, &disabled, spec))
            .expect("identity");
    let identity_visible = identity_pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert_eq!(
        identity_visible, full_visible,
        "disabled clip plane did not match unclipped render"
    );
}

/// Validates the full 3-pass stencil capping (Approach B, "solid cut") on
/// WARP. The render must not crash and must produce visible output — the
/// stencil increment/decrement + cap draw sequence runs end-to-end.
#[test]
fn cut_triangle_capped_renders() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");

    let cut = occluview_render::CutViewSpec {
        plane: ClipPlane::new([0.0, 1.0, 0.0], 0.0),
        cap_color: [0.0, 1.0, 0.0, 1.0],
        show_hollow: false,
    };
    let spec = dark_thumbnail_spec();
    let pixels = pollster::block_on(offscreen.render_with_cut(&mesh, &cam, &cut, 10.0, spec))
        .expect("cut render");

    let non_bg = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(non_bg > 0, "capped cut rendered nothing visible");
}

/// Validates the convenience entry point `render_cut_view` — auto-frames an
/// orthographic camera along the plane normal and renders the solid cut.
/// Proves the full cut-view pipeline (camera + clip + stencil cap) runs
/// end-to-end on WARP without crashing.
#[test]
fn render_cut_view_end_to_end() {
    let _gpu = gpu_test_lock();
    let mesh = triangle_mesh();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let cut = occluview_render::CutViewSpec {
        plane: ClipPlane::new([0.0, 0.0, 1.0], 0.0),
        cap_color: [1.0, 0.0, 0.0, 1.0],
        show_hollow: false,
    };
    let spec = dark_thumbnail_spec();
    let pixels =
        pollster::block_on(offscreen.render_cut_view(&mesh, &cut, spec)).expect("render_cut_view");
    let non_bg = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(non_bg > 0, "render_cut_view produced nothing visible");
}

fn render_point_cloud_to_pixels() -> Vec<u8> {
    let _gpu = gpu_test_lock();
    let mesh = point_cloud_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let spec = dark_thumbnail_spec();
    pollster::block_on(offscreen.render(&mesh, &cam, spec)).expect("render")
}

#[test]
fn point_cloud_renders_readable_splats() {
    let pixels = render_point_cloud_to_pixels();
    let non_bg = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(
        non_bg > 80,
        "point cloud stayed sparse ({non_bg} non-bg pixels); expected readable splats"
    );
    assert!(
        non_bg < 400,
        "point cloud splats grew too large ({non_bg} non-bg pixels)"
    );
}

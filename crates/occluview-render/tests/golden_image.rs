//! Golden-image regression test for the offscreen renderer.
//!
//! Renders a fixed scene (one triangle) at 64x64 through the Offscreen path on
//! a software rasterizer -- Lavapipe in the Linux CI job, WARP on the Windows
//! runner, both selected in `.github/workflows/ci.yml` -- and compares the
//! RGBA8 output to a stored PNG baseline within a tolerance.
//!
//! Baselines live in `tests/golden/baselines/<name>.png`. A missing baseline
//! is a failure, not an invitation to write one: deleting it makes the test
//! panic. To regenerate after an intentional shader change, run the ignored
//! `regenerate_golden_triangle` below, then commit the new PNG with a clear
//! visual justification.

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
        show_texture: 1,
        measured_map: 0,
        padding: [0; 2],
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
        show_texture: 1,
        measured_map: 0,
        padding: [0; 2],
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
    for px in pixels.as_chunks::<4>().0 {
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

/// Validates the clip-plane discard (Approach A, "hollow cut") on the software rasterizer. A
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
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();

    // Render clipped: plane normal +Y, distance 0 — discards the top half
    // of the triangle (where world Y > 0).
    let clip = ClipPlane::new([0.0, 1.0, 0.0], 0.0);
    let cut_pixels =
        pollster::block_on(offscreen.render_clipped(&mesh, &cam, &clip, spec)).expect("cut render");
    let cut_visible = cut_pixels
        .as_chunks::<4>()
        .0
        .iter()
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
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert_eq!(
        identity_visible, full_visible,
        "disabled clip plane did not match unclipped render"
    );
}

/// Validates the full 3-pass stencil capping (Approach B, "solid cut") on the
/// software rasterizer. The render must not crash and must produce visible
/// output — the
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
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(non_bg > 0, "capped cut rendered nothing visible");
}

/// Validates the convenience entry point `render_cut_view` — auto-frames an
/// orthographic camera along the plane normal and renders the solid cut.
/// Proves the full cut-view pipeline (camera + clip + stencil cap) runs
/// end-to-end on the software rasterizer without crashing.
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
        .as_chunks::<4>()
        .0
        .iter()
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
        .as_chunks::<4>()
        .0
        .iter()
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

/// A shallow dome in one flat colour.
///
/// Curvature is the point: a flat triangle has uniform luminance under any
/// lighting, so a shading test built on one would pass whatever the shader did.
/// This fan has a raised centre, so its normals — and therefore its shading —
/// vary across the surface.
fn colored_dome_mesh(color: [u8; 4]) -> Mesh {
    const RING: usize = 12;
    let mut builder = MeshBuilder::new();
    let apex = builder.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.0, 0.45))
            .with_normal(Vec3::Z)
            .with_color(color),
    );
    let rim: Vec<u32> = (0..RING)
        .map(|step| {
            #[allow(clippy::cast_precision_loss)]
            let angle = std::f32::consts::TAU * step as f32 / RING as f32;
            let position = Vec3::new(0.6 * angle.cos(), 0.6 * angle.sin(), 0.0);
            builder.push_vertex(
                Vertex::at(position)
                    .with_normal((position + Vec3::Z * 0.35).normalize())
                    .with_color(color),
            )
        })
        .collect();
    for step in 0..RING {
        builder.push_triangle(apex, rim[step], rim[(step + 1) % RING]);
    }
    builder.build().expect("valid dome mesh")
}

/// A measured colour map has two jobs at once, and an earlier version of this
/// flag only did the first: it emitted the exact colour with no lighting, which
/// left the surface a flat silhouette with no readable form. A heat map you
/// cannot see the shape of tells you nothing about a scan.
///
/// So this test requires BOTH: the hue must survive (the ramp is the
/// measurement), and the surface must still be shaded (luminance has to vary
/// across it). It is also the only check that the flag reaches the shader at
/// the right offset — a struct-layout slip would read another field's bits and
/// could not be caught by comparing field names.
#[test]
fn a_measured_map_keeps_its_hue_and_its_shading() {
    let _gpu = gpu_test_lock();
    let color = [24u8, 200, 64, 255];
    let mesh = colored_dome_mesh(color);
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");

    let render = |measured_map: u32| {
        let uniform = GpuMeshUniform {
            measured_map,
            ..identity_uniform([1.0, 1.0, 1.0, 1.0], 1.0)
        };
        let entries = [occluview_render::SceneDrawEntry {
            mesh: &mesh,
            uniform: &uniform,
            texture: None,
        }];
        pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
            .expect("render scene")
    };

    let map_pixels = render(1);
    let lit_pixels = render(0);

    let background = DARK_TEST_BACKGROUND;
    let is_background = |px: &[u8]| {
        (f64::from(px[0]) / 255.0 - background[0]).abs() < 0.02
            && (f64::from(px[1]) / 255.0 - background[1]).abs() < 0.02
    };

    let mut covered = 0usize;
    let mut brightest = 0u8;
    let mut darkest = 255u8;
    for px in map_pixels.as_chunks::<4>().0 {
        if is_background(px) {
            continue;
        }
        covered += 1;
        // Hue: green dominates this colour and blue beats red. Shading scales
        // every channel together, so the ordering must survive it.
        assert!(
            px[1] > px[2] && px[2] > px[0],
            "the measured hue did not survive: {px:?}"
        );
        brightest = brightest.max(px[1]);
        darkest = darkest.min(px[1]);
    }
    assert!(covered > 50, "the mapped triangle rendered almost nothing");
    assert!(
        brightest.saturating_sub(darkest) > 8,
        "a measured map must still be shaded: the surface came out flat \
         ({darkest}..{brightest}), which is the unreadable blob this flag once produced"
    );

    let differs_from_plain_lit = lit_pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(map_pixels.as_chunks::<4>().0.iter())
        .any(|(lit, mapped)| !is_background(mapped) && lit[..3] != mapped[..3]);
    assert!(
        differs_from_plain_lit,
        "the mapped pass must differ from the ordinary lit one, or this proves nothing"
    );
}

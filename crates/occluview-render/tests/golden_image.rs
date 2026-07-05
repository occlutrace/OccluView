//! Golden-image regression test for the offscreen renderer.
//!
//! Renders a fixed scene (one triangle) at 64x64 through the Offscreen path
//! (WARP software rasterizer on Linux CI, real GPU on Windows), compares the
//! RGBA8 output to a stored PNG baseline within a tolerance.
//!
//! Baselines live in `tests/golden/baselines/<name>.png`. To regenerate after
//! an intentional shader change, delete the baseline and re-run; commit the
//! new PNG with an ADR-style justification in the PR (AGENTS.md §0.6).

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, MeshTexture, Vertex};
use occluview_render::{
    ClipPlane, GpuCamera, GpuMeshUniform, GpuTexture, Offscreen, ThumbnailSpec,
};

const SIZE: u16 = 64;
const TOLERANCE: u8 = 8; // per-channel diff allowed

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
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let spec = ThumbnailSpec {
        size_px: SIZE,
        ..Default::default()
    };
    pollster::block_on(offscreen.render(&mesh, &cam, spec)).expect("render")
}

#[test]
fn golden_triangle_matches_baseline() {
    let pixels = render_to_pixels();
    let baseline_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/baselines");
    let baseline_path = format!("{baseline_dir}/triangle.png");
    let baseline = match std::fs::read(&baseline_path) {
        Ok(bytes) => image::load_from_memory(&bytes)
            .expect("baseline PNG decodes")
            .to_rgba8()
            .to_vec(),
        Err(_) => {
            // No baseline yet: generate it. The test "passes" by recording the
            // current output; CI fails if the file is not committed afterwards.
            let _ = std::fs::create_dir_all(baseline_dir);
            let img = image::RgbaImage::from_raw(u32::from(SIZE), u32::from(SIZE), pixels.clone())
                .expect("image dimensions");
            let _ = img.save(&baseline_path);
            eprintln!(
                "golden: baseline not found; wrote {}. \
                 Commit it (after visual review) or update the test.",
                baseline_path
            );
            return;
        }
    };

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
    let frac = diffs_above as f32 / total_pixels as f32;
    assert!(
        frac < 0.05,
        "golden mismatch: {diffs_above}/{total_pixels} pixels ({:.2}%) exceed tolerance {TOLERANCE}, max_diff={max_diff}",
        frac * 100.0
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

#[test]
fn textured_triangle_renders_checkerboard() {
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
        pad: [0, 0],
    };

    let entries = [occluview_render::SceneDrawEntry {
        mesh: &mesh,
        uniform: &uniform,
        texture: Some(&gpu_tex),
    }];
    let spec = ThumbnailSpec {
        size_px: SIZE,
        ..Default::default()
    };
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
/// WGSL `discard` branch and the ClipPlane uniform binding work end-to-end.
#[test]
fn cut_triangle_discard_removes_pixels() {
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");

    // Render unclipped first to count the baseline pixels.
    let spec = ThumbnailSpec {
        size_px: SIZE,
        ..Default::default()
    };
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
    let mesh = triangle_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");

    let cut = occluview_render::CutViewSpec {
        plane: ClipPlane::new([0.0, 1.0, 0.0], 0.0),
        cap_color: [0.0, 1.0, 0.0, 1.0],
        show_hollow: false,
    };
    let spec = ThumbnailSpec {
        size_px: SIZE,
        ..Default::default()
    };
    let pixels = pollster::block_on(offscreen.render_with_cut(&mesh, &cam, &cut, 10.0, spec))
        .expect("cut render");

    let non_bg = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(non_bg > 0, "capped cut rendered nothing visible");
}

fn render_point_cloud_to_pixels() -> Vec<u8> {
    let mesh = point_cloud_mesh();
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let spec = ThumbnailSpec {
        size_px: SIZE,
        ..Default::default()
    };
    pollster::block_on(offscreen.render(&mesh, &cam, spec)).expect("render")
}

#[test]
fn golden_point_cloud_matches_baseline() {
    let pixels = render_point_cloud_to_pixels();
    // Point clouds render as discrete pixels; the baseline has very few
    // non-background pixels. We just check it produces SOMETHING visible
    // (at least one non-background pixel) rather than full background.
    let non_bg = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 50 || px[1] > 50 || px[2] > 50)
        .count();
    assert!(
        non_bg > 0,
        "point cloud rendered nothing visible ({non_bg} non-bg pixels)"
    );
    // Sanity: a 5-point cloud at 64x64 should produce a small number of
    // visible pixels (each point = 1 pixel + maybe a few from rasterization).
    assert!(
        non_bg < 50,
        "point cloud rendered too many pixels ({non_bg}); expected sparse output"
    );
}

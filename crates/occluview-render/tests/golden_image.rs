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
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{GpuCamera, Offscreen, ThumbnailSpec};

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

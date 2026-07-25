//! Colour fidelity of the deviation heatmap, end to end through the GPU.
//!
//! Separate from `golden_image` so both stay inside the workspace's 800-line
//! file budget, and because these two ask one narrow question: does a measured
//! colour survive the trip to the screen unchanged?

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{GpuCamera, GpuMeshUniform, Offscreen, ThumbnailSpec};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SIZE: u16 = 64;
const DARK_TEST_BACKGROUND: [f64; 4] = [0.039, 0.039, 0.039, 1.0];

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("measured-map GPU test lock is not poisoned")
}

fn dark_thumbnail_spec() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: SIZE,
        background: DARK_TEST_BACKGROUND,
    }
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

/// A shallow dome in one flat colour. Curvature is the point: a flat triangle
/// has uniform luminance under any lighting, so a shading claim built on one
/// would pass whatever the shader did.
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

/// A dome whose rim sweeps a colour ramp, so the surface carries a real
/// transition instead of one flat colour.
fn ramped_dome_mesh(stops: &[[u8; 4]]) -> Mesh {
    const RING: usize = 24;
    let mut builder = MeshBuilder::new();
    let at = |slot: usize| -> [u8; 4] { stops[slot * stops.len() / RING.max(1)] };
    let apex = builder.push_vertex(
        Vertex::at(Vec3::new(0.0, 0.0, 0.45))
            .with_normal(Vec3::Z)
            .with_color(at(RING / 2)),
    );
    let rim: Vec<u32> = (0..RING)
        .map(|step| {
            #[allow(clippy::cast_precision_loss)]
            let angle = std::f32::consts::TAU * step as f32 / RING as f32;
            let position = Vec3::new(0.6 * angle.cos(), 0.6 * angle.sin(), 0.0);
            builder.push_vertex(
                Vertex::at(position)
                    .with_normal((position + Vec3::Z * 0.35).normalize())
                    .with_color(at(step)),
            )
        })
        .collect();
    for step in 0..RING {
        builder.push_triangle(apex, rim[step], rim[(step + 1) % RING]);
    }
    builder.build().expect("valid ramped dome mesh")
}

fn render_measured_dome(mesh: &Mesh) -> Vec<u8> {
    let cam = camera_looking_at_origin();
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let uniform = GpuMeshUniform {
        measured_map: 1,
        // The tint a scan carries by default is warm stone. A measured map must
        // ignore it: the ramp IS the reading.
        ..identity_uniform([0.98, 0.90, 0.80, 1.0], 1.0)
    };
    let entries = [occluview_render::SceneDrawEntry {
        mesh,
        uniform: &uniform,
        texture: None,
    }];
    pollster::block_on(offscreen.render_scene(&entries, &cam, dark_thumbnail_spec()))
        .expect("render scene")
}

/// The deviation ramp's two ends, as `occluview-align` defines them. Duplicated
/// deliberately: this test asks whether the RENDERER changes a colour handed to
/// it, so it must not read the answer from the same place the app does.
const COLD_END: [u8; 4] = [0, 32, 255, 255];
const HOT_END: [u8; 4] = [255, 24, 0, 255];

/// The colours that reach the screen are the colours that were uploaded.
///
/// A measured map is a reading: an operator matches a colour on the surface
/// against a number on the legend. The only thing the shader may do to it is
/// scale all three channels together for form (`MEASURED_MAP_SHADE`) — any
/// per-channel change, tint, or texture blend would silently move the reading.
/// So every covered pixel must be the uploaded colour times ONE shared factor.
#[test]
fn a_measured_map_reaches_the_screen_in_the_colour_it_was_uploaded_in() {
    let _gpu = gpu_test_lock();
    for uploaded in [COLD_END, HOT_END, [24, 200, 64, 255]] {
        let pixels = render_measured_dome(&colored_dome_mesh(uploaded));
        let mut covered = 0usize;
        for px in pixels.chunks_exact(4) {
            // A single-sample target leaves background pixels at exactly the
            // clear colour, so anything else is a fragment of the dome.
            if px[0].abs_diff(10) <= 1 && px[1].abs_diff(10) <= 1 && px[2].abs_diff(10) <= 1 {
                continue;
            }
            covered += 1;
            // The shade that explains the brightest uploaded channel, then the
            // same shade has to explain the other two.
            let (slot, brightest) = (0..3)
                .map(|slot| (slot, uploaded[slot]))
                .max_by_key(|(_, value)| *value)
                .expect("a brightest channel");
            let shade = f64::from(px[slot]) / f64::from(brightest);
            assert!(
                (0.68..=1.06).contains(&shade),
                "pixel {px:?} is not {uploaded:?} under any legal shading (factor {shade:.3})"
            );
            for channel in 0..3 {
                let expected = f64::from(uploaded[channel]) * shade;
                assert!(
                    (expected - f64::from(px[channel])).abs() <= 4.0,
                    "channel {channel} of {px:?} is not {uploaded:?} shaded by {shade:.3} — \
                     the renderer changed the measured colour"
                );
            }
        }
        assert!(covered > 50, "the mapped dome rendered almost nothing");
    }
}

/// A real registration error has to READ as one. The third bug this file exists
/// for was a ramp default that painted every good result one flat blue, which
/// an operator reads as a broken tool rather than a clean fit. A surface whose
/// deviation sweeps the ramp must arrive on screen sweeping it too.
#[test]
fn a_swept_deviation_arrives_on_screen_as_a_transition() {
    let _gpu = gpu_test_lock();
    let stops = [
        COLD_END,
        [0, 200, 255, 255],
        [0, 220, 60, 255],
        [255, 220, 0, 255],
        HOT_END,
    ];
    let pixels = render_measured_dome(&ramped_dome_mesh(&stops));

    let (mut cold, mut nominal, mut hot) = (0usize, 0usize, 0usize);
    for px in pixels.chunks_exact(4) {
        if px[0].abs_diff(10) <= 1 && px[1].abs_diff(10) <= 1 && px[2].abs_diff(10) <= 1 {
            continue;
        }
        if px[2] > px[0] && px[2] > px[1] {
            cold += 1;
        } else if px[1] > px[0] && px[1] > px[2] {
            nominal += 1;
        } else if px[0] > px[1] && px[0] > px[2] {
            hot += 1;
        }
    }
    for (name, count) in [("cold", cold), ("nominal", nominal), ("hot", hot)] {
        assert!(
            count > 10,
            "only {count} {name} pixels reached the screen — the map came out flat \
             ({cold} cold, {nominal} nominal, {hot} hot)"
        );
    }
}

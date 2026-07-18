//! DIAGNOSTIC (ignored): reproduce the app's mini-slice-window render path and
//! dump PNGs comparing the camera placed on the KEPT side (current behavior)
//! vs the CUT-AWAY side (proposed section-view fix). Two-jaw fixture.
//!
//! `cargo test -p occluview-render --test cutview_diag -- --ignored --nocapture`

#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::many_single_char_names,
    clippy::too_many_arguments,
    clippy::print_stderr
)]

use glam::{Mat4, Vec3};
use occluview_core::{Aabb, Mesh, MeshBuilder, Vertex};
use occluview_render::{
    cut_view_camera_focused, slice_view_basis, ClipPlane, GpuCamera, GpuMeshUniform, Offscreen,
    PreparedScene, PreparedSceneSource, ThumbnailSpec,
};

const BG: [f64; 4] = [0.93, 0.94, 0.95, 1.0];

fn identity_uniform(tint: [f32; 4]) -> GpuMeshUniform {
    GpuMeshUniform {
        model: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        tint,
        opacity: 1.0,
        has_texture: 0,
        show_orientation: 0,
        show_vertex_colors: 1,
    }
}

/// An OPEN trough "arch" (only the top arc of the tube cross-section) centered
/// at `center` — a faithful stand-in for an open dental occlusal-surface shell,
/// which has NO closed interior, so a plane section is a visible cut edge.
fn arch(center: Vec3, major: f32, minor: f32, color: [u8; 4]) -> Mesh {
    let (seg_major, seg_minor) = (120usize, 20usize);
    // Open cross-section: only the outer/top arc of the tube (a U/gutter).
    let v0 = -0.28 * std::f32::consts::TAU;
    let v1 = 0.28 * std::f32::consts::TAU;
    let mut b = MeshBuilder::new();
    let mut grid = vec![vec![0u32; seg_minor + 1]; seg_major + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let u = (i as f32 / seg_major as f32) * std::f32::consts::TAU;
        let (su, cu) = u.sin_cos();
        for (j, cell) in row.iter_mut().enumerate() {
            let v = v0 + (v1 - v0) * (j as f32 / seg_minor as f32);
            let (sv, cv) = v.sin_cos();
            let n = Vec3::new(cu * cv, sv, su * cv);
            let pos = center
                + Vec3::new(
                    cu * (major + minor * cv),
                    minor * sv,
                    su * (major + minor * cv),
                );
            *cell = b.push_vertex(Vertex::at(pos).with_normal(n).with_color(color));
        }
    }
    for i in 0..seg_major {
        for j in 0..seg_minor {
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
    b.build().expect("valid arch mesh")
}

/// A UV sphere centered at `center`.
fn sphere(center: Vec3, radius: f32, color: [u8; 4]) -> Mesh {
    let (stacks, slices) = (48usize, 64usize);
    let mut b = MeshBuilder::new();
    let mut grid = vec![vec![0u32; slices + 1]; stacks + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        let phi = (i as f32 / stacks as f32) * std::f32::consts::PI;
        for (j, cell) in row.iter_mut().enumerate() {
            let theta = (j as f32 / slices as f32) * std::f32::consts::TAU;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
            *cell = b.push_vertex(
                Vertex::at(center + n * radius)
                    .with_normal(n)
                    .with_color(color),
            );
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
    b.build().expect("valid sphere mesh")
}

fn two_jaw(offscreen: &Offscreen) -> (PreparedScene, Aabb) {
    // Two "tooth" spheres straddling the cut plane so the section is a clear
    // disc face, plus arches to give a full-mesh silhouette.
    let upper = arch(Vec3::new(0.0, 0.6, 0.0), 2.0, 0.55, [222, 210, 196, 255]);
    let lower = arch(Vec3::new(0.0, -0.6, 0.0), 2.0, 0.55, [206, 198, 214, 255]);
    let tooth_u = sphere(Vec3::new(0.4, 0.5, 2.2), 0.55, [230, 218, 202, 255]);
    let tooth_l = sphere(Vec3::new(0.4, -0.5, 2.2), 0.55, [210, 202, 218, 255]);
    let bbox = Aabb::from_min_max(Vec3::new(-2.6, -1.4, -2.6), Vec3::new(2.6, 1.4, 3.1));
    let scene = offscreen.prepare_scene(&[
        PreparedSceneSource {
            mesh: &upper,
            uniform: identity_uniform([1.0, 1.0, 1.0, 1.0]),
            visible: true,
            wireframe: false,
        },
        PreparedSceneSource {
            mesh: &lower,
            uniform: identity_uniform([1.0, 1.0, 1.0, 1.0]),
            visible: true,
            wireframe: false,
        },
        PreparedSceneSource {
            mesh: &tooth_u,
            uniform: identity_uniform([1.0, 1.0, 1.0, 1.0]),
            visible: true,
            wireframe: false,
        },
        PreparedSceneSource {
            mesh: &tooth_l,
            uniform: identity_uniform([1.0, 1.0, 1.0, 1.0]),
            visible: true,
            wireframe: false,
        },
    ]);
    (scene, bbox)
}

/// Camera placement helper. `side = +1` puts the eye on the `+normal` (KEPT)
/// side (current behavior); `side = -1` on the CUT-AWAY side (proposed fix).
fn side_camera(plane: &ClipPlane, focus: Vec3, half_extent: f32, side: f32) -> GpuCamera {
    let normal = Vec3::from_array(plane.normal).normalize_or(Vec3::Y);
    let half_extent = half_extent.max(0.1);
    let distance = half_extent * 8.0 + 1.0;
    let eye = focus + normal * distance * side;
    let up = if normal.dot(Vec3::Y).abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let view = Mat4::look_at_rh(eye, focus, up);
    let proj = Mat4::orthographic_rh(
        -half_extent,
        half_extent,
        -half_extent,
        half_extent,
        0.1,
        distance * 2.0 + half_extent,
    );
    GpuCamera::new(view, proj, Vec3::new(0.4, 0.8, 0.5), eye)
}

#[test]
#[ignore = "writes PNGs to the scratchpad for manual inspection"]
fn cutview_camera_side_dump() {
    let out_dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/cutview-polish"
    );
    std::fs::create_dir_all(out_dir).expect("create dump dir");
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let (scene, bbox) = two_jaw(&offscreen);

    // Sawblade cut: an upright plane through the arch. Normal ~ +X keeps the
    // right half. Focus on the front of the arch, framed tight to a "disc".
    let focus = Vec3::new(0.4, 0.0, 2.0);
    let half_extent = 1.6; // ~ disc radius * 1.6
    let poses: [(&str, [f32; 3], f32); 2] = [
        ("xcut", [1.0, 0.0, 0.2], focus_dist(focus, [1.0, 0.0, 0.2])),
        (
            "oblique",
            [0.8, 0.3, 0.5],
            focus_dist(focus, [0.8, 0.3, 0.5]),
        ),
    ];

    for (name, n, dist) in &poses {
        let plane = ClipPlane::new(*n, *dist);
        let spec = ThumbnailSpec {
            size_px: 320,
            background: BG,
        };

        // (0) "Full mesh" baseline: no clip, whole-bbox framing — what the owner
        // says the window WRONGLY shows.
        let full_cam = side_camera(
            &ClipPlane::new(*n, *dist),
            bbox.center(),
            bbox.half_diagonal(),
            -1.0,
        );
        let full = pollster::block_on(offscreen.render_prepared_scene_with_clip(
            &scene,
            &full_cam,
            &ClipPlane::disabled(),
            spec,
        ))
        .expect("full render");
        save_png(
            &format!("{out_dir}/baseline_{name}_fullmesh.png"),
            &full,
            320,
        );

        let kept_cam = side_camera(&plane, focus, half_extent, 1.0);
        let kept = pollster::block_on(
            offscreen.render_prepared_scene_with_clip(&scene, &kept_cam, &plane, spec),
        )
        .expect("kept render");
        save_png(&format!("{out_dir}/before_{name}_keptside.png"), &kept, 320);

        // Proposed fix: view from the CUT-AWAY side so the section face is
        // front-most. This is what cut_view_camera_focused must do after the fix.
        let cutaway_cam = side_camera(&plane, focus, half_extent, -1.0);
        let cutaway = pollster::block_on(offscreen.render_prepared_scene_with_clip(
            &scene,
            &cutaway_cam,
            &plane,
            spec,
        ))
        .expect("cutaway render");
        save_png(
            &format!("{out_dir}/after_{name}_sectionside.png"),
            &cutaway,
            320,
        );

        // The production function after the fix: cut-away side, section view.
        let prod_cam = cut_view_camera_focused(&plane, focus, half_extent, bbox.half_diagonal());
        let mut prod = pollster::block_on(
            offscreen.render_prepared_scene_with_clip(&scene, &prod_cam, &plane, spec),
        )
        .expect("prod render");
        save_png(&format!("{out_dir}/prod_{name}_fixed.png"), &prod, 320);

        // Ruler overlay: two section-plane points EXACTLY 7.30 mm apart, mapped
        // to panel pixels with the same basis the slice was rendered with, then
        // drawn as markers + connecting line. Proves the ruler tracks geometry.
        let normal = Vec3::from_array(plane.normal).normalize();
        let (right, up) = slice_view_basis(normal);
        // 7.30 mm = |(4.38, 5.84)| along the in-plane (right, up) basis.
        let a = focus - right * 2.19 - up * 2.92;
        let b = focus + right * 2.19 + up * 2.92;
        let pa = world_to_panel(focus, right, up, f64::from(half_extent), a, 320);
        let pb = world_to_panel(focus, right, up, f64::from(half_extent), b, 320);
        draw_line(&mut prod, 320, pa, pb, [66, 117, 204, 255]);
        draw_disc(&mut prod, 320, pa, 4, [66, 117, 204, 255]);
        draw_disc(&mut prod, 320, pb, 4, [66, 117, 204, 255]);
        save_png(&format!("{out_dir}/ruler_{name}_7p30mm.png"), &prod, 320);
    }
    eprintln!("cutview-polish PNGs written to {out_dir}");
}

/// Zoom-at-cursor sequence: render the SAME section framed disc-wide, then
/// magnified 1.8x and 3.3x anchored at a fixed off-center pixel. A crosshair
/// marks that pixel in every frame — the section feature under it stays put
/// while everything scales around it (item: in-panel zoom-to-cursor). Mirrors
/// `cut_ruler::SlicePlaneMap::zoom_focus_at_cursor` exactly.
#[test]
#[ignore = "writes PNGs to the scratchpad for manual inspection"]
fn zoom_at_cursor_sequence_dump() {
    let out_dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/cutview-r3"
    );
    std::fs::create_dir_all(out_dir).expect("create dump dir");
    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let (scene, bbox) = two_jaw(&offscreen);

    let size: u16 = 340;
    let normal = [1.0_f32, 0.0, 0.2];
    let plane = ClipPlane::new(normal, focus_dist(Vec3::new(0.4, 0.0, 2.0), normal));
    let base_focus = Vec3::new(0.4, 0.0, 2.0);
    let base_half = 1.6_f32;
    let n = Vec3::from_array(normal).normalize();
    let (right, up) = slice_view_basis(n);

    // Anchor pixel: upper-right of the panel, over a tooth section.
    let cursor = (i32::from(size) * 66 / 100, i32::from(size) * 34 / 100);
    let spec = ThumbnailSpec {
        size_px: size,
        background: BG,
    };

    for (name, half_ratio) in [
        ("z1_disc", 1.0_f32),
        ("z2_1p8x", 1.0 / 1.8),
        ("z3_3p3x", 1.0 / 3.3),
    ] {
        // Zoom-to-cursor math (identical to the app's SlicePlaneMap helper).
        let ndc_x = f64::from(cursor.0) / f64::from(size) * 2.0 - 1.0;
        let ndc_y = 1.0 - f64::from(cursor.1) / f64::from(size) * 2.0;
        let dir = right * (ndc_x as f32) + up * (ndc_y as f32);
        let world_cursor = base_focus + dir * base_half;
        let new_half = base_half * half_ratio;
        let new_focus = world_cursor - dir * new_half;

        let cam = cut_view_camera_focused(&plane, new_focus, new_half, bbox.half_diagonal());
        let mut px = pollster::block_on(
            offscreen.render_prepared_scene_with_clip(&scene, &cam, &plane, spec),
        )
        .expect("zoom render");
        // Crosshair at the fixed anchor pixel.
        for d in -8..=8 {
            put_px(&mut px, size, cursor.0 + d, cursor.1, [214, 64, 64, 255]);
            put_px(&mut px, size, cursor.0, cursor.1 + d, [214, 64, 64, 255]);
        }
        save_png(&format!("{out_dir}/zoom_{name}.png"), &px, size);
    }
    eprintln!("zoom-at-cursor PNGs written to {out_dir}");
}

fn focus_dist(focus: Vec3, n: [f32; 3]) -> f32 {
    let n = Vec3::from_array(n).normalize();
    n.dot(focus)
}

/// World section-plane point -> panel pixel (matches `cut_ruler::SlicePlaneMap`).
fn world_to_panel(
    focus: Vec3,
    right: Vec3,
    up: Vec3,
    half_extent: f64,
    world: Vec3,
    size: u16,
) -> (i32, i32) {
    let d = world - focus;
    let ndc_x = f64::from(right.dot(d)) / half_extent;
    let ndc_y = f64::from(up.dot(d)) / half_extent;
    let w = f64::from(size);
    let px = (ndc_x * 0.5 + 0.5) * w;
    let py = (0.5 - ndc_y * 0.5) * w;
    (px as i32, py as i32)
}

fn put_px(buf: &mut [u8], size: u16, x: i32, y: i32, rgba: [u8; 4]) {
    let s = i32::from(size);
    if x < 0 || y < 0 || x >= s || y >= s {
        return;
    }
    let i = ((y * s + x) * 4) as usize;
    buf[i..i + 4].copy_from_slice(&rgba);
}

fn draw_disc(buf: &mut [u8], size: u16, c: (i32, i32), r: i32, rgba: [u8; 4]) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                put_px(buf, size, c.0 + dx, c.1 + dy, rgba);
            }
        }
    }
}

fn draw_line(buf: &mut [u8], size: u16, a: (i32, i32), b: (i32, i32), rgba: [u8; 4]) {
    let steps = (a.0 - b.0).abs().max((a.1 - b.1).abs()).max(1);
    for k in 0..=steps {
        let t = f64::from(k) / f64::from(steps);
        let x = f64::from(a.0) + t * f64::from(b.0 - a.0);
        let y = f64::from(a.1) + t * f64::from(b.1 - a.1);
        put_px(buf, size, x as i32, y as i32, rgba);
    }
}

fn save_png(path: &str, pixels: &[u8], size: u16) {
    let img = image::RgbaImage::from_raw(u32::from(size), u32::from(size), pixels.to_vec())
        .expect("rgba buffer matches dimensions");
    img.save(path).expect("write png");
}

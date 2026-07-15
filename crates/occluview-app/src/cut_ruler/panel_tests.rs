#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::many_single_char_names,
    clippy::too_many_arguments
)]

use super::*;
use crate::measure_draw;
use occluview_render::slice_view_basis;

fn proof_viewport() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 820.0))
}

fn image_rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(120.0, 340.0), egui::vec2(300.0, 300.0))
}

fn flat_cam() -> SliceCam {
    SliceCam {
        focus: Vec3::ZERO,
        normal: Vec3::Z,
        half_extent: 12.0,
    }
}

fn press(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    }
}

fn release(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    }
}

/// Run one Section-panel frame with a raw-input event list, returning the
/// panel outcome. The ruler carries state across frames.
fn run_panel_frame(
    ctx: &egui::Context,
    vp: egui::Rect,
    events: Vec<egui::Event>,
    ruler: &mut CutRuler,
) -> SectionPanelOut {
    let raw = egui::RawInput {
        screen_rect: Some(vp),
        events,
        ..Default::default()
    };
    let mut captured = None;
    let _full = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let render = SectionRender {
                mode: SectionDisplay::Lines,
                measure_mode: SliceMeasureMode::Distance,
                magnet: false, // raw placement, so we can count anchors exactly
                texture: None,
                section: None,
                color_for: |_id: SceneMeshId| ui_theme::TEXT,
            };
            captured = Some(show_section_panel(ui, vp, flat_cam(), ruler, render));
        });
    });
    captured.expect("panel ran")
}

#[test]
fn section_header_close_is_a_full_size_shared_control() {
    let ctx = egui::Context::default();
    let vp = proof_viewport();
    let panel = section_panel_rect(vp).expect("panel fits");
    let close = egui::pos2(
        panel.right() - PANEL_PAD_PX - 12.0,
        panel.top() + PANEL_PAD_PX + 10.0,
    );
    let mut ruler = CutRuler::default();
    run_panel_frame(&ctx, vp, vec![egui::Event::PointerMoved(close)], &mut ruler);
    run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(close), press(close)],
        &mut ruler,
    );
    let outcome = run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(close), release(close)],
        &mut ruler,
    );

    assert_eq!(outcome.command, SectionPanelCommand::Close);
    assert!(outcome.consumed);
}

#[test]
fn panel_click_with_small_jitter_places_a_point() {
    let ctx = egui::Context::default();
    let vp = proof_viewport();
    let p = section_image_rect_for(vp).unwrap().center();
    let mut ruler = CutRuler::default();
    // Warm-up frame: egui 0.29 hit-tests pointer events against the PREVIOUS
    // frame's widget rects, so the ruler widget must exist before the press.
    run_panel_frame(&ctx, vp, vec![egui::Event::PointerMoved(p)], &mut ruler);
    run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(p), press(p)],
        &mut ruler,
    );
    // Release 2 px away (below egui's drag threshold) => a click.
    let jitter = p + egui::vec2(2.0, 0.0);
    run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(jitter), release(jitter)],
        &mut ruler,
    );
    assert_eq!(
        ruler.anchors().len(),
        1,
        "a 2 px-jitter click must still place a measurement point"
    );
}

#[test]
fn panel_drag_pans_and_places_nothing() {
    let ctx = egui::Context::default();
    let vp = proof_viewport();
    let p = section_image_rect_for(vp).unwrap().center();
    let mut ruler = CutRuler::default();
    // Warm-up frame so the ruler widget is registered before the press.
    run_panel_frame(&ctx, vp, vec![egui::Event::PointerMoved(p)], &mut ruler);
    run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(p), press(p)],
        &mut ruler,
    );
    // Move 40 px (well past the drag threshold): this is a pan.
    let dragged = p + egui::vec2(40.0, 0.0);
    let out = run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(dragged)],
        &mut ruler,
    );
    assert!(out.panned, "a past-threshold drag must pan the section");
    // Release far away — even OUTSIDE the panel: still no point placed.
    let outside = egui::pos2(20.0, 20.0);
    run_panel_frame(
        &ctx,
        vp,
        vec![egui::Event::PointerMoved(outside), release(outside)],
        &mut ruler,
    );
    assert_eq!(
        ruler.anchors().len(),
        0,
        "a drag (released even outside the panel) must place nothing"
    );
}

// ---- render proof ------------------------------------------------------

/// A real hexagonal cross-section: a centered cube cut by a tilted plane,
/// computed by the production kernel, plus a cam framing it in the panel.
fn proof_section() -> (SceneSection, SliceCam) {
    use occluview_core::scene::{SectionPlane, VisibilityFilter};
    use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
    let s = 8.0_f32;
    let corner = |x: f32, y: f32, z: f32| Vertex::at(Vec3::new(x * s, y * s, z * s));
    let vertices = vec![
        corner(-1.0, -1.0, -1.0),
        corner(1.0, -1.0, -1.0),
        corner(1.0, 1.0, -1.0),
        corner(-1.0, 1.0, -1.0),
        corner(-1.0, -1.0, 1.0),
        corner(1.0, -1.0, 1.0),
        corner(1.0, 1.0, 1.0),
        corner(-1.0, 1.0, 1.0),
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 5, 1, 0, 4, 5, 3, 2, 6, 3, 6, 7, 0, 3, 7, 0, 7, 4,
        1, 5, 6, 1, 6, 2,
    ];
    let mesh = Mesh::new(Some("proof-cube".into()), vertices, indices).expect("cube");
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    let normal = Vec3::new(0.5, 0.72, 0.48).normalize();
    let plane = SectionPlane::new(normal, 0.0).expect("plane");
    let section = SceneSection::compute(&scene, plane, &VisibilityFilter::SceneVisibility);
    let cam = SliceCam {
        focus: Vec3::ZERO,
        normal,
        half_extent: 13.0,
    };
    (section, cam)
}

/// A synthetic shaded slice image that fills the section contour, so `Mesh`
/// mode reads like the real offscreen render (which needs a GPU).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn synthetic_slice_image(section: &SceneSection, cam: SliceCam, size: usize) -> egui::ColorImage {
    let (right, up) = slice_view_basis(cam.normal);
    let half = cam.half_extent.max(0.1);
    // Contour in plane (right, up) coordinates.
    let mut loops: Vec<Vec<(f32, f32)>> = Vec::new();
    for layer in &section.per_layer {
        for polyline in &layer.polylines {
            loops.push(
                polyline
                    .points
                    .iter()
                    .map(|p| {
                        let d = p.as_vec3() - cam.focus;
                        (right.dot(d), up.dot(d))
                    })
                    .collect(),
            );
        }
    }
    let mut pixels = vec![egui::Color32::from_rgb(232, 235, 238); size * size];
    for ty in 0..size {
        for tx in 0..size {
            let a = ((tx as f32 + 0.5) / size as f32 * 2.0 - 1.0) * half;
            let b = (1.0 - (ty as f32 + 0.5) / size as f32 * 2.0) * half;
            if loops.iter().any(|poly| point_in_polygon(a, b, poly)) {
                // Soft radial shading toward the section center.
                let r = ((a * a + b * b).sqrt() / half).clamp(0.0, 1.0);
                let shade = (220.0 - 40.0 * r) as u8;
                pixels[ty * size + tx] = egui::Color32::from_rgb(shade, shade - 20, shade - 46);
            }
        }
    }
    egui::ColorImage {
        size: [size, size],
        pixels,
    }
}

fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) {
            let t = (y - yi) / (yj - yi);
            if x < xi + t * (xj - xi) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Software-rasterize the real egui output of one frame into an RGBA buffer of
/// `panel_rect` size, sampling the font atlas and any color textures the frame
/// uploaded — the honest "rasterize the real egui output" proof.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
fn rasterize_panel(ctx: &egui::Context, full: &egui::FullOutput, panel: egui::Rect) -> Vec<u8> {
    use egui::epaint::{ImageData, Primitive};
    use std::collections::HashMap;
    let w = panel.width() as usize;
    let h = panel.height() as usize;
    let mut font: Option<(usize, usize, Vec<f32>)> = None;
    let mut colors: HashMap<egui::TextureId, (usize, usize, Vec<egui::Color32>)> = HashMap::new();
    for (id, delta) in &full.textures_delta.set {
        match &delta.image {
            ImageData::Font(f) => font = Some((f.size[0], f.size[1], f.pixels.clone())),
            ImageData::Color(c) => {
                colors.insert(*id, (c.size[0], c.size[1], c.pixels.clone()));
            }
        }
    }
    let mut buf = vec![0u8; w * h * 4];
    for px in buf.chunks_exact_mut(4) {
        px.copy_from_slice(&[226, 230, 234, 255]);
    }
    let ppp = full.pixels_per_point;
    let sample = |tex: egui::TextureId, u: f32, v: f32| -> (f32, f32, f32, f32) {
        if let Some((cw, ch, pix)) = colors.get(&tex) {
            let cx = ((u * *cw as f32) as usize).min(cw.saturating_sub(1));
            let cy = ((v * *ch as f32) as usize).min(ch.saturating_sub(1));
            let texel = pix[cy * cw + cx];
            (
                f32::from(texel.r()) / 255.0,
                f32::from(texel.g()) / 255.0,
                f32::from(texel.b()) / 255.0,
                f32::from(texel.a()) / 255.0,
            )
        } else if let Some((fw, fh, pix)) = &font {
            let fx = ((u * *fw as f32) as usize).min(fw.saturating_sub(1));
            let fy = ((v * *fh as f32) as usize).min(fh.saturating_sub(1));
            let cov = pix[fy * fw + fx].clamp(0.0, 1.0);
            (cov, cov, cov, cov)
        } else {
            (1.0, 1.0, 1.0, 1.0)
        }
    };
    for prim in ctx.tessellate(full.shapes.clone(), ppp) {
        let Primitive::Mesh(mesh) = prim.primitive else {
            continue;
        };
        for tri in mesh.indices.chunks_exact(3) {
            let vtx = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            let pt: Vec<egui::Pos2> = vtx
                .iter()
                .map(|v| {
                    egui::pos2(
                        (v.pos.x * ppp) - panel.left(),
                        (v.pos.y * ppp) - panel.top(),
                    )
                })
                .collect();
            let (min_x, max_x) = (
                pt.iter()
                    .map(|p| p.x)
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(0.0) as usize,
                (pt.iter().map(|p| p.x).fold(f32::MIN, f32::max).ceil() as usize).min(w),
            );
            let (min_y, max_y) = (
                pt.iter()
                    .map(|p| p.y)
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(0.0) as usize,
                (pt.iter().map(|p| p.y).fold(f32::MIN, f32::max).ceil() as usize).min(h),
            );
            let area = edge(pt[0], pt[1], pt[2]);
            if area.abs() < 1e-6 {
                continue;
            }
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let c = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = edge(pt[1], pt[2], c);
                    let w1 = edge(pt[2], pt[0], c);
                    let w2 = edge(pt[0], pt[1], c);
                    let inside = (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0)
                        || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0);
                    if !inside {
                        continue;
                    }
                    let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
                    let chan = |sel: fn(egui::Color32) -> u8| {
                        l0 * f32::from(sel(vtx[0].color))
                            + l1 * f32::from(sel(vtx[1].color))
                            + l2 * f32::from(sel(vtx[2].color))
                    };
                    let col = [
                        chan(|c| c.r()),
                        chan(|c| c.g()),
                        chan(|c| c.b()),
                        chan(|c| c.a()),
                    ];
                    let u = l0 * vtx[0].uv.x + l1 * vtx[1].uv.x + l2 * vtx[2].uv.x;
                    let v = l0 * vtx[0].uv.y + l1 * vtx[1].uv.y + l2 * vtx[2].uv.y;
                    let (mr, mg, mb, ma) = sample(mesh.texture_id, u, v);
                    // Premultiplied `vertex_color * texel`, blended over dst.
                    let fr = col[0] / 255.0 * mr;
                    let fg = col[1] / 255.0 * mg;
                    let fb = col[2] / 255.0 * mb;
                    let fa = col[3] / 255.0 * ma;
                    let idx = (y * w + x) * 4;
                    for (k, fk) in [fr, fg, fb].into_iter().enumerate() {
                        let dst = f32::from(buf[idx + k]);
                        buf[idx + k] = (fk * 255.0 + dst * (1.0 - fa)).clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    buf
}

fn edge(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// A ready-made in-slice measurement to seed into the proof ruler.
enum ProofMeasure {
    /// Two distance anchors (world section-plane points).
    Distance(Vec<Vec3>),
    /// A wall-thickness reading (entry, exit, mm) drawn as the shared ray.
    Thickness(Vec3, Vec3, f32),
}

/// Render one Section-panel scenario to a PNG and return its path.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn render_proof_png(
    dir: &str,
    name: &str,
    cam: SliceCam,
    section: &SceneSection,
    mode: SectionDisplay,
    texture_image: Option<egui::ColorImage>,
    measure: &ProofMeasure,
) -> String {
    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::light());
    let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 820.0));
    let mut ruler = CutRuler::default();
    let measure_mode = match measure {
        ProofMeasure::Distance(anchors) => {
            for a in anchors {
                ruler.place(*a, cam);
            }
            SliceMeasureMode::Distance
        }
        ProofMeasure::Thickness(entry, exit, mm) => {
            ruler.set_thickness(*entry, *exit, *mm, cam);
            SliceMeasureMode::Thickness
        }
    };
    let full = ctx.run(
        egui::RawInput {
            screen_rect: Some(vp),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let texture = texture_image.clone().map(|img| {
                    ui.ctx()
                        .load_texture("proof-slice", img, egui::TextureOptions::LINEAR)
                });
                let render = SectionRender {
                    mode,
                    measure_mode,
                    magnet: true,
                    texture: texture.as_ref(),
                    section: Some(section),
                    color_for: |_id: SceneMeshId| ui_theme::ACCENT,
                };
                let _ = show_section_panel(ui, vp, cam, &mut ruler, render);
            });
        },
    );
    let panel = section_panel_rect(vp).expect("panel fits");
    let buf = rasterize_panel(&ctx, &full, panel);
    let (w, h) = (panel.width() as u32, panel.height() as u32);
    let path = format!("{dir}/{name}.png");
    image::RgbaImage::from_raw(w, h, buf)
        .expect("raster buffer")
        .save(&path)
        .expect("save png");
    path
}

#[test]
#[ignore = "writes Section-panel render-proof PNGs to the scratchpad"]
#[allow(clippy::print_stderr)]
fn section_panel_render_proof() {
    let dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/section-panel-proof"
    );
    std::fs::create_dir_all(dir).expect("scratchpad dir");
    let (section, cam) = proof_section();
    let (right, up) = slice_view_basis(cam.normal);
    // Two ruler anchors on the section plane, a known distance apart.
    let anchors = [
        cam.focus - right * 4.0 + up * 2.0,
        cam.focus + right * 5.0 - up * 3.0,
    ];

    let distance = ProofMeasure::Distance(anchors.to_vec());
    let a = render_proof_png(
        dir,
        "lines",
        cam,
        &section,
        SectionDisplay::Lines,
        None,
        &distance,
    );
    // Panned: shove the focus in-plane so a different part is centered.
    let panned = SliceCam {
        focus: cam.focus + right * 5.0 + up * 3.0,
        ..cam
    };
    let b = render_proof_png(
        dir,
        "lines_panned",
        panned,
        &section,
        SectionDisplay::Lines,
        None,
        &distance,
    );
    let slice = synthetic_slice_image(&section, cam, 256);
    let c = render_proof_png(
        dir,
        "mesh",
        cam,
        &section,
        SectionDisplay::Mesh,
        Some(slice),
        &distance,
    );
    // Hostile: an empty section (plane misses the mesh) must show the honest
    // empty state in Lines mode, never a stale picture.
    let empty = SceneSection::default();
    let d = render_proof_png(
        dir,
        "lines_empty",
        cam,
        &empty,
        SectionDisplay::Lines,
        None,
        &ProofMeasure::Distance(Vec::new()),
    );
    // Feature E: a one-click in-slice wall-thickness probe. Cast the ray from
    // a real contour point so the shared ray+chip reads exactly like the
    // main-viewport probe, edge-on inside the section.
    let segments = section_segments(&section);
    let click_world = cam.focus - up * 12.0; // below the hexagon: snaps to its lower edge
    let thickness = probe_section::slice_wall_thickness(click_world, cam.normal, &segments)
        .expect("in-slice wall probe finds an opposite edge");
    let e = render_proof_png(
        dir,
        "thickness_probe",
        cam,
        &section,
        SectionDisplay::Lines,
        None,
        &ProofMeasure::Thickness(thickness.entry, thickness.exit, thickness.thickness_mm),
    );
    eprintln!("section-panel render proof written:\n  {a}\n  {b}\n  {c}\n  {d}\n  {e}");
}

#[test]
fn one_click_thickness_places_a_wall_reading_and_a_second_mode_switch_is_clean() {
    // The panel's Thickness mode places a one-click wall reading from the
    // contour (feature E); switching back to Distance and placing two points
    // replaces it with a distance, and each mode's clear is honest.
    let (section, cam) = proof_section();
    let (_right, up) = slice_view_basis(cam.normal);
    let map = SlicePlaneMap::new(cam, image_rect());
    let mut ruler = CutRuler::default();

    // A thickness click below the hexagon snaps to its lower edge and reads
    // the wall across to the opposite edge.
    let click = map.world_to_panel(cam.focus - up * 12.0);
    place_measurement(
        click,
        &map,
        &mut ruler,
        &RulerPlacement {
            cam,
            measure_mode: SliceMeasureMode::Thickness,
            magnet: true,
            section: Some(&section),
        },
    );
    assert!(
        ruler.thickness_reading_mm().is_some(),
        "a contour thickness click must place a wall reading"
    );
    assert!(
        ruler.anchors().is_empty(),
        "thickness is not a distance point"
    );

    // Switching to Distance and placing two points replaces the thickness.
    for pos in [egui::pos2(200.0, 420.0), egui::pos2(360.0, 520.0)] {
        place_measurement(
            pos,
            &map,
            &mut ruler,
            &RulerPlacement {
                cam,
                measure_mode: SliceMeasureMode::Distance,
                magnet: false,
                section: Some(&section),
            },
        );
    }
    assert!(
        ruler.thickness_reading_mm().is_none(),
        "distance replaced it"
    );
    assert_eq!(ruler.anchors().len(), 2);
}

#[test]
fn thickness_click_off_the_contour_is_an_honest_no_op() {
    // A thickness click with no section, or far from any edge that has no
    // opposite wall, places nothing (honest).
    let (section, cam) = proof_section();
    let map = SlicePlaneMap::new(cam, image_rect());
    let mut ruler = CutRuler::default();
    // Empty section: nothing to probe.
    place_measurement(
        map.world_to_panel(cam.focus),
        &map,
        &mut ruler,
        &RulerPlacement {
            cam,
            measure_mode: SliceMeasureMode::Thickness,
            magnet: true,
            section: Some(&SceneSection::default()),
        },
    );
    assert!(ruler.thickness_reading_mm().is_none());
    assert!(ruler.anchors().is_empty());
    // A real section but the click is on the contour: it DOES read (guards
    // the test above against being vacuous).
    let (_r, up) = slice_view_basis(cam.normal);
    place_measurement(
        map.world_to_panel(cam.focus - up * 12.0),
        &map,
        &mut ruler,
        &RulerPlacement {
            cam,
            measure_mode: SliceMeasureMode::Thickness,
            magnet: true,
            section: Some(&section),
        },
    );
    assert!(ruler.thickness_reading_mm().is_some());
}

/// Feature D render proof: ONE frame showing the main-viewport thickness
/// marker (the shared "ray" over the model stand-in) AND the auto-opened
/// Section panel presenting the SAME wall edge-on with its chord+mm — so the
/// two views can be eyeballed side by side. Rasterizes the real egui output.
#[test]
#[ignore = "writes the probe-driven cut-view render proof to the scratchpad"]
#[allow(
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn probe_cut_view_render_proof() {
    let dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/section-panel-proof"
    );
    std::fs::create_dir_all(dir).expect("scratchpad dir");
    let ctx = egui::Context::default();
    ctx.set_visuals(egui::Visuals::light());
    let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 820.0));
    let (section, cam) = proof_section();
    let (_right, up) = slice_view_basis(cam.normal);
    // The SAME wall the section shows: probe the contour so the panel chord
    // and the main marker carry one identical measurement.
    let segments = section_segments(&section);
    let probe = probe_section::slice_wall_thickness(cam.focus - up * 12.0, cam.normal, &segments)
        .expect("in-slice wall probe");
    let mut ruler = CutRuler::default();
    ruler.set_thickness(probe.entry, probe.exit, probe.thickness_mm, cam);

    let full = ctx.run(
        egui::RawInput {
            screen_rect: Some(vp),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Model stand-in for the main 3D viewport.
                ui.painter()
                    .rect_filled(vp, 0.0, egui::Color32::from_rgb(226, 230, 234));
                // Main-viewport thickness marker: the EXACT shared ray the
                // measure overlay paints (entry dot, chord, exit dot, mm chip).
                let entry = egui::pos2(320.0, 250.0);
                let exit = egui::pos2(320.0, 250.0 + 90.0);
                measure_draw::thickness_ray(
                    ui.painter(),
                    entry,
                    exit,
                    &format!("{:.2} mm", probe.thickness_mm),
                );
                // The auto-opened Section panel with the same wall edge-on.
                let render = SectionRender {
                    mode: SectionDisplay::Lines,
                    measure_mode: SliceMeasureMode::Thickness,
                    magnet: true,
                    texture: None,
                    section: Some(&section),
                    color_for: |_id: SceneMeshId| ui_theme::ACCENT,
                };
                let _ = show_section_panel(ui, vp, cam, &mut ruler, render);
            });
        },
    );
    let buf = rasterize_panel(&ctx, &full, vp);
    let path = format!("{dir}/probe_cut_view.png");
    image::RgbaImage::from_raw(vp.width() as u32, vp.height() as u32, buf)
        .expect("raster buffer")
        .save(&path)
        .expect("save png");
    eprintln!("probe-driven cut-view render proof written:\n  {path}");
}

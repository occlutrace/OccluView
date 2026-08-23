#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::many_single_char_names,
    clippy::too_many_arguments
)]

use super::*;
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

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::many_single_char_names,
    clippy::too_many_arguments
)]
use super::*;
use occluview_render::slice_view_basis;

/// DIAGNOSTIC: draw a to-scale schematic of the cut-mode viewport chrome —
/// the docked Section panel (bottom-right, with the real slice composited
/// into its image area), the lifted axis gizmo above it, the bottom-left
/// scale bar, and the top-right cut strip — so the new layout can be eyeballed.
/// `cargo test -p occluview-app --bins cut_ruler::tests::layout_schematic_dump -- --ignored`
#[test]
#[ignore = "writes a layout schematic PNG to the scratchpad"]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::print_stderr,
    clippy::too_many_lines
)]
fn layout_schematic_dump() {
    let out_dir = concat!(
        "/tmp/claude-1101/-home-wow-occlutraceio/",
        "4e21c36a-f8d7-487e-89e0-33dc0df28bdb/scratchpad/cutview-r3"
    );
    std::fs::create_dir_all(out_dir).unwrap();
    let (w, h) = (1600u32, 900u32);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let put = |buf: &mut [u8], x: i32, y: i32, c: [u8; 4]| {
        if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&c);
        }
    };
    let fill = |buf: &mut [u8], r: egui::Rect, c: [u8; 4]| {
        for y in r.top() as i32..r.bottom() as i32 {
            for x in r.left() as i32..r.right() as i32 {
                put(buf, x, y, c);
            }
        }
    };
    let stroke = |buf: &mut [u8], r: egui::Rect, c: [u8; 4]| {
        for x in r.left() as i32..r.right() as i32 {
            put(buf, x, r.top() as i32, c);
            put(buf, x, r.bottom() as i32 - 1, c);
        }
        for y in r.top() as i32..r.bottom() as i32 {
            put(buf, r.left() as i32, y, c);
            put(buf, r.right() as i32 - 1, y, c);
        }
    };
    // Viewport background (app grey).
    fill(
        &mut buf,
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w as f32, h as f32)),
        [226, 230, 234, 255],
    );
    let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w as f32, h as f32));

    // Docked Section panel (bottom-right) + header + real slice composite.
    let panel = section_panel_rect(vp).unwrap();
    fill(&mut buf, panel, [243, 245, 248, 255]);
    stroke(&mut buf, panel, [150, 160, 172, 255]);
    let header = egui::Rect::from_min_max(
        panel.min,
        egui::pos2(panel.right(), panel.top() + PANEL_PAD_PX + PANEL_HEADER_PX),
    );
    stroke(&mut buf, header, [150, 160, 172, 255]);
    let image = section_image_rect(panel);
    if let Ok(slice) = image::open(format!("{out_dir}/zoom_z2_1p8x.png")) {
        let slice = slice.to_rgba8();
        for y in 0..image.height() as u32 {
            for x in 0..image.width() as u32 {
                let sx = x * slice.width() / image.width() as u32;
                let sy = y * slice.height() / image.height() as u32;
                let p = slice
                    .get_pixel(sx.min(slice.width() - 1), sy.min(slice.height() - 1))
                    .0;
                put(
                    &mut buf,
                    image.left() as i32 + x as i32,
                    image.top() as i32 + y as i32,
                    p,
                );
            }
        }
    } else {
        fill(&mut buf, image, [210, 210, 216, 255]);
    }
    stroke(&mut buf, image, [120, 130, 142, 255]);

    // Lifted axis gizmo above the panel (same formula as axis_gizmo.rs).
    let gizmo_c = egui::pos2(vp.right() - 16.0 - 24.0, panel.top() - 16.0 - 34.0);
    for dy in -34..=34 {
        for dx in -34..=34 {
            if dx * dx + dy * dy <= 34 * 34 {
                put(
                    &mut buf,
                    gizmo_c.x as i32 + dx,
                    gizmo_c.y as i32 + dy,
                    [248, 250, 252, 255],
                );
            }
        }
    }
    for (col, off) in [
        ([224, 92, 92, 255], (24.0, 0.0)),
        ([112, 198, 120, 255], (0.0, -24.0)),
        ([96, 150, 234, 255], (-16.0, 12.0)),
    ] {
        fill(
            &mut buf,
            egui::Rect::from_center_size(
                egui::pos2(gizmo_c.x + off.0, gizmo_c.y + off.1),
                egui::vec2(16.0, 16.0),
            ),
            col,
        );
    }

    // Bottom-left scale bar + status placeholder.
    for x in 16..216 {
        put(&mut buf, x, h as i32 - 16, [15, 23, 42, 255]);
    }
    stroke(
        &mut buf,
        egui::Rect::from_min_size(egui::pos2(14.0, h as f32 - 48.0), egui::vec2(430.0, 34.0)),
        [120, 130, 142, 255],
    );

    // Top-right cut strip.
    stroke(
        &mut buf,
        egui::Rect::from_min_size(
            egui::pos2(vp.right() - 14.0 - 232.0, 52.0),
            egui::vec2(232.0, 74.0),
        ),
        [120, 130, 142, 255],
    );

    image::RgbaImage::from_raw(w, h, buf)
        .unwrap()
        .save(format!("{out_dir}/layout_schematic.png"))
        .unwrap();
    eprintln!("layout schematic written to {out_dir}/layout_schematic.png");
}

fn cam(normal: Vec3, half_extent: f32) -> SliceCam {
    SliceCam {
        focus: Vec3::new(3.0, -2.0, 7.0),
        normal: normal.normalize(),
        half_extent,
    }
}

fn image_rect() -> egui::Rect {
    // Non-square origin offset to catch coordinate-frame mistakes.
    egui::Rect::from_min_size(egui::pos2(120.0, 340.0), egui::vec2(300.0, 300.0))
}

#[test]
fn section_panel_docks_bottom_right_clear_of_left_chrome() {
    let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
    let panel = section_panel_rect(vp).unwrap();
    // Bottom-RIGHT: hugs the right and bottom edges within the margins.
    assert!(
        (vp.right() - panel.right() - PANEL_MARGIN_PX).abs() < 0.5,
        "panel right edge should sit one margin from the viewport right: {panel:?}"
    );
    assert!(
        (vp.bottom() - panel.bottom() - PANEL_BOTTOM_GAP_PX).abs() < 0.5,
        "panel bottom edge should sit one gap from the viewport bottom: {panel:?}"
    );
    // Its left edge is well past center, so the bottom-LEFT scale bar/status
    // chrome is never covered.
    assert!(
        panel.left() > vp.center().x,
        "panel must stay in the right half, clear of the bottom-left chrome: {panel:?}"
    );
    // The square image sub-rect is the ~340 px the owner asked for.
    let image = section_image_rect(panel);
    assert!(
        (image.width() - image.height()).abs() < 0.5,
        "image must be square"
    );
    assert!(image.width() > 320.0, "image should be the bigger ~340 px");
}

#[test]
fn section_panel_never_collides_with_chrome_across_window_sizes() {
    // Adversarial sweep (144 violations in the pre-fix layout): wherever
    // the panel decides to show, it must coexist with the lifted axis gizmo
    // and the bottom-left status pill — at EVERY window
    // size. Where it cannot, it hides instead of painting over chrome.
    let mut shown = 0usize;
    for w in (320..=2000).step_by(20) {
        for h in (240..=1200).step_by(20) {
            #[allow(clippy::cast_precision_loss)]
            let vp =
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w as f32, h as f32));
            let Some(panel) = section_panel_rect(vp) else {
                continue;
            };
            shown += 1;
            assert!(vp.contains_rect(panel), "{w}x{h}: panel leaves viewport");
            let pill = crate::app_chrome::status_overlay_rect(vp);
            assert!(
                !panel.intersects(pill),
                "{w}x{h}: panel covers the status pill"
            );
            let gizmo = crate::viewer::axis_gizmo::axis_gizmo_footprint(vp, Some(panel));
            assert!(
                vp.contains_rect(gizmo),
                "{w}x{h}: lifted gizmo leaves the viewport"
            );
            assert!(
                !gizmo.intersects(panel),
                "{w}x{h}: lifted gizmo overlaps the panel"
            );
            let image = section_image_rect(panel);
            assert!(
                image.width() >= MIN_IMAGE_SIDE_PX - 0.5,
                "{w}x{h}: unreadable panel should have hidden instead"
            );
        }
    }
    // The sweep must actually exercise shown panels (big windows exist).
    assert!(shown > 100, "sweep too weak: only {shown} shown panels");
}

#[test]
fn panel_world_round_trips_to_the_same_pixel() {
    let map = SlicePlaneMap::new(cam(Vec3::X, 8.0), image_rect());
    for pos in [
        egui::pos2(120.0, 340.0),
        egui::pos2(270.0, 490.0),
        egui::pos2(400.0, 620.0),
        egui::pos2(180.0, 610.0),
    ] {
        let world = map.panel_to_world(pos);
        let back = map.world_to_panel(world);
        assert!(
            (back - pos).length() < 1e-2,
            "round-trip drifted: {pos:?} -> {back:?}"
        );
    }
}

#[test]
fn pixel_span_measures_the_exact_ground_truth_mm() {
    // The ortho half-extent fixes mm-per-pixel: 300 px image, half_extent
    // 8 mm => 16 mm across 300 px => 16/300 mm/px. A 90x120 px right triangle
    // spans 150 px => exactly 150 * 16/300 = 8.00 mm, independent of the
    // (orthonormal) plane basis. Integer legs avoid f32 pixel quantization.
    let c = cam(Vec3::new(1.0, 0.3, -0.2), 8.0);
    let rect = image_rect();
    let map = SlicePlaneMap::new(c, rect);
    let center = rect.center();
    let p0 = egui::pos2(center.x - 45.0, center.y + 60.0);
    let p1 = egui::pos2(center.x + 45.0, center.y - 60.0);
    let mut ruler = CutRuler::default();
    ruler.place(map.panel_to_world(p0), c);
    ruler.place(map.panel_to_world(p1), c);
    let measured = ruler.distance_mm().unwrap();
    assert!(
        (measured - 8.00).abs() < 1e-4,
        "expected 8.00 mm, measured {measured}"
    );
}

#[test]
fn two_points_730_mm_apart_read_back_730() {
    // Ground truth straight from the section-plane basis: two world points
    // placed exactly 7.30 mm apart must measure 7.30 mm and display "7.30".
    let c = cam(Vec3::new(0.6, 0.7, 0.4), 6.0);
    let (right, up) = slice_view_basis(c.normal);
    let a = c.focus + right * 1.5;
    // 7.30 mm along a unit in-plane direction (3-4-5-scaled: 4.38, 5.84).
    let dir = (right * 0.6 + up * 0.8).normalize();
    let b = a + dir * 7.30;
    let mut ruler = CutRuler::default();
    ruler.place(a, c);
    ruler.place(b, c);
    let measured = ruler.distance_mm().unwrap();
    assert!((measured - 7.30).abs() < 1e-4, "measured {measured}");
    assert_eq!(format!("{measured:.2}"), "7.30");
}

#[test]
fn a_pure_zoom_keeps_the_measurement() {
    // Placing at one half-extent then re-syncing at a different half-extent
    // (a wheel zoom) keeps the same plane, so the anchors and the measured
    // distance survive.
    let placed = cam(Vec3::new(1.0, 0.2, 0.0), 8.0);
    let map = SlicePlaneMap::new(placed, image_rect());
    let mut ruler = CutRuler::default();
    ruler.place(map.panel_to_world(egui::pos2(200.0, 420.0)), placed);
    ruler.place(map.panel_to_world(egui::pos2(360.0, 520.0)), placed);
    let before = ruler.distance_mm().unwrap();

    let zoomed = SliceCam {
        half_extent: 3.5,
        ..placed
    };
    ruler.sync_plane(zoomed);
    assert_eq!(ruler.anchors().len(), 2, "zoom must not drop the ruler");
    let after = ruler.distance_mm().unwrap();
    assert!(
        (before - after).abs() < 1e-4,
        "zoom changed the measured mm: {before} -> {after}"
    );
}

#[test]
fn changing_the_plane_clears_the_measurement() {
    let placed = cam(Vec3::X, 8.0);
    let map = SlicePlaneMap::new(placed, image_rect());
    let mut ruler = CutRuler::default();
    ruler.place(map.panel_to_world(egui::pos2(200.0, 420.0)), placed);
    ruler.place(map.panel_to_world(egui::pos2(360.0, 520.0)), placed);
    assert!(ruler.distance_mm().is_some());

    // A tilt (different normal) is a different section -> clear.
    ruler.sync_plane(cam(Vec3::new(1.0, 0.5, 0.0), 8.0));
    assert!(ruler.anchors().is_empty(), "tilt must clear the ruler");

    // Re-place, then a push/pull (same normal, new offset) also clears.
    ruler.place(map.panel_to_world(egui::pos2(210.0, 430.0)), placed);
    let pushed = SliceCam {
        focus: placed.focus + Vec3::X * 4.0,
        ..placed
    };
    ruler.sync_plane(pushed);
    assert!(ruler.anchors().is_empty(), "push/pull must clear the ruler");
}

#[test]
fn zoom_at_cursor_keeps_the_section_point_under_the_pointer_fixed() {
    // Magnify (half_ratio < 1) anchored at an off-center pixel: the section
    // point that WAS under the cursor must map to the SAME pixel after the
    // zoom, and the ruler mm mapping must stay exact under the new framing.
    let c = cam(Vec3::new(1.0, 0.3, -0.2), 8.0);
    let rect = image_rect();
    let map0 = SlicePlaneMap::new(c, rect);
    let cursor = egui::pos2(rect.left() + 210.0, rect.top() + 90.0);
    let world_under_cursor = map0.panel_to_world(cursor);

    for half_ratio in [0.5_f32, 0.8, 1.25, 2.0] {
        let (new_focus, new_half) = SlicePlaneMap::zoom_focus_at_cursor(
            c.focus,
            c.half_extent,
            c.normal,
            rect,
            cursor,
            half_ratio,
        );
        // In-plane move only: the plane offset is unchanged.
        let n = c.normal.normalize();
        assert!(
            (n.dot(new_focus) - n.dot(c.focus)).abs() < 1e-3,
            "zoom moved the focus off the section plane (ratio {half_ratio})"
        );
        let zoomed = SliceCam {
            focus: new_focus,
            half_extent: new_half,
            ..c
        };
        let map1 = SlicePlaneMap::new(zoomed, rect);
        let back = map1.world_to_panel(world_under_cursor);
        assert!(
            (back - cursor).length() < 1e-1,
            "point under cursor drifted for ratio {half_ratio}: {cursor:?} -> {back:?}"
        );
        // The mm mapping is still exact: a full-width span reads 2*new_half mm.
        let left_mid = egui::pos2(rect.left(), rect.center().y);
        let right_mid = egui::pos2(rect.right(), rect.center().y);
        let span = SlicePlaneMap::distance_mm(
            map1.panel_to_world(left_mid),
            map1.panel_to_world(right_mid),
        );
        assert!(
            (span - f64::from(2.0 * new_half)).abs() < 1e-2,
            "mm-per-pixel drifted after zoom (ratio {half_ratio}): span {span}"
        );
    }
}

#[test]
fn a_third_click_starts_a_new_measurement() {
    let c = cam(Vec3::X, 8.0);
    let map = SlicePlaneMap::new(c, image_rect());
    let mut ruler = CutRuler::default();
    ruler.place(map.panel_to_world(egui::pos2(200.0, 420.0)), c);
    ruler.place(map.panel_to_world(egui::pos2(360.0, 520.0)), c);
    ruler.place(map.panel_to_world(egui::pos2(300.0, 400.0)), c);
    assert_eq!(ruler.anchors().len(), 1, "third click restarts the ruler");
    assert!(ruler.distance_mm().is_none());
}

#[test]
fn pan_moves_the_world_point_with_the_cursor_and_stays_in_plane() {
    let c = cam(Vec3::new(1.0, 0.3, -0.2), 8.0);
    let rect = image_rect();
    let map = SlicePlaneMap::new(c, rect);
    let cursor = egui::pos2(rect.left() + 210.0, rect.top() + 90.0);
    let grabbed = map.panel_to_world(cursor);
    for delta in [
        egui::vec2(30.0, -12.0),
        egui::vec2(-50.0, 40.0),
        egui::vec2(0.0, 0.0),
    ] {
        // `pointer` is where the cursor IS now (after moving by `delta`).
        let pan = map.pan_delta_for_drag(cursor + delta, delta);
        // In-plane only: the plane offset (normal · focus) is unchanged.
        let n = c.normal.normalize();
        assert!(n.dot(pan).abs() < 1e-4, "pan left the section plane: {pan}");
        let panned = SliceCam {
            focus: c.focus + pan,
            ..c
        };
        let map2 = SlicePlaneMap::new(panned, rect);
        let back = map2.world_to_panel(grabbed);
        assert!(
            (back - (cursor + delta)).length() < 1e-1,
            "grabbed point drifted off the cursor for delta {delta:?}: {back:?}"
        );
    }
}

#[test]
fn pan_leaves_the_measured_distance_unchanged() {
    let c = cam(Vec3::new(0.4, 0.7, 0.5), 9.0);
    let rect = image_rect();
    let map = SlicePlaneMap::new(c, rect);
    let mut ruler = CutRuler::default();
    ruler.place(
        map.panel_to_world(egui::pos2(rect.left() + 80.0, rect.top() + 100.0)),
        c,
    );
    ruler.place(
        map.panel_to_world(egui::pos2(rect.left() + 220.0, rect.top() + 190.0)),
        c,
    );
    let before = ruler.distance_mm().unwrap();

    let pan = map.pan_delta_for_drag(
        egui::pos2(rect.center().x + 40.0, rect.center().y),
        egui::vec2(40.0, 0.0),
    );
    let panned = SliceCam {
        focus: c.focus + pan,
        ..c
    };
    // In-plane pan keeps the same section, so the ruler survives...
    ruler.sync_plane(panned);
    assert_eq!(ruler.anchors().len(), 2, "in-plane pan keeps the section");
    // ...and the world-anchored distance is unchanged.
    let after = ruler.distance_mm().unwrap();
    assert!(
        (before - after).abs() < 1e-6,
        "pan changed the measured mm: {before} -> {after}"
    );
}

#[test]
fn zoom_at_cursor_holds_after_an_extreme_pan() {
    // Hostile: shove the focus thousands of mm in-plane, THEN zoom-to-cursor.
    // The point under the pointer must still stay fixed (no precision blowup).
    let c = cam(Vec3::new(1.0, 0.2, -0.3), 8.0);
    let rect = image_rect();
    let (right, up) = slice_view_basis(c.normal);
    let panned = SliceCam {
        focus: c.focus + right * 5000.0 - up * 3000.0,
        ..c
    };
    let map = SlicePlaneMap::new(panned, rect);
    let cursor = egui::pos2(rect.left() + 190.0, rect.top() + 70.0);
    let world_under = map.panel_to_world(cursor);
    for ratio in [0.5_f32, 2.0] {
        let (new_focus, new_half) = SlicePlaneMap::zoom_focus_at_cursor(
            panned.focus,
            panned.half_extent,
            panned.normal,
            rect,
            cursor,
            ratio,
        );
        let zoomed = SliceCam {
            focus: new_focus,
            half_extent: new_half,
            ..panned
        };
        let back = SlicePlaneMap::new(zoomed, rect).world_to_panel(world_under);
        assert!(
            (back - cursor).length() < 1.0,
            "extreme-pan zoom drifted (ratio {ratio}): {back:?} vs {cursor:?}"
        );
    }
}

// ---- egui-driven click-vs-drag discrimination -------------------------

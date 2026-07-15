use super::*;
use occluview_core::{CameraProjection, Mesh, SceneMesh, Vertex};

fn ortho_camera_above() -> Camera {
    Camera {
        target: Vec3::ZERO,
        distance: 100.0,
        yaw: 0.0,
        pitch: 0.0,
        orientation: None,
        projection: CameraProjection::Orthographic,
        orthographic_height: 100.0,
        fovy: 45.0_f32.to_radians(),
        near: 0.1,
        far: 10_000.0,
    }
}

fn box_polygon(min: egui::Pos2, max: egui::Pos2) -> Vec<egui::Pos2> {
    vec![min, egui::pos2(max.x, min.y), max, egui::pos2(min.x, max.y)]
}

#[test]
fn screen_polygon_selection_takes_faces_overlapping_outline_not_disjoint_ones() {
    // Triangle 0 sits at the viewport center (overlaps the outline); triangle
    // 1 is far off, its whole screen bbox disjoint from the outline. Under
    // the intersection rule the overlapping face is taken and the disjoint
    // one is pruned — 0 selected, 1 not.
    let mesh = Mesh::new(
        Some("lasso".into()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(100.0, 100.0, 0.0)),
            Vertex::at(Vec3::new(101.0, 100.0, 0.0)),
            Vertex::at(Vec3::new(100.0, 101.0, 0.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    );
    assert!(mesh.is_ok(), "mesh should construct");
    let Ok(mesh) = mesh else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, 2) else {
        return;
    };
    let camera = ortho_camera_above();
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let polygon = box_polygon(egui::pos2(100.0, 100.0), egui::pos2(300.0, 300.0));

    let changed = selection.select_screen_polygon(
        &scene,
        &camera,
        ScreenPolygonSelectionRequest {
            viewport_rect: viewport,
            polygon_px: &polygon,
            unmark: false,
            through_mesh: false,
        },
    );

    assert_eq!(changed, Some(true));
    assert_eq!(selection.selected_faces, vec![true, false]);
}

#[test]
fn screen_polygon_selection_surface_mode_excludes_back_faces() {
    // Two triangles near the center: triangle 0 wound +z (faces the camera
    // above), triangle 1 wound -z (faces away). Surface mode selects only 0;
    // through-mesh mode selects both.
    let mesh = Mesh::new(
        Some("facing".into()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(4.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(3.0, 1.0, 0.0)),
        ],
        // [3,5,4] reverses [3,4,5] → triangle 1 faces -z.
        vec![0, 1, 2, 3, 5, 4],
    );
    assert!(mesh.is_ok(), "mesh should construct");
    let Ok(mesh) = mesh else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut surface) = FaceSelectionState::empty_for_layer(layer_id, 2) else {
        return;
    };
    let camera = ortho_camera_above();
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let polygon = box_polygon(egui::pos2(100.0, 100.0), egui::pos2(300.0, 300.0));

    surface
        .select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &polygon,
                unmark: false,
                through_mesh: false,
            },
        )
        .unwrap_or(false);
    assert_eq!(surface.selected_faces, vec![true, false]);

    let Some(mut through) = FaceSelectionState::empty_for_layer(layer_id, 2) else {
        return;
    };
    through
        .select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &polygon,
                unmark: false,
                through_mesh: true,
            },
        )
        .unwrap_or(false);
    assert_eq!(through.selected_faces, vec![true, true]);
}

#[test]
fn screen_polygon_selection_accumulates_and_shift_unmarks() {
    // exocad convention: completed outlines ACCUMULATE into the highlight
    // by default; an outline with unmark=true (SHIFT) clears its interior.
    let mesh = Mesh::new(
        Some("two".into()),
        vec![
            Vertex::at(Vec3::new(-10.0, -10.0, 0.0)),
            Vertex::at(Vec3::new(-8.0, -10.0, 0.0)),
            Vertex::at(Vec3::new(-9.0, -8.0, 0.0)),
            Vertex::at(Vec3::new(10.0, 10.0, 0.0)),
            Vertex::at(Vec3::new(12.0, 10.0, 0.0)),
            Vertex::at(Vec3::new(11.0, 12.0, 0.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    );
    assert!(mesh.is_ok(), "mesh should construct");
    let Ok(mesh) = mesh else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, 2) else {
        return;
    };
    let camera = ortho_camera_above();
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    // World ±10 maps near screen x ≈ 160 / 240 (center 200). Tall boxes cover
    // both possible screen-y directions (egui screen y is down) so each
    // triangle's full footprint lands inside its own polygon, never both.
    let left = box_polygon(egui::pos2(110.0, 100.0), egui::pos2(195.0, 320.0));
    let right = box_polygon(egui::pos2(205.0, 100.0), egui::pos2(290.0, 320.0));

    assert_eq!(
        selection.select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &left,
                unmark: false,
                through_mesh: false,
            },
        ),
        Some(true)
    );
    assert_eq!(selection.selected_faces, vec![true, false]);

    // Second outline without SHIFT accumulates into the highlight.
    assert_eq!(
        selection.select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &right,
                unmark: false,
                through_mesh: false,
            },
        ),
        Some(true)
    );
    assert_eq!(selection.selected_faces, vec![true, true]);

    // SHIFT outline un-marks its interior, leaving the rest highlighted.
    assert_eq!(
        selection.select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &right,
                unmark: true,
                through_mesh: false,
            },
        ),
        Some(true)
    );
    assert_eq!(selection.selected_faces, vec![true, false]);

    // Un-marking an already-clear region reports no change.
    assert_eq!(
        selection.select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &right,
                unmark: true,
                through_mesh: false,
            },
        ),
        Some(false)
    );
}

#[test]
fn screen_polygon_marquee_selects_every_enclosed_face_on_dense_mesh() {
    // An 8x8 quad grid (128 triangles). The marquee is a 4-point polygon
    // through the same path as the lasso: EVERY enclosed front face must
    // be taken — no per-pixel-cell decimation.
    let side: u32 = 8;
    let mut vertices = Vec::new();
    for y in 0..=side {
        for x in 0..=side {
            let fx = f32::from(u16::try_from(x).unwrap_or(0));
            let fy = f32::from(u16::try_from(y).unwrap_or(0));
            vertices.push(Vertex::at(Vec3::new(fx - 4.0, fy - 4.0, 0.0)));
        }
    }
    let mut indices = Vec::new();
    let stride = side + 1;
    for y in 0..side {
        for x in 0..side {
            let base = y * stride + x;
            indices.extend_from_slice(&[base, base + 1, base + stride]);
            indices.extend_from_slice(&[base + 1, base + stride + 1, base + stride]);
        }
    }
    let triangle_count = indices.len() / 3;
    let mesh = Mesh::new(Some("grid".into()), vertices, indices);
    assert!(mesh.is_ok(), "grid mesh should construct");
    let Ok(mesh) = mesh else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, triangle_count) else {
        return;
    };
    let camera = ortho_camera_above();
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    // The grid spans world +/-4 => comfortably inside this pixel rect.
    let marquee = box_polygon(egui::pos2(50.0, 50.0), egui::pos2(350.0, 350.0));

    let changed = selection.select_screen_polygon(
        &scene,
        &camera,
        ScreenPolygonSelectionRequest {
            viewport_rect: viewport,
            polygon_px: &marquee,
            unmark: false,
            through_mesh: false,
        },
    );

    assert_eq!(changed, Some(true));
    assert_eq!(selection.selected_count(), triangle_count);
}

// Intersection helper: run one arbitrary outline over `mesh` and return
// (selected, total). Unlike `surface_selected`, the caller supplies the
// polygon, so it exercises the polygon/triangle intersection rule directly.
fn polygon_selected(
    mesh: Mesh,
    camera: &Camera,
    polygon: &[egui::Pos2],
    through_mesh: bool,
) -> (usize, usize) {
    let triangle_count = mesh.triangle_count();
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, triangle_count) else {
        return (0, triangle_count);
    };
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    selection
        .select_screen_polygon(
            &scene,
            camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: polygon,
                unmark: false,
                through_mesh,
            },
        )
        .unwrap_or(false);
    (selection.selected_count(), triangle_count)
}

#[test]
fn micro_triangle_selects_in_surface_mode_when_zoomed() {
    // Hi-res lab scanners emit facets with ~15 um edges. The old absolute
    // degeneracy epsilon (mm^4 units) silently culled them from surface
    // mode no matter how large they were on screen; degeneracy must be
    // relative to the triangle's own scale.
    let micro = Mesh::new(
        Some("micro-tri".into()),
        vec![
            Vertex::at(Vec3::new(-0.0075, -0.0075, 0.0)),
            Vertex::at(Vec3::new(0.0075, -0.0075, 0.0)),
            Vertex::at(Vec3::new(0.0, 0.0075, 0.0)),
        ],
        vec![0, 1, 2],
    );
    assert!(micro.is_ok(), "micro mesh should construct");
    let Ok(micro) = micro else {
        return;
    };
    // Zoom the orthographic camera until the 15 um triangle spans ~60 px.
    let mut camera = ortho_camera_above();
    camera.orthographic_height = 0.1;
    let lasso = box_polygon(egui::pos2(150.0, 150.0), egui::pos2(250.0, 250.0));
    let (sel, total) = polygon_selected(micro, &camera, &lasso, false);
    assert_eq!(total, 1);
    assert_eq!(
        sel, 1,
        "a camera-facing micro triangle must select in surface mode"
    );
}

#[test]
fn zero_area_triangle_still_skipped_in_surface_mode() {
    // Genuinely degenerate geometry (all three corners collinear) has no
    // orientation; surface mode must keep skipping it while through mode
    // keeps taking whatever the outline touches.
    let sliver = Mesh::new(
        Some("zero-area".into()),
        vec![
            Vertex::at(Vec3::new(-10.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(10.0, 0.0, 0.0)),
        ],
        vec![0, 1, 2],
    );
    assert!(sliver.is_ok(), "collinear mesh should construct");
    let Ok(sliver) = sliver else {
        return;
    };
    let lasso = box_polygon(egui::pos2(100.0, 100.0), egui::pos2(300.0, 300.0));
    let (surface_sel, _) = polygon_selected(sliver.clone(), &ortho_camera_above(), &lasso, false);
    assert_eq!(surface_sel, 0, "zero-area triangle has no facing side");
    let (through_sel, _) = polygon_selected(sliver, &ortho_camera_above(), &lasso, true);
    assert_eq!(through_sel, 1, "through mode still takes what it touches");
}

#[test]
fn small_lasso_marks_huge_flat_quad_the_old_all_verts_rule_dropped() {
    // The operator's bug: a large FLAT surface (a few huge triangles) with a
    // lasso smaller than a triangle selected NOTHING under the old "all three
    // vertices inside" rule, while dense curved regions (many tiny triangles)
    // always caught. A quad filling the viewport is two triangles whose
    // vertices sit at the far corners; a small central lasso encloses none of
    // them but straddles their shared diagonal. Intersection marks both.
    let quad = Mesh::new(
        Some("flat-quad".into()),
        vec![
            Vertex::at(Vec3::new(-40.0, -40.0, 0.0)),
            Vertex::at(Vec3::new(40.0, -40.0, 0.0)),
            Vertex::at(Vec3::new(-40.0, 40.0, 0.0)),
            Vertex::at(Vec3::new(40.0, 40.0, 0.0)),
        ],
        // Both triangles wound to face +z (toward the camera above).
        vec![0, 1, 2, 1, 3, 2],
    );
    assert!(quad.is_ok(), "quad mesh should construct");
    let Ok(quad) = quad else {
        return;
    };
    // A small box at the screen center; every quad vertex projects to a far
    // corner, so no triangle vertex lands inside it (the old rule -> 0).
    let lasso = box_polygon(egui::pos2(180.0, 180.0), egui::pos2(220.0, 220.0));
    // Surface mode (through_mesh = false) — exactly the reported condition.
    let (sel, total) = polygon_selected(quad, &ortho_camera_above(), &lasso, false);
    assert_eq!(total, 2, "quad is two triangles");
    assert_eq!(
        sel, 2,
        "a small lasso must mark the whole flat quad it sits on"
    );
}

#[test]
fn lasso_fully_inside_one_giant_triangle_marks_it() {
    // A single huge triangle; the lasso sits wholly in its interior, its
    // outline touching no edge and enclosing no triangle vertex. Only the
    // "outline vertex inside triangle" test can catch this — the flat-face
    // case in its purest form.
    let triangle = Mesh::new(
        Some("giant-tri".into()),
        vec![
            Vertex::at(Vec3::new(-45.0, -45.0, 0.0)),
            Vertex::at(Vec3::new(45.0, -45.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 45.0, 0.0)),
        ],
        vec![0, 1, 2],
    );
    assert!(triangle.is_ok(), "triangle mesh should construct");
    let Ok(triangle) = triangle else {
        return;
    };
    let lasso = box_polygon(egui::pos2(180.0, 180.0), egui::pos2(220.0, 220.0));
    let (sel, total) = polygon_selected(triangle, &ortho_camera_above(), &lasso, false);
    assert_eq!(total, 1);
    assert_eq!(sel, 1, "a lasso inside a giant triangle must mark it");
}

#[test]
fn lasso_edge_crossing_big_triangle_marks_it() {
    // A wide, thin horizontal lasso spans the full width across the middle of
    // a big triangle: every lasso corner is left/right OUTSIDE the triangle,
    // every triangle vertex is above/below OUTSIDE the lasso — so only the
    // edge-crossing test can catch it.
    let triangle = Mesh::new(
        Some("cross-tri".into()),
        vec![
            Vertex::at(Vec3::new(0.0, 40.0, 0.0)),
            Vertex::at(Vec3::new(-35.0, -40.0, 0.0)),
            Vertex::at(Vec3::new(35.0, -40.0, 0.0)),
        ],
        vec![0, 1, 2],
    );
    assert!(triangle.is_ok(), "triangle mesh should construct");
    let Ok(triangle) = triangle else {
        return;
    };
    // Band across screen y in [290, 310]; corners at x = 10 / 390 sit outside
    // the triangle's ~[90, 310] span there, so no corner is inside it.
    let lasso = box_polygon(egui::pos2(10.0, 290.0), egui::pos2(390.0, 310.0));
    let (sel, total) = polygon_selected(triangle, &ortho_camera_above(), &lasso, false);
    assert_eq!(total, 1);
    assert_eq!(
        sel, 1,
        "a lasso edge slicing through a triangle must mark it"
    );
}

// A dense circular lasso of `point_count` points and the given screen
// radius about the viewport center — a freehand outline with many samples.
fn circular_lasso(point_count: u32, radius: f32) -> Vec<egui::Pos2> {
    let center = egui::pos2(200.0, 200.0);
    let point_count_f = f32::from(u16::try_from(point_count).unwrap_or(1)).max(1.0);
    (0..point_count)
        .map(|i| {
            let theta =
                std::f32::consts::TAU * (f32::from(u16::try_from(i).unwrap_or(0)) / point_count_f);
            egui::pos2(
                center.x + radius * theta.cos(),
                center.y + radius * theta.sin(),
            )
        })
        .collect()
}

// Intentional test diagnostics: the perf smoke reports its measured wall
// times to the test log (the crate otherwise denies stray prints).
#[allow(clippy::print_stderr)]
#[test]
fn perf_dense_lasso_over_large_mesh_stays_bounded() {
    // Perf smoke: a 200-point lasso over a ~500k-triangle grid, measured for
    // two lasso sizes — a representative regional selection (target < 150 ms)
    // and a near-worst-case one covering ~half the mesh. Prints both wall
    // times and asserts only a LOOSE bound (the box is shared under
    // concurrent load; this is a smoke guard against an O(N*P) blow-up, not a
    // precise benchmark). The bbox prune makes the effective cost scale with
    // the triangles under the outline, not the whole mesh.
    let cells: u32 = 500; // 2 * 500 * 500 = 500_000 triangles.
    let stride = cells + 1;
    let extent = 40.0_f32;
    let cells_f = f32::from(u16::try_from(cells).unwrap_or(1)).max(1.0);
    let mut vertices = Vec::new();
    for iy in 0..=cells {
        for ix in 0..=cells {
            let fx = (f32::from(u16::try_from(ix).unwrap_or(0)) / cells_f) * 2.0 * extent - extent;
            let fy = (f32::from(u16::try_from(iy).unwrap_or(0)) / cells_f) * 2.0 * extent - extent;
            vertices.push(Vertex::at(Vec3::new(fx, fy, 0.0)));
        }
    }
    let mut indices = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            let base = y * stride + x;
            indices.extend_from_slice(&[base, base + 1, base + stride]);
            indices.extend_from_slice(&[base + 1, base + stride + 1, base + stride]);
        }
    }
    let triangle_count = indices.len() / 3;
    let Ok(mesh) = Mesh::new(Some("perf".into()), vertices, indices) else {
        return;
    };
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let camera = ortho_camera_above();
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));

    // (label, radius): a regional dense lasso then a wide near-worst-case one.
    for &(label, radius) in &[("regional", 55.0_f32), ("wide", 120.0_f32)] {
        let lasso = circular_lasso(200, radius);
        let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, triangle_count)
        else {
            return;
        };
        let started = std::time::Instant::now();
        let changed = selection.select_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &lasso,
                unmark: false,
                through_mesh: true,
            },
        );
        let elapsed = started.elapsed();
        eprintln!(
                "perf[{label}]: {}-point lasso over {triangle_count} triangles selected {} in {elapsed:?}",
                lasso.len(),
                selection.selected_count(),
            );

        assert_eq!(changed, Some(true));
        assert!(
            selection.selected_count() > 0,
            "the {label} lasso must select the disk it covers"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "{label} lasso selection must stay well-bounded, took {elapsed:?}"
        );
    }
}

// Surface-mode helper: run a full-viewport polygon over `mesh` and return
// (selected, total). The polygon covers the whole viewport so containment
// never rejects a face — only the front-facing test can — isolating the
// surface-mode gate under test.
fn surface_selected(mesh: Mesh, camera: &Camera) -> (usize, usize) {
    let triangle_count = mesh.triangle_count();
    let mut scene = Scene::new();
    let layer_index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[layer_index].id();
    let Some(mut selection) = FaceSelectionState::empty_for_layer(layer_id, triangle_count) else {
        return (0, triangle_count);
    };
    let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    let polygon = box_polygon(egui::pos2(5.0, 5.0), egui::pos2(395.0, 395.0));
    selection
        .select_screen_polygon(
            &scene,
            camera,
            ScreenPolygonSelectionRequest {
                viewport_rect: viewport,
                polygon_px: &polygon,
                unmark: false,
                through_mesh: false,
            },
        )
        .unwrap_or(false);
    (selection.selected_count(), triangle_count)
}

fn rot_x(p: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(p.x, p.y * c - p.z * s, p.y * s + p.z * c)
}

fn rot_y(p: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(p.x * c + p.z * s, p.y, -p.x * s + p.z * c)
}

/// Flat grid facing +z, then rotated `tilt_deg` about an axis and
/// translated by `offset`. Outward winding (geometric normal +z before
/// rotation). `Vertex::at` leaves stored normals unset (zeroed) as a plain
/// STL would — the surface test must ignore them and use geometry.
fn grid_plane(side: u32, extent: f32, rotate: impl Fn(Vec3) -> Vec3, offset: Vec3) -> Mesh {
    let side_f = f32::from(u16::try_from(side).unwrap_or(1)).max(1.0);
    let mut vertices = Vec::new();
    for iy in 0..=side {
        for ix in 0..=side {
            let fx = (f32::from(u16::try_from(ix).unwrap_or(0)) / side_f) * 2.0 * extent - extent;
            let fy = (f32::from(u16::try_from(iy).unwrap_or(0)) / side_f) * 2.0 * extent - extent;
            vertices.push(Vertex::at(rotate(Vec3::new(fx, fy, 0.0)) + offset));
        }
    }
    let mut indices = Vec::new();
    let stride = side + 1;
    for y in 0..side {
        for x in 0..side {
            let base = y * stride + x;
            indices.extend_from_slice(&[base, base + 1, base + stride]);
            indices.extend_from_slice(&[base + 1, base + stride + 1, base + stride]);
        }
    }
    Mesh::new(Some("grid".into()), vertices, indices).unwrap_or_else(|_| Mesh::empty())
}

/// A curved annular cap on ±z, outward-wound, sweeping from 10° to 75° off
/// the pole (avoids the coincident-pole degenerate fan and stays short of
/// the grazing equator, so every face is a clean, non-degenerate front/back
/// face). `front` puts the cap toward the camera (all triangles
/// front-facing); `!front` faces it away (all back-facing).
fn hemisphere(rings: u32, segs: u32, radius: f32, front: bool) -> Mesh {
    let sign = if front { 1.0 } else { -1.0 };
    let phi_min = 10.0_f32.to_radians();
    let phi_max = 75.0_f32.to_radians();
    let rings_f = f32::from(u16::try_from(rings).unwrap_or(1)).max(1.0);
    let segs_f = f32::from(u16::try_from(segs).unwrap_or(1)).max(1.0);
    let mut vertices = Vec::new();
    for r in 0..=rings {
        let t = f32::from(u16::try_from(r).unwrap_or(0)) / rings_f;
        let phi = phi_min + (phi_max - phi_min) * t;
        let z = sign * radius * phi.cos();
        let rr = radius * phi.sin();
        for s in 0..=segs {
            let theta = std::f32::consts::TAU * (f32::from(u16::try_from(s).unwrap_or(0)) / segs_f);
            vertices.push(Vertex::at(Vec3::new(rr * theta.cos(), rr * theta.sin(), z)));
        }
    }
    let stride = segs + 1;
    let mut indices = Vec::new();
    for r in 0..rings {
        for s in 0..segs {
            let base = r * stride + s;
            if front {
                indices.extend_from_slice(&[base, base + stride, base + 1]);
                indices.extend_from_slice(&[base + 1, base + stride, base + stride + 1]);
            } else {
                indices.extend_from_slice(&[base, base + 1, base + stride]);
                indices.extend_from_slice(&[base + 1, base + stride + 1, base + stride]);
            }
        }
    }
    Mesh::new(Some("hemi".into()), vertices, indices).unwrap_or_else(|_| Mesh::empty())
}

#[test]
fn surface_selects_flat_plane_facing_camera() {
    // A flat grid squarely facing the camera: every front face is enclosed
    // and selected. Guards against ever culling a plainly-visible flat area.
    let (sel, total) = surface_selected(
        grid_plane(16, 30.0, |p| p, Vec3::ZERO),
        &ortho_camera_above(),
    );
    assert_eq!(sel, total, "flat facing plane must fully select");
}

#[test]
fn surface_selects_tilted_planes() {
    // Rotated about X across a wide oblique range; still fully front-facing
    // (normal keeps a positive component toward the viewer), so all select.
    for &tilt in &[30.0_f32, 60.0, 85.0] {
        let a = tilt.to_radians();
        let (sel, total) = surface_selected(
            grid_plane(16, 30.0, move |p| rot_x(p, a), Vec3::ZERO),
            &ortho_camera_above(),
        );
        assert_eq!(sel, total, "tilted plane {tilt}deg must fully select");
    }
}

#[test]
fn surface_selects_oblique_flat_patch_offset_from_view_axis() {
    // Regression for the ortho front-facing bug: a genuinely front-facing
    // flat patch (normal z-component cos(beta) > 0) that is oblique AND
    // laterally offset from the view axis. The old `eye - centroid` test
    // swung the effective view vector with the lateral offset and culled the
    // WHOLE patch (0 selected); the ortho-consistent constant view direction
    // selects all of it. This is the "flat surface off to the side won't
    // select" the operator reported.
    for &beta in &[70.0_f32, 78.0, 84.0] {
        let a = beta.to_radians();
        for &off_x in &[20.0_f32, 30.0] {
            let (sel, total) = surface_selected(
                grid_plane(12, 5.0, move |p| rot_y(p, a), Vec3::new(off_x, 0.0, 0.0)),
                &ortho_camera_above(),
            );
            assert_eq!(
                sel, total,
                "oblique front-facing flat patch beta={beta} offX={off_x} must fully select"
            );
        }
    }
}

#[test]
fn surface_selects_dome_front_but_not_back() {
    // A dome facing the camera fully selects (all front-facing); the same
    // dome turned away selects nothing (all back-facing). The front-facing
    // gate must keep culling true back faces.
    let (front_sel, front_total) =
        surface_selected(hemisphere(16, 24, 30.0, true), &ortho_camera_above());
    assert_eq!(front_sel, front_total, "front dome must fully select");
    assert!(front_total > 0, "front dome must have faces");

    let (back_sel, _) = surface_selected(hemisphere(16, 24, 30.0, false), &ortho_camera_above());
    assert_eq!(back_sel, 0, "back-facing dome must select nothing");
}

#[test]
fn surface_selects_by_geometry_ignoring_stored_vertex_normals() {
    // A flat plane facing the camera (geometry front-faces +z) but with
    // every STORED vertex normal deliberately pointing AWAY (-z). If the
    // surface test trusted stored normals it would cull the whole plane;
    // because it recomputes the face normal from positions, it selects all.
    // (Mesh::new repairs all-zero normals, so hostile-but-usable normals are
    // the meaningful adversary for "never trust stored normals".)
    let side: u32 = 16;
    let side_f = f32::from(u16::try_from(side).unwrap_or(1)).max(1.0);
    let extent = 20.0_f32;
    let mut vertices = Vec::new();
    for iy in 0..=side {
        for ix in 0..=side {
            let fx = (f32::from(u16::try_from(ix).unwrap_or(0)) / side_f) * 2.0 * extent - extent;
            let fy = (f32::from(u16::try_from(iy).unwrap_or(0)) / side_f) * 2.0 * extent - extent;
            vertices.push(Vertex::at(Vec3::new(fx, fy, 0.0)).with_normal(Vec3::NEG_Z));
        }
    }
    let mut indices = Vec::new();
    let stride = side + 1;
    for y in 0..side {
        for x in 0..side {
            let base = y * stride + x;
            indices.extend_from_slice(&[base, base + 1, base + stride]);
            indices.extend_from_slice(&[base + 1, base + stride + 1, base + stride]);
        }
    }
    let Ok(plane) = Mesh::new(Some("hostile".into()), vertices, indices) else {
        return;
    };
    assert!(
        plane.vertices().iter().all(|v| v.normal[2] < 0.0),
        "fixture must keep stored normals pointing away from the camera"
    );
    let (sel, total) = surface_selected(plane, &ortho_camera_above());
    assert_eq!(
        sel, total,
        "flat plane must fully select from geometry despite hostile stored normals"
    );
}

#[test]
fn point_in_polygon_handles_box_and_concave_outlines() {
    let box_poly = box_polygon(egui::pos2(10.0, 10.0), egui::pos2(20.0, 30.0));
    assert!(point_in_polygon(egui::pos2(15.0, 20.0), &box_poly));
    assert!(!point_in_polygon(egui::pos2(5.0, 20.0), &box_poly));
    assert!(!point_in_polygon(egui::pos2(25.0, 20.0), &box_poly));
    // Exactly on the outline is implementation-defined; only assert strict
    // interior/exterior here.
    assert!(!point_in_polygon(egui::pos2(15.0, 40.0), &box_poly));
}

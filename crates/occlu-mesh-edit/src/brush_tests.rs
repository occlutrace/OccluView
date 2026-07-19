//! Tests for the freeform sculpting brush kernel (split out of
//! `brush.rs` to hold the workspace's 800-line file budget).

use crate::brush::*;
use crate::brush_math::shortest_incident_edge;
use crate::{EditVertex, MeshEditBuffers, MeshEditError, MeshTopology};
use glam::Vec3;

fn v(position: [f32; 3]) -> EditVertex {
    EditVertex::at(position)
}

/// A flat 11x11 grid (1mm spacing) with random-ish bump noise added to
/// interior vertices, sharing indexed topology (not soup) — the baseline
/// fixture for the Smooth/Add/Remove tests.
fn bumpy_patch(bump: f32) -> MeshEditBuffers {
    let n = 11usize;
    let mut vertices = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let x = i as f32 - (n as f32 - 1.0) / 2.0;
            let y = j as f32 - (n as f32 - 1.0) / 2.0;
            // Deterministic pseudo-noise: cheap, reproducible, no RNG dep.
            let noise = ((i * 7 + j * 13) % 5) as f32 - 2.0;
            let z = if i > 0 && i < n - 1 && j > 0 && j < n - 1 {
                noise * bump
            } else {
                0.0
            };
            vertices.push(v([x, y, z]));
        }
    }
    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    let idx = |i: usize, j: usize| (j * n + i) as u32;
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let mut mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");
    mesh
}

fn height_variance(mesh: &MeshEditBuffers, n: usize) -> f32 {
    let heights: Vec<f32> = mesh.vertices.iter().map(|v| v.position[2]).collect();
    let interior: Vec<f32> = (1..n - 1)
        .flat_map(|j| (1..n - 1).map(move |i| j * n + i))
        .map(|idx| heights[idx])
        .collect();
    let mean = interior.iter().sum::<f32>() / interior.len() as f32;
    interior.iter().map(|h| (h - mean).powi(2)).sum::<f32>() / interior.len() as f32
}

/// A dab centered at the origin. `view_dir` points into the scene from a
/// camera above the +Z face, so the camera-oriented brush normal comes out
/// +Z: Add builds toward +Z, Remove carves toward -Z (matching the flat
/// patch's own outward normal, but now robust to inverted normals too).
fn center_stroke(radius_mm: f32, strength: f32) -> BrushStroke {
    BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm,
        strength,
        view_dir: [0.0, 0.0, -1.0],
    }
}

#[test]
fn smooth_reduces_height_variance_without_eroding_the_patch_extent() {
    let n = 11;
    let mesh = bumpy_patch(0.6);
    let before_variance = height_variance(&mesh, n);
    let before_bbox = bbox_diagonal(&mesh);

    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    for _ in 0..20 {
        let outcome = session.apply_stroke(center_stroke(6.0, 1.0), BrushMode::Smooth);
        assert!(!outcome.touched_vertices.is_empty());
    }
    let result = session.finish();

    let after_variance = height_variance(&result.mesh, n);
    let after_bbox = bbox_diagonal(&result.mesh);
    assert!(
        after_variance < before_variance * 0.5,
        "smoothing should substantially flatten bumps: {before_variance} -> {after_variance}"
    );
    // The open boundary is pinned, so the patch's XY extent (which dominates
    // the diagonal) must not erode inward the way an un-pinned Laplacian would.
    assert!(
        after_bbox > before_bbox * 0.9,
        "boundary-pinned smoothing must not shrink the patch extent: {before_bbox} -> {after_bbox}"
    );
    assert!(result.report.moved_vertices > 0);
}

#[test]
fn smooth_leaves_open_boundary_vertices_pinned() {
    let mesh = bumpy_patch(0.6);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // A corner vertex of the grid: it sits on the open boundary, so smoothing
    // over it must not drag it inward and erode the scan edge.
    let corner = 0usize;
    let before = mesh.vertices[corner].position;
    for _ in 0..10 {
        session.apply_stroke(
            BrushStroke {
                center: mesh.vertices[corner].position,
                radius_mm: 4.0,
                strength: 1.0,
                view_dir: [0.0, 0.0, -1.0],
            },
            BrushMode::Smooth,
        );
    }
    assert_eq!(
        session.position(corner).to_array(),
        before,
        "an open-boundary corner must stay pinned under smoothing"
    );
}

#[test]
fn falloff_leaves_vertices_outside_the_radius_untouched() {
    let mesh = bumpy_patch(0.6);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let outcome = session.apply_stroke(center_stroke(2.0, 1.0), BrushMode::Smooth);
    let result = session.finish();

    // Far corner vertex (index 0, at roughly (-5,-5,0)) is well outside a
    // 2mm-radius dab centered at the origin.
    assert!(!outcome.touched_vertices.contains(&0));
    assert_eq!(
        result.mesh.vertices[0].position, mesh.vertices[0].position,
        "vertex outside the brush radius must not move"
    );
}

#[test]
fn add_builds_toward_the_camera_and_remove_carves_away() {
    let mesh = bumpy_patch(0.0); // perfectly flat patch
    let center_index = 5 * 11 + 5; // the exact center vertex

    let mut add_session = BrushSession::prepare(&mesh).expect("prepare");
    add_session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Add);
    assert!(
        add_session.position(center_index).z > 0.01,
        "Add should build the surface up toward the camera (+Z here)"
    );

    let mut remove_session = BrushSession::prepare(&mesh).expect("prepare");
    remove_session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Remove);
    assert!(
        remove_session.position(center_index).z < -0.01,
        "Remove should carve the surface away from the camera (-Z here)"
    );
}

#[test]
fn add_builds_toward_the_camera_with_a_partially_inverted_patch() {
    // A realistic messy patch: a MINORITY (~1 in 3) of the normals are flipped,
    // the rest correct. The front (camera-facing) bucket still dominates, so the
    // brush normal comes from AVERAGING the trusted majority — not the pure
    // camera fallback that a fully-inverted patch would take — and Add still
    // builds toward the camera (+Z).
    let mut mesh = bumpy_patch(0.0);
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        if index % 3 == 0 {
            vertex.normal = [-vertex.normal[0], -vertex.normal[1], -vertex.normal[2]];
        }
    }
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let center_index = 5 * 11 + 5;
    session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Add);
    assert!(
        session.position(center_index).z > 0.01,
        "Add must build toward the camera when the trusted-facing normals are the majority"
    );
}

#[test]
fn the_clamp_holds_on_soup_duplicates_of_a_tiny_edged_corner() {
    // Two triangles sharing an edge as SOUP, scaled so the real edges are 0.1mm.
    // The shared corner has duplicate array slots whose own welded ring is empty
    // — pre-fix they'd inherit the generous isolated-vertex budget and overrun
    // the representative's tight clamp. The step must stay bounded by the REAL
    // 0.1mm edge (budget 0.1 * 0.5 = 0.05mm), not the loose 0.5mm fallback, even
    // under a dab whose raw amplitude (radius 1.0 * gain 0.08 = 0.08mm) exceeds
    // the correct budget.
    let s = 0.1_f32;
    let vertices = vec![
        v([0.0, 0.0, 0.0]),
        v([s, 0.0, 0.0]),
        v([0.0, s, 0.0]),
        v([s, 0.0, 0.0]),
        v([s, s, 0.0]),
        v([0.0, s, 0.0]),
    ];
    let mut mesh = MeshEditBuffers {
        vertices,
        indices: vec![0, 1, 2, 3, 4, 5],
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");

    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    session.apply_stroke(
        BrushStroke {
            center: [s * 0.5, s * 0.5, 0.0],
            radius_mm: 1.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        },
        BrushMode::Add,
    );
    // Both soup copies of the shared corner (1 and 3) must move together AND by
    // no more than the real-edge budget.
    let moved_1 = session.position(1);
    let moved_3 = session.position(3);
    assert_eq!(
        moved_1.to_array(),
        moved_3.to_array(),
        "soup copies must not crack"
    );
    assert!(
        moved_1.z <= s * 0.5 + 1e-4,
        "the clamp must bound the soup corner to its real 0.05mm budget, got {}",
        moved_1.z
    );
}

#[test]
fn add_builds_toward_the_camera_even_with_inverted_normals() {
    // A flat patch whose normals have been flipped to -Z (an inverted-normal
    // scan patch). The camera is still above (+Z), so Add must STILL build
    // toward the camera (+Z) — the brush normal is chosen by camera agreement,
    // not by the scan's untrustworthy per-vertex normals.
    let mut mesh = bumpy_patch(0.0);
    for vertex in &mut mesh.vertices {
        vertex.normal = [-vertex.normal[0], -vertex.normal[1], -vertex.normal[2]];
    }
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let center_index = 5 * 11 + 5;
    session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Add);
    assert!(
        session.position(center_index).z > 0.01,
        "Add must build toward the camera regardless of inverted scan normals"
    );
}

#[test]
fn add_pushes_the_region_coherently_without_carving_a_pothole() {
    // The old per-vertex-normal push left potholes: an interior vertex could
    // end up LOWER than its neighbors. Coherent single-normal push must leave
    // the brushed dome monotone — the center is the highest point, and every
    // ring closer to the center is at least as high as the ring outside it.
    let mesh = bumpy_patch(0.0);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    for _ in 0..6 {
        session.apply_stroke(center_stroke(5.0, 1.0), BrushMode::Add);
    }
    let n = 11usize;
    let height = |i: usize, j: usize| session.position(j * n + i).z;
    let center = height(5, 5);
    let inner_ring = height(4, 5)
        .min(height(6, 5))
        .min(height(5, 4))
        .min(height(5, 6));
    let outer_ring = height(3, 5)
        .min(height(7, 5))
        .min(height(5, 3))
        .min(height(5, 7));
    assert!(
        center >= inner_ring - 1e-4 && inner_ring >= outer_ring - 1e-4,
        "Add must form a monotone dome, not a potholed surface: \
         center {center}, inner {inner_ring}, outer {outer_ring}"
    );
}

#[test]
fn guard_prevents_triangle_inversion_under_a_large_add_stroke() {
    let mesh = bumpy_patch(0.0);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // Many repeated full-strength dabs: without the clamp this would eventually
    // invert the central fan of triangles.
    for _ in 0..40 {
        session.apply_stroke(center_stroke(3.0, 1.0), BrushMode::Add);
    }
    let result = session.finish();

    for triangle in result.mesh.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (pa, pb, pc) = (
            Vec3::from_array(result.mesh.vertices[a].position),
            Vec3::from_array(result.mesh.vertices[b].position),
            Vec3::from_array(result.mesh.vertices[c].position),
        );
        let area = (pb - pa).cross(pc - pa).length();
        assert!(
            area > 1e-6,
            "guard must keep every triangle non-degenerate under repeated dabs"
        );
    }
}

#[test]
fn soup_duplicates_of_one_corner_never_crack_apart() {
    // Two triangles sharing an edge, expressed as SOUP: every corner is
    // its own vertex, so the shared edge's two corners each appear twice
    // at byte-identical positions/colors/uv.
    let p0 = [0.0, 0.0, 0.0];
    let p1 = [1.0, 0.0, 0.0];
    let p2 = [0.0, 1.0, 0.0];
    let p3 = [1.0, 1.0, 0.0];
    let vertices = vec![
        v(p0),
        v(p1),
        v(p2), // triangle 0: 0,1,2
        v(p1),
        v(p3),
        v(p2), // triangle 1: shares edge p1-p2 with triangle 0
    ];
    let mut mesh = MeshEditBuffers {
        vertices,
        indices: vec![0, 1, 2, 3, 4, 5],
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");

    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // Dab centered on the shared edge, radius large enough to move it.
    session.apply_stroke(
        BrushStroke {
            center: [0.5, 0.0, 0.0],
            radius_mm: 5.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        },
        BrushMode::Add,
    );
    let result = session.finish();

    // Vertex 1 (soup copy of p1 in triangle 0) and vertex 3 (soup copy of
    // p1 in triangle 1) must end up at the SAME position — no crack.
    assert_eq!(
        result.mesh.vertices[1].position, result.mesh.vertices[3].position,
        "soup duplicates of the same physical corner must move together"
    );
}

#[test]
fn zero_strength_and_zero_radius_are_no_ops() {
    let mesh = bumpy_patch(0.4);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    assert!(session
        .apply_stroke(center_stroke(4.0, 0.0), BrushMode::Smooth)
        .touched_vertices
        .is_empty());
    assert!(session
        .apply_stroke(center_stroke(0.0, 1.0), BrushMode::Smooth)
        .touched_vertices
        .is_empty());
    let result = session.finish();
    assert_eq!(result.report.moved_vertices, 0);
}

#[test]
fn smooth_pass_count_grows_with_strength_so_forced_mode_is_strongest() {
    // A firmer press / forced Shift mode (strength ~1) must smooth more than a
    // light touch: strictly more of the surface flattens in one dab.
    let light = {
        let mesh = bumpy_patch(0.6);
        let mut session = BrushSession::prepare(&mesh).expect("prepare");
        session.apply_stroke(center_stroke(6.0, 0.1), BrushMode::Smooth);
        height_variance(&session.finish().mesh, 11)
    };
    let forced = {
        let mesh = bumpy_patch(0.6);
        let mut session = BrushSession::prepare(&mesh).expect("prepare");
        session.apply_stroke(center_stroke(6.0, 1.0), BrushMode::Smooth);
        height_variance(&session.finish().mesh, 11)
    };
    assert!(
        forced < light,
        "forced smoothing (more passes) must flatten more than a light touch: \
         forced {forced} vs light {light}"
    );
}

#[test]
fn point_cloud_is_rejected() {
    let mesh = MeshEditBuffers {
        vertices: vec![v([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };
    assert_eq!(
        BrushSession::prepare(&mesh).err(),
        Some(MeshEditError::UnsupportedPointCloud)
    );
}

#[test]
fn stroke_application_is_deterministic() {
    let mesh = bumpy_patch(0.6);
    let run = || {
        let mut session = BrushSession::prepare(&mesh).expect("prepare");
        session.apply_stroke(center_stroke(6.0, 1.0), BrushMode::Smooth);
        session.finish().mesh
    };
    let a = run();
    let b = run();
    assert_eq!(a.vertices.len(), b.vertices.len());
    for (va, vb) in a.vertices.iter().zip(&b.vertices) {
        assert_eq!(va.position, vb.position);
        assert_eq!(va.normal, vb.normal);
    }
}

// Regression for the anti-inversion guard's budget floor (issue review
// 2026-07-18): a genuinely small welded edge must NOT be floored, or
// `clamp_step` permits a larger step than the local topology can tolerate.
#[test]
fn shortest_incident_edge_does_not_floor_a_genuinely_small_edge() {
    let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.01, 0.0, 0.0)];
    let result = shortest_incident_edge(&positions, &[1], positions[0]);
    assert!(
        (result - 0.01).abs() < 1e-6,
        "a real 0.01mm neighbor edge must not be floored: got {result}"
    );
}

#[test]
fn shortest_incident_edge_caps_a_large_edge_at_one_millimeter() {
    let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(50.0, 0.0, 0.0)];
    let result = shortest_incident_edge(&positions, &[1], positions[0]);
    assert!(
        (result - 1.0).abs() < 1e-6,
        "a huge edge should still be capped at 1mm so one dab can't take an oversized jump: got {result}"
    );
}

#[test]
fn shortest_incident_edge_falls_back_generously_when_isolated() {
    let positions = vec![Vec3::new(0.0, 0.0, 0.0)];
    let result = shortest_incident_edge(&positions, &[], positions[0]);
    assert!(
        (result - 1.0).abs() < 1e-6,
        "an isolated vertex (no neighbors to violate) should fall back to a generous budget: got {result}"
    );
}

/// A dense, fine-scale patch -- realistic for a fine occlusal groove or margin
/// line, where a coarse floor on `shortest_incident_edge` would inflate the
/// anti-inversion budget past what this topology can tolerate.
fn dense_patch(spacing: f32) -> MeshEditBuffers {
    let n = 7usize;
    let mut vertices = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let x = (i as f32 - (n as f32 - 1.0) / 2.0) * spacing;
            let y = (j as f32 - (n as f32 - 1.0) / 2.0) * spacing;
            vertices.push(v([x, y, 0.0]));
        }
    }
    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    let idx = |i: usize, j: usize| (j * n + i) as u32;
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let mut mesh = MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    };
    crate::recompute_all_normals(&mut mesh.vertices, &mesh.indices).expect("seed normals");
    mesh
}

#[test]
fn guard_scales_the_step_budget_with_a_genuinely_tiny_edge() {
    let spacing = 0.02_f32;
    let mesh = dense_patch(spacing);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let center_index = 3 * 7 + 3;
    let before_z = mesh.vertices[center_index].position[2];

    // A radius/gain that WANTS to move more than the local topology allows, so
    // the edge-proportional clamp is what actually decides the step.
    let outcome = session.apply_stroke(center_stroke(0.5, 1.0), BrushMode::Add);
    assert!(!outcome.touched_vertices.is_empty());
    let after_z = session.position(center_index).z;
    let step = (after_z - before_z).abs();

    // The push is bounded by spacing * MAX_STEP_FRACTION_OF_EDGE (0.5) = 0.01mm
    // — scaled to the real edge length, not a coarse floor — and the auto-smooth
    // pass only reduces it further, so the net step must stay within that budget.
    // A loose (fallback) budget would have let the push reach the full ~0.04mm.
    let expected_budget = spacing * 0.5;
    assert!(
        step <= expected_budget + 1e-4,
        "the clamped step ({step}mm) must stay within the edge-proportional \
         budget ({expected_budget}mm)"
    );
}

// Regression for the spatial-grid staleness bug (issue review 2026-07-18):
// `VertexGrid` indexes positions as of its last build, so a session that moves
// a vertex far without rebuilding would keep searching near its STALE original
// bucket and silently miss it once the cursor follows it there.
#[test]
fn apply_stroke_still_finds_a_vertex_after_sustained_building_far_from_its_start() {
    let mesh = bumpy_patch(0.0); // flat patch, easy to reason about
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let center_index = 5 * 11 + 5;

    // Build the center up repeatedly (one dab per input frame) -- far enough in
    // +Z to cross the grid's drift-rebuild threshold (cell size = radius/4 = 0.5,
    // threshold = half a cell = 0.25) so a rebuild is genuinely exercised.
    for _ in 0..30 {
        session.apply_stroke(center_stroke(2.0, 1.0), BrushMode::Add);
    }
    let moved_position = session.position(center_index);
    assert!(
        moved_position.z > 0.3,
        "the vertex should have built past the grid's drift-rebuild threshold: {moved_position}"
    );

    // The real test: a dab on the SAME session, centered on the vertex's NEW
    // location. This only succeeds if the spatial grid tracked the drift -- a
    // grid indexed only by the pre-build positions would keep the vertex
    // bucketed near its stale original spot and silently miss it here.
    let outcome = session.apply_stroke(
        BrushStroke {
            center: moved_position.to_array(),
            radius_mm: 0.5,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        },
        BrushMode::Smooth,
    );
    assert!(
        outcome.touched_vertices.contains(&center_index),
        "a dab centered on the vertex's current (built-up) position must still find it"
    );
}

fn bbox_diagonal(mesh: &MeshEditBuffers) -> f32 {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for vertex in &mesh.vertices {
        let p = Vec3::from_array(vertex.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (hi - lo).length()
}

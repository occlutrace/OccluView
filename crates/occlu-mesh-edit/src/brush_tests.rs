//! Tests for the freeform sculpting brush kernel (split out of
//! `brush.rs` to hold the workspace's 800-line file budget).

use crate::brush::*;
use crate::{EditVertex, FaceSelection, MeshEditBuffers, MeshEditError, MeshTopology};
use glam::Vec3;

fn v(position: [f32; 3]) -> EditVertex {
    EditVertex::at(position)
}

/// A flat 11x11 grid (1mm spacing) with random-ish bump noise added to
/// interior vertices, sharing indexed topology (not soup) — the baseline
/// fixture for Smooth/Add/Remove/Drag tests.
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

fn center_stroke(radius_mm: f32, strength: f32) -> BrushStroke {
    BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm,
        strength,
    }
}

#[test]
fn smooth_reduces_height_variance_without_collapsing_the_patch() {
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
        after_variance < before_variance * 0.7,
        "smoothing should substantially flatten bumps: {before_variance} -> {after_variance}"
    );
    assert!(
            after_bbox > before_bbox * 0.8,
            "Taubin smoothing must not collapse the patch like a naive Laplacian: {before_bbox} -> {after_bbox}"
        );
    assert!(result.report.moved_vertices > 0);
}

#[test]
fn falloff_leaves_vertices_outside_the_radius_untouched() {
    let mesh = bumpy_patch(0.6);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    let outcome = session.apply_stroke(center_stroke(2.0, 1.0), BrushMode::Smooth);
    let result = session.finish();

    // Far corner vertex (index 0, at roughly (-5,-5,0)) is well outside a
    // 2mm-radius stroke centered at the origin.
    assert!(!outcome.touched_vertices.contains(&0));
    assert_eq!(
        result.mesh.vertices[0].position, mesh.vertices[0].position,
        "vertex outside the brush radius must not move"
    );
}

#[test]
fn add_bulges_outward_and_remove_bulges_inward() {
    let mesh = bumpy_patch(0.0); // perfectly flat patch
    let mut add_session = BrushSession::prepare(&mesh).expect("prepare");
    add_session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Add);
    let added = add_session.finish();
    let center_index = 5 * 11 + 5; // the exact center vertex
    assert!(
        added.mesh.vertices[center_index].position[2] > 0.01,
        "Add should push the surface outward along +normal"
    );

    let mut remove_session = BrushSession::prepare(&mesh).expect("prepare");
    remove_session.apply_stroke(center_stroke(4.0, 1.0), BrushMode::Remove);
    let removed = remove_session.finish();
    assert!(
        removed.mesh.vertices[center_index].position[2] < -0.01,
        "Remove should pull the surface inward along -normal"
    );
}

#[test]
fn drag_moves_touched_vertices_toward_the_delta_direction() {
    let mesh = bumpy_patch(0.0);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    session.apply_stroke(
        center_stroke(4.0, 1.0),
        BrushMode::Drag {
            delta: [1.5, 0.0, 0.0],
        },
    );
    let result = session.finish();
    let center_index = 5 * 11 + 5;
    let before_x = mesh.vertices[center_index].position[0];
    let after_x = result.mesh.vertices[center_index].position[0];
    assert!(
        after_x > before_x,
        "dragging with a +X delta should move the center vertex toward +X"
    );
}

#[test]
fn guard_prevents_triangle_inversion_under_a_large_add_stroke() {
    let mesh = bumpy_patch(0.0);
    let mut session = BrushSession::prepare(&mesh).expect("prepare");
    // Many repeated full-strength strokes: without the clamp this would
    // eventually invert the central fan of triangles.
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
            "guard must keep every triangle non-degenerate under repeated strokes"
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
    // Stroke centered on the shared edge, radius large enough to move it.
    session.apply_stroke(
        BrushStroke {
            center: [0.5, 0.0, 0.0],
            radius_mm: 5.0,
            strength: 1.0,
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

fn select_interior(mesh: &MeshEditBuffers, n: usize) -> FaceSelection {
    let mask: Vec<bool> = mesh
        .indices
        .chunks_exact(3)
        .map(|triangle| {
            triangle.iter().all(|&raw| {
                let idx = raw as usize;
                let (i, j) = (idx % n, idx / n);
                i > 1 && i < n - 2 && j > 1 && j < n - 2
            })
        })
        .collect();
    FaceSelection::new(mask)
}

#[test]
fn smooth_selected_faces_flattens_the_marked_region_and_blends_the_border() {
    let n = 11;
    let mesh = bumpy_patch(0.6);
    let selection = select_interior(&mesh, n);
    assert!(selection.selected_count() > 0);
    let before_variance = height_variance(&mesh, n);

    let result = smooth_selected_faces(&mesh, &selection).expect("smooth selection");

    let after_variance = height_variance(&result.mesh, n);
    assert!(
        after_variance < before_variance * 0.7,
        "one-click Smooth should substantially flatten the marked bumps: \
             {before_variance} -> {after_variance}"
    );
    assert!(result.report.moved_vertices > 0);
    // The margin means the falloff reaches slightly past the strict
    // interior selection too, so a boundary ring vertex may move a touch —
    // that softness IS the "blend the transition" contract. The far
    // corner of the patch, well outside the enclosing sphere, must not.
    assert_eq!(
        result.mesh.vertices[0].position, mesh.vertices[0].position,
        "the far corner, outside the padded enclosing sphere, must stay put"
    );
}

#[test]
fn smooth_selected_faces_is_a_no_op_on_an_empty_selection() {
    let mesh = bumpy_patch(0.6);
    let empty = FaceSelection::new(vec![false; mesh.triangle_count()]);
    let result = smooth_selected_faces(&mesh, &empty).expect("empty selection smooth");
    assert_eq!(result.report.moved_vertices, 0);
    for (before, after) in mesh.vertices.iter().zip(&result.mesh.vertices) {
        assert_eq!(before.position, after.position);
    }
}

#[test]
fn smooth_selected_faces_rejects_a_mismatched_selection_length() {
    let mesh = bumpy_patch(0.0);
    let wrong_length = FaceSelection::new(vec![true; mesh.triangle_count() + 1]);
    assert!(smooth_selected_faces(&mesh, &wrong_length).is_err());
}

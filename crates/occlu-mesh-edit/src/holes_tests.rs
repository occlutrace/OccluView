//! Hole-filling kernel tests for the dental-lab bug fixes: adjacent pinched
//! rims, lasso-majority selection gating, and the mm perimeter cap. Kept in
//! their own module because `tests.rs` is over the file-size budget.

use crate::{
    fill_holes, EditVertex, FaceSelection, MeshEditBuffers, MeshEditOptions, MeshTopology,
};

fn tri_mesh(vertices: Vec<EditVertex>, indices: Vec<u32>) -> MeshEditBuffers {
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// Count directed edges left without their reverse twin: 0 means watertight.
fn boundary_edge_count(indices: &[u32]) -> usize {
    let mut directed = std::collections::HashSet::new();
    for triangle in indices.chunks_exact(3) {
        for edge in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            directed.insert(edge);
        }
    }
    directed
        .iter()
        .filter(|(a, b)| !directed.contains(&(*b, *a)))
        .count()
}

/// Two square "bowls" (apex + 4-vertex rim, exocad hole shape) that share
/// EXACTLY one rim vertex (index 0). That shared vertex is a boundary junction:
/// its boundary in/out degree is 2, so the classic walk dead-ends at it and
/// BOTH rims stay open. This is the "closes random holes, leaves the neighbor"
/// bug reduced to its core.
fn two_bowls_sharing_a_pinch_vertex() -> MeshEditBuffers {
    let vertices = vec![
        EditVertex::at([0.0, 0.0, 0.0]),  // 0: P, the shared pinch vertex
        EditVertex::at([-1.0, 0.0, 0.0]), // 1: bowl A rim
        EditVertex::at([-1.0, 1.0, 0.0]), // 2: bowl A rim
        EditVertex::at([0.0, 1.0, 0.0]),  // 3: bowl A rim
        EditVertex::at([-0.5, 0.5, 1.0]), // 4: bowl A apex
        EditVertex::at([0.0, -1.0, 0.0]), // 5: bowl B rim
        EditVertex::at([1.0, -1.0, 0.0]), // 6: bowl B rim
        EditVertex::at([1.0, 0.0, 0.0]),  // 7: bowl B rim
        EditVertex::at([0.5, -0.5, 1.0]), // 8: bowl B apex
    ];
    // Bowl A: apex 4 over rim [0,1,2,3]; bowl B: apex 8 over rim [0,5,6,7].
    let indices = vec![
        4, 0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, // bowl A
        8, 0, 5, 8, 5, 6, 8, 6, 7, 8, 7, 0, // bowl B
    ];
    tri_mesh(vertices, indices)
}

/// Flat fan: a central apex over an `rim_len`-gon rim of the given radius. The
/// single boundary loop is the rim; triangle `i` owns rim edge `i`, so a face
/// selection maps one-to-one onto rim coverage.
fn fan_mesh(rim_len: usize, radius: f32) -> MeshEditBuffers {
    let mut vertices = vec![EditVertex::at([0.0, 0.0, 0.0])]; // apex = vertex 0
    for index in 0..rim_len {
        let theta = std::f32::consts::TAU * (index as f32) / (rim_len as f32);
        vertices.push(EditVertex::at([
            radius * theta.cos(),
            radius * theta.sin(),
            0.0,
        ]));
    }
    let mut indices = Vec::new();
    for index in 0..rim_len {
        let a = 1 + index as u32;
        let b = 1 + ((index + 1) % rim_len) as u32;
        indices.extend_from_slice(&[0, a, b]);
    }
    tri_mesh(vertices, indices)
}

/// A `true` mask over the first `count` faces of `face_count` total.
fn first_n_selected(face_count: usize, count: usize) -> FaceSelection {
    FaceSelection::new((0..face_count).map(|index| index < count).collect())
}

#[test]
fn adjacent_pinched_rims_both_close() {
    let mesh = two_bowls_sharing_a_pinch_vertex();

    // Direct evidence the junction is detected and split exactly once.
    let split = crate::pinch::split_boundary_pinch_vertices(&mesh).expect("pinch split");
    let (_, split_count) = split.expect("one pinch vertex split");
    assert_eq!(split_count, 1);

    // End-to-end: BOTH rims now close (previously the shared vertex dead-ended
    // the walk and neither did), and the result is watertight. The fixture is
    // a bare pair of bowls whose rims ARE its only boundary, so the scan
    // border guard is off — this test exercises the pinch machinery.
    let options = MeshEditOptions {
        protect_scan_border: false,
        ..MeshEditOptions::default()
    };
    let result = fill_holes(&mesh, None, options).expect("close adjacent holes");
    assert_eq!(result.report.filled_holes, 2);
    assert!(result.report.warnings.is_empty());
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

#[test]
fn lasso_majority_selection_closes_a_large_rim() {
    // 64-edge rim; a low edge cap would refuse it without a selection, but an
    // explicit selection is intent and lifts the ceiling so it closes.
    let mesh = fan_mesh(64, 1.0);
    let tight = MeshEditOptions {
        max_boundary_loop: 10,
        ..MeshEditOptions::default()
    };

    let unselected = fill_holes(&mesh, None, tight).expect("unselected large rim");
    assert_eq!(unselected.report.filled_holes, 0);
    assert_eq!(unselected.report.warnings.len(), 1); // one oversize skip

    let selection = first_n_selected(64, 64);
    let selected = fill_holes(&mesh, Some(&selection), tight).expect("selected large rim");
    assert_eq!(selected.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&selected.mesh.indices), 0);
}

#[test]
fn selection_qualifies_on_majority_rim_coverage() {
    let mesh = fan_mesh(16, 1.0);

    // 9/16 owning faces marked -> at least half -> the circled hole stitches.
    let majority = first_n_selected(16, 9);
    let closed =
        fill_holes(&mesh, Some(&majority), MeshEditOptions::default()).expect("majority selection");
    assert_eq!(closed.report.filled_holes, 1);

    // 7/16 marked -> below half -> left open, and NOT flagged as degeneracy.
    let minority = first_n_selected(16, 7);
    let open =
        fill_holes(&mesh, Some(&minority), MeshEditOptions::default()).expect("minority selection");
    assert_eq!(open.report.filled_holes, 0);
    assert!(open.report.warnings.is_empty());
}

#[test]
fn mm_perimeter_cap_gates_the_unselected_button() {
    // radius 2 -> perimeter ~12.5 mm; radius 3 -> ~18.7 mm.
    let within = fan_mesh(16, 2.0);
    let over = fan_mesh(16, 3.0);
    let capped = MeshEditOptions {
        max_rim_perimeter_mm: Some(15.0),
        // Lone-fan fixtures have no real border; this test gates on mm only.
        protect_scan_border: false,
        ..MeshEditOptions::default()
    };

    let closed = fill_holes(&within, None, capped).expect("within mm cap");
    assert_eq!(closed.report.filled_holes, 1);

    let skipped = fill_holes(&over, None, capped).expect("over mm cap");
    assert_eq!(skipped.report.filled_holes, 0);
    assert_eq!(skipped.report.warnings.len(), 1); // honest oversize skip
}

#[test]
fn selection_overrides_the_mm_perimeter_cap() {
    // A 15 mm-perimeter rim exceeds a 5 mm cap, but an explicit full selection
    // is intent and the mm cap is ignored for the selected path.
    let mesh = fan_mesh(16, 1.0); // perimeter ~6.2 mm
    let tiny_cap = MeshEditOptions {
        max_rim_perimeter_mm: Some(5.0),
        ..MeshEditOptions::default()
    };

    let button = fill_holes(&mesh, None, tiny_cap).expect("button over mm cap");
    assert_eq!(button.report.filled_holes, 0);

    let selection = first_n_selected(16, 16);
    let lasso = fill_holes(&mesh, Some(&selection), tiny_cap).expect("selection ignores mm cap");
    assert_eq!(lasso.report.filled_holes, 1);
}

#[test]
fn default_options_leave_mm_gating_off() {
    // No mm cap set (kernel/repair default): a rim closes purely on edge count
    // once the border guard is out of the picture, so callers that never opt
    // into the mm restraint keep unbounded interior fills.
    let mesh = fan_mesh(16, 3.0); // ~18.7 mm perimeter, no cap -> still closes
    assert_eq!(MeshEditOptions::default().max_rim_perimeter_mm, None);
    let options = MeshEditOptions {
        protect_scan_border: false,
        ..MeshEditOptions::default()
    };
    let result = fill_holes(&mesh, None, options).expect("no mm cap");
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(boundary_edge_count(&result.mesh.indices), 0);
}

#[test]
fn lone_triangle_rim_is_never_capped_with_its_reverse_twin() {
    // A single free triangle's boundary is a 3-edge rim whose only cap is the
    // triangle's own reverse twin — a zero-volume sliver that later cleanup
    // removes, so filling it would churn forever. It must be refused.
    let vertices = vec![
        EditVertex::at([0.0, 0.0, 0.0]),
        EditVertex::at([1.0, 0.0, 0.0]),
        EditVertex::at([0.0, 1.0, 0.0]),
    ];
    let mesh = tri_mesh(vertices, vec![0, 1, 2]);
    let result =
        fill_holes(&mesh, None, MeshEditOptions::default()).expect("fill runs on a lone triangle");
    assert_eq!(
        result.report.filled_holes, 0,
        "the reverse-twin cap must be refused"
    );
    assert_eq!(
        result.mesh.indices.len(),
        3,
        "no duplicate face may be emitted"
    );
}

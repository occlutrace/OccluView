use super::{closed_cube, closed_u_prism, request};
use crate::bridge_split::cap::cap_open_part;
use crate::bridge_split::rims::build_cut_loops;
use crate::{
    split_bridge, validate_bridge_split_part, BridgeSplitError, EditVertex, MeshEditBuffers,
    MeshTopology,
};
use glam::DVec3;

#[test]
fn branching_cut_graph_is_refused() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([0.0, 1.0, 0.0]),
            EditVertex::at([-1.0, 0.0, 0.0]),
        ],
        indices: Vec::new(),
        topology: MeshTopology::TriangleMesh,
    };
    let edges = [[0, 1], [1, 2], [2, 0], [0, 3], [3, 2]];

    assert!(matches!(
        build_cut_loops(&mesh, &edges),
        Err(BridgeSplitError::DamagedCutRim { .. })
    ));
}

#[test]
fn self_crossing_cut_loop_is_refused_without_partial_cap() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([-1.0, -1.0, 0.0]),
            EditVertex::at([1.0, 1.0, 0.0]),
            EditVertex::at([-1.0, 1.0, 0.0]),
            EditVertex::at([1.0, -1.0, 0.0]),
        ],
        indices: Vec::new(),
        topology: MeshTopology::TriangleMesh,
    };
    let edges = [[0, 1], [1, 2], [2, 3], [3, 0]];

    assert!(matches!(
        cap_open_part(mesh, &edges, DVec3::Z),
        Err(BridgeSplitError::CapFailed { .. })
    ));
}

#[test]
fn non_planar_cut_loop_is_refused() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([-1.0, -1.0, 0.0]),
            EditVertex::at([1.0, -1.0, 0.0]),
            EditVertex::at([1.0, 1.0, 0.02]),
            EditVertex::at([-1.0, 1.0, 0.0]),
        ],
        indices: Vec::new(),
        topology: MeshTopology::TriangleMesh,
    };
    let edges = [[0, 1], [1, 2], [2, 3], [3, 0]];

    assert!(matches!(
        cap_open_part(mesh, &edges, DVec3::Z),
        Err(BridgeSplitError::CapFailed { .. })
    ));
}

#[test]
fn side_with_multiple_physical_components_is_returned_as_one_logical_part() {
    let source = closed_u_prism();
    let original = source.clone();

    let result = split_bridge(&source, request()).expect("multi-shell logical side");

    assert_eq!(validate_bridge_split_part(&result.part_a), Ok(2));
    assert_eq!(validate_bridge_split_part(&result.part_b), Ok(1));
    assert_eq!(source, original);
}

#[test]
fn finite_disc_failure_leaves_source_untouched() {
    let source = closed_cube();
    let original = source.clone();
    let mut limited = request();
    limited.disc_radius_mm = 0.1;
    limited.max_disc_radius_mm = 0.1;

    assert!(matches!(
        split_bridge(&source, limited),
        Err(BridgeSplitError::DiscLimitExceeded { .. })
    ));
    assert_eq!(source, original);
}

#[test]
fn full_supported_kerf_range_produces_closed_parts() {
    for kerf_mm in [0.01, 1.0] {
        let mut split_request = request();
        split_request.kerf_mm = kerf_mm;
        let result = split_bridge(&closed_cube(), split_request).expect("supported kerf");
        assert_eq!(result.report.kerf_mm, kerf_mm);
        assert_eq!(result.report.part_a_cut_loops, 1);
        assert_eq!(result.report.part_b_cut_loops, 1);
    }
}

#[test]
fn unrepresentable_absolute_kerf_is_refused_instead_of_silently_collapsing() {
    let mut source = closed_cube();
    for vertex in &mut source.vertices {
        vertex.position[0] += 1_000_000.0;
    }
    let mut split_request = request();
    split_request.center.x = 1_000_000.0;

    assert!(matches!(
        split_bridge(&source, split_request),
        Err(BridgeSplitError::SeparationViolation { .. })
    ));
}

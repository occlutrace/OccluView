use super::{
    closed_cube, closed_l_prism, closed_torus, exploded_cube_with_payload_seams,
    exploded_cube_with_uniform_payload, hollow_cube_with_payload, request,
};
use crate::topology::{canonical_topology, TopologyWeldPolicy};
use crate::topology_analysis::{connected_components, edge_incidence, topology_defects};
use crate::{split_bridge, split_bridge_surface, BridgeSplitResult, MeshEditBuffers};
use glam::Vec3;

fn assert_closed_manifold(mesh: &MeshEditBuffers) {
    let canonical = canonical_topology(mesh, TopologyWeldPolicy::PositionOnly)
        .expect("canonical output topology");
    let incidence = edge_incidence(canonical.indices());
    let defects = topology_defects(canonical.indices(), &incidence);
    assert_eq!(defects.boundary_edges, 0);
    assert_eq!(defects.non_manifold_edges, 0);
    assert_eq!(defects.inconsistent_winding_edges, 0);
    assert_eq!(defects.non_manifold_vertices, 0);
    assert_eq!(
        connected_components(canonical.indices(), &incidence)
            .members
            .len(),
        1
    );
}

fn assert_result_manufacturable(result: &BridgeSplitResult) {
    assert_closed_manifold(&result.part_a);
    assert_closed_manifold(&result.part_b);
    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    for mesh in [&result.part_a, &result.part_b] {
        for face in mesh.indices.chunks_exact(3) {
            assert_ne!(face[0], face[1]);
            assert_ne!(face[1], face[2]);
            assert_ne!(face[2], face[0]);
            let a = Vec3::from_array(mesh.vertices[face[0] as usize].position);
            let b = Vec3::from_array(mesh.vertices[face[1] as usize].position);
            let c = Vec3::from_array(mesh.vertices[face[2] as usize].position);
            assert!((b - a).cross(c - a).length_squared() > 0.0);
        }
    }
}

#[test]
fn cube_produces_two_closed_manifold_parts_with_requested_gap() {
    let result = split_bridge(&closed_cube(), request()).expect("cube splits and caps");
    assert_result_manufacturable(&result);
    let positive_min = result
        .part_a
        .vertices
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::INFINITY, f32::min);
    let negative_max = result
        .part_b
        .vertices
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((positive_min - negative_max - 0.05).abs() <= 2.0e-6);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn opposite_caps_face_outward() {
    let result = split_bridge(&closed_cube(), request()).expect("cube splits and caps");
    let cap_normal_x = |mesh: &MeshEditBuffers, boundary_x: f32| -> Vec<f32> {
        mesh.indices
            .chunks_exact(3)
            .filter_map(|face| {
                let points = [
                    Vec3::from_array(mesh.vertices[face[0] as usize].position),
                    Vec3::from_array(mesh.vertices[face[1] as usize].position),
                    Vec3::from_array(mesh.vertices[face[2] as usize].position),
                ];
                points
                    .iter()
                    .all(|point| (point.x - boundary_x).abs() <= 1.0e-6)
                    .then(|| (points[1] - points[0]).cross(points[2] - points[0]).x)
            })
            .collect()
    };
    let positive_cap = cap_normal_x(&result.part_a, 0.025);
    let negative_cap = cap_normal_x(&result.part_b, -0.025);
    assert!(!positive_cap.is_empty() && positive_cap.iter().all(|normal| *normal < 0.0));
    assert!(!negative_cap.is_empty() && negative_cap.iter().all(|normal| *normal > 0.0));
}

#[test]
fn uniform_stl_soup_matches_indexed_manifold_behavior() {
    let result = split_bridge(&exploded_cube_with_uniform_payload(), request())
        .expect("soup splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn payload_seam_soup_succeeds_end_to_end() {
    let result = split_bridge(&exploded_cube_with_payload_seams(), request())
        .expect("payload-seamed soup splits and caps");
    assert_result_manufacturable(&result);
}

#[test]
fn surface_split_caps_nested_connector_surfaces_as_a_planar_ring() {
    let result = split_bridge_surface(&hollow_cube_with_payload(), request())
        .expect("nested surface shells split");

    for part in [&result.part_a, &result.part_b] {
        let canonical = canonical_topology(part, TopologyWeldPolicy::PositionOnly)
            .expect("canonical output topology");
        let incidence = edge_incidence(canonical.indices());
        let defects = topology_defects(canonical.indices(), &incidence);
        assert_eq!(defects.boundary_edges, 0, "surface cap left a boundary");
        assert_eq!(defects.non_manifold_edges, 0);
        assert_eq!(defects.inconsistent_winding_edges, 0);
        assert_eq!(
            connected_components(canonical.indices(), &incidence)
                .members
                .len(),
            1
        );
        assert!(part
            .vertices
            .iter()
            .all(|vertex| vertex.position[0].abs() >= 0.024_999));
    }
    assert_eq!(result.report.part_a_cut_loops, 2);
    assert_eq!(result.report.part_b_cut_loops, 2);
}

#[test]
fn near_plane_source_vertex_caps_without_a_split_rim() {
    let mut mesh = closed_cube();
    mesh.vertices[1].position[0] = 0.025_001;
    let result = split_bridge(&mesh, request()).expect("snapped vertex remains a closed rim");
    assert_result_manufacturable(&result);
}

#[test]
fn every_simple_cut_loop_is_capped_atomically() {
    let result = split_bridge(&closed_torus(32, 16), request()).expect("torus splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 2);
    assert_eq!(result.report.part_b_cut_loops, 2);
}

#[test]
fn concave_planar_connector_loop_caps_completely() {
    let result = split_bridge(&closed_l_prism(), request()).expect("concave prism splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn open_surface_split_preserves_natural_source_borders() {
    let mut source = closed_cube();
    source.indices.drain(0..3);

    let result = split_bridge_surface(&source, request()).expect("open surface clips");

    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    assert!(!result.report.parts_closed);
    assert_eq!(
        result.report.part_a_triangles,
        result.part_a.triangle_count()
    );
    assert_eq!(
        result.report.part_b_triangles,
        result.part_b.triangle_count()
    );
}

#[test]
fn complete_split_is_deterministic() {
    let mesh = closed_torus(20, 10);
    let first = split_bridge(&mesh, request()).expect("first split");
    let second = split_bridge(&mesh, request()).expect("second split");
    assert_eq!(first, second);
}

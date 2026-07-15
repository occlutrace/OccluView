use super::{
    closed_cube, closed_tetrahedra_sharing_only_one_vertex, disconnected_cubes,
    exploded_cube_with_payload_seams, point_touching_cubes_with_separate_topology, request,
};
use crate::{
    validate_bridge_split, validate_bridge_split_part, BridgeSplitError, EditVertex,
    MeshEditBuffers, MeshEditError, MeshTopology,
};
use glam::Vec3;

#[test]
fn point_cloud_is_rejected() {
    let mesh = MeshEditBuffers {
        vertices: vec![EditVertex::at([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };

    assert_eq!(
        validate_bridge_split(&mesh, request()).unwrap_err(),
        BridgeSplitError::Mesh(MeshEditError::UnsupportedPointCloud)
    );
}

#[test]
fn malformed_indices_are_rejected() {
    let mut mesh = closed_cube();
    mesh.indices.pop();

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::Mesh(MeshEditError::MalformedMesh { .. }))
    ));
}

#[test]
fn out_of_range_index_is_rejected() {
    let mut mesh = closed_cube();
    mesh.indices[0] = u32::try_from(mesh.vertices.len()).expect("small fixture");

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::Mesh(MeshEditError::MalformedMesh { .. }))
    ));
}

#[test]
fn non_finite_vertex_payload_is_rejected() {
    let mut mesh = closed_cube();
    mesh.vertices[0].position[1] = f32::NAN;

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::Mesh(MeshEditError::MalformedMesh { .. }))
    ));
}

#[test]
fn non_finite_normal_or_uv_is_rejected() {
    let mut bad_normal = closed_cube();
    bad_normal.vertices[2].normal[0] = f32::INFINITY;
    assert!(matches!(
        validate_bridge_split(&bad_normal, request()),
        Err(BridgeSplitError::Mesh(MeshEditError::MalformedMesh { .. }))
    ));

    let mut bad_uv = closed_cube();
    bad_uv.vertices[3].uv[1] = f32::NAN;
    assert!(matches!(
        validate_bridge_split(&bad_uv, request()),
        Err(BridgeSplitError::Mesh(MeshEditError::MalformedMesh { .. }))
    ));
}

#[test]
fn invalid_plane_normal_is_rejected() {
    for normal in [Vec3::ZERO, Vec3::new(f32::INFINITY, 0.0, 0.0)] {
        let mut invalid = request();
        invalid.normal = normal;
        assert!(matches!(
            validate_bridge_split(&closed_cube(), invalid),
            Err(BridgeSplitError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn invalid_center_is_rejected() {
    let mut invalid = request();
    invalid.center = Vec3::new(0.0, f32::NAN, 0.0);
    assert!(matches!(
        validate_bridge_split(&closed_cube(), invalid),
        Err(BridgeSplitError::InvalidRequest { .. })
    ));
}

#[test]
fn invalid_kerf_is_rejected() {
    for kerf_mm in [0.0, -0.01, f32::NAN, f32::INFINITY] {
        let mut invalid = request();
        invalid.kerf_mm = kerf_mm;
        assert!(matches!(
            validate_bridge_split(&closed_cube(), invalid),
            Err(BridgeSplitError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn invalid_disc_radius_is_rejected() {
    for max_disc_radius_mm in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut invalid = request();
        invalid.max_disc_radius_mm = max_disc_radius_mm;
        assert!(matches!(
            validate_bridge_split(&closed_cube(), invalid),
            Err(BridgeSplitError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn invalid_selected_disc_radius_is_rejected() {
    for disc_radius_mm in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut invalid = request();
        invalid.disc_radius_mm = disc_radius_mm;
        assert!(matches!(
            validate_bridge_split(&closed_cube(), invalid),
            Err(BridgeSplitError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn empty_triangle_mesh_is_rejected() {
    let mesh = MeshEditBuffers::default();
    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::EmptyInput)
    ));
}

#[test]
fn open_mesh_is_rejected() {
    let mut mesh = closed_cube();
    mesh.indices.truncate(mesh.indices.len() - 3);

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::OpenOrNonManifold {
            boundary_edges,
            non_manifold_edges: 0,
            ..
        }) if boundary_edges > 0
    ));
}

#[test]
fn non_manifold_edge_is_rejected() {
    let mut mesh = closed_cube();
    let tip = u32::try_from(mesh.vertices.len()).expect("small fixture");
    mesh.vertices.push(EditVertex::at([0.0, -2.0, 0.0]));
    mesh.indices.extend([0, 1, tip]);

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::OpenOrNonManifold {
            non_manifold_edges,
            ..
        }) if non_manifold_edges > 0
    ));
}

#[test]
fn face_collapsed_by_geometric_seam_recovery_is_rejected() {
    let mut mesh = closed_cube();
    mesh.vertices[1].position = mesh.vertices[0].position;

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::DegenerateInput { faces }) if faces > 0
    ));
}

#[test]
fn inconsistent_winding_is_rejected() {
    let mut mesh = closed_cube();
    mesh.indices.swap(0, 1);

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::OpenOrNonManifold {
            inconsistent_winding_edges,
            ..
        }) if inconsistent_winding_edges > 0
    ));
}

#[test]
fn disconnected_target_is_rejected() {
    assert_eq!(
        validate_bridge_split(&disconnected_cubes(), request()).unwrap_err(),
        BridgeSplitError::DisconnectedInput { components: 2 }
    );
}

#[test]
fn disconnected_closed_output_part_is_validated_component_wise() {
    assert_eq!(validate_bridge_split_part(&disconnected_cubes()), Ok(2));
}

#[test]
fn separate_closed_output_components_are_not_welded_at_one_touching_point() {
    assert_eq!(
        validate_bridge_split_part(&point_touching_cubes_with_separate_topology()),
        Ok(2)
    );
}

#[test]
fn closed_stl_soup_passes_geometric_preflight() {
    let mesh = exploded_cube_with_payload_seams();
    validate_bridge_split(&mesh, request()).expect("closed soup is valid");
}

#[test]
fn geometric_preflight_does_not_mutate_payload_seams() {
    let mesh = exploded_cube_with_payload_seams();
    let original = mesh.clone();

    validate_bridge_split(&mesh, request()).expect("payload seams are geometrically closed");

    assert_eq!(mesh, original);
}

#[test]
fn bowtie_vertex_is_rejected_even_when_every_edge_has_two_faces() {
    assert!(matches!(
        validate_bridge_split(&closed_tetrahedra_sharing_only_one_vertex(), request()),
        Err(BridgeSplitError::OpenOrNonManifold {
            boundary_edges: 0,
            non_manifold_edges: 0,
            non_manifold_vertices: 1,
            ..
        })
    ));
}

#[test]
fn signed_zero_position_seams_are_geometrically_identical() {
    let mut mesh = exploded_cube_with_payload_seams();
    for vertex in &mut mesh.vertices {
        vertex.position[0] += 1.0;
    }
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        if vertex.position[0] == 0.0 && index % 2 == 0 {
            vertex.position[0] = -0.0;
        }
    }

    validate_bridge_split(&mesh, request()).expect("signed zero is the same position");
}

#[test]
fn near_coincident_payload_seam_is_not_tolerance_welded() {
    let mut mesh = exploded_cube_with_payload_seams();
    mesh.vertices[0].position[0] += 1.0e-6;

    assert!(matches!(
        validate_bridge_split(&mesh, request()),
        Err(BridgeSplitError::OpenOrNonManifold { boundary_edges, .. }) if boundary_edges > 0
    ));
}

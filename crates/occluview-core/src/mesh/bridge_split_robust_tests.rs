use super::{
    bridge_split_mesh_in_world, bridge_split_prepared_mesh_in_world, mesh_edit_buffers_from_mesh,
    normalize_bridge_split_input, prepare_bridge_split_source, CoreBridgeSplitError,
    CoreBridgeSplitResult, Mesh, PreparedBridgeSplitSource, Vertex,
};
use glam::{Affine3A, Vec3};
use occlu_mesh_edit::{BridgeSplitError, BridgeSplitRequest};
use std::sync::Arc;

fn cube_with_precision_sliver() -> Mesh {
    let positions = [
        [0.0, -1.0, -1.0],
        [2.0, -1.0, -1.0],
        [2.0, 1.0, -1.0],
        [0.0, 1.0, -1.0],
        [0.0, -1.0, 1.0],
        [2.0, -1.0, 1.0],
        [2.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0e-8, -1.0, -1.0],
    ];
    let vertices = positions
        .into_iter()
        .map(|position| Vertex::at(position.into()))
        .collect();
    let indices = vec![
        0, 2, 8, 8, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 8, 5, 8, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2,
        0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
    ];
    Mesh::new(Some("Precision sliver".to_string()), vertices, indices).expect("sliver cube")
}

fn append_box(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, min: Vec3, max: Vec3) {
    let base = u32::try_from(vertices.len()).expect("small fixture");
    vertices.extend([
        Vertex::at(Vec3::new(min.x, min.y, min.z)),
        Vertex::at(Vec3::new(max.x, min.y, min.z)),
        Vertex::at(Vec3::new(max.x, max.y, min.z)),
        Vertex::at(Vec3::new(min.x, max.y, min.z)),
        Vertex::at(Vec3::new(min.x, min.y, max.z)),
        Vertex::at(Vec3::new(max.x, min.y, max.z)),
        Vertex::at(Vec3::new(max.x, max.y, max.z)),
        Vertex::at(Vec3::new(min.x, max.y, max.z)),
    ]);
    indices.extend([
        base,
        base + 2,
        base + 1,
        base,
        base + 3,
        base + 2,
        base + 4,
        base + 5,
        base + 6,
        base + 4,
        base + 6,
        base + 7,
        base,
        base + 1,
        base + 5,
        base,
        base + 5,
        base + 4,
        base + 3,
        base + 7,
        base + 6,
        base + 3,
        base + 6,
        base + 2,
        base,
        base + 4,
        base + 7,
        base,
        base + 7,
        base + 3,
        base + 1,
        base + 2,
        base + 6,
        base + 1,
        base + 6,
        base + 5,
    ]);
}

fn overlapping_shell_bridge() -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    append_box(
        &mut vertices,
        &mut indices,
        Vec3::new(-2.0, -1.0, -1.0),
        Vec3::new(0.8, 1.0, 1.0),
    );
    append_box(
        &mut vertices,
        &mut indices,
        Vec3::new(-0.8, -1.0, -1.0),
        Vec3::new(2.0, 1.0, 1.0),
    );
    append_box(
        &mut vertices,
        &mut indices,
        Vec3::new(3.0, -0.15, -0.15),
        Vec3::new(3.2, 0.15, 0.15),
    );
    Mesh::new(
        Some("Overlapping bridge shells".to_string()),
        vertices,
        indices,
    )
    .expect("closed multi-shell fixture")
}

fn extruded_u_bridge() -> Mesh {
    let outline = [
        [-10.0, -10.0],
        [10.0, -10.0],
        [10.0, 10.0],
        [6.0, 10.0],
        [6.0, -6.0],
        [-6.0, -6.0],
        [-6.0, 10.0],
        [-10.0, 10.0],
    ];
    let mut vertices = Vec::with_capacity(outline.len() * 2);
    for z in [-1.0, 1.0] {
        vertices.extend(outline.iter().map(|&[x, y]| Vertex::at(Vec3::new(x, y, z))));
    }
    let faces = [
        [1, 2, 3],
        [1, 3, 4],
        [0, 1, 4],
        [0, 4, 5],
        [0, 5, 6],
        [0, 6, 7],
    ];
    let count = u32::try_from(outline.len()).expect("small fixture");
    let mut indices = Vec::new();
    for [a, b, c] in faces {
        indices.extend([c, b, a, a + count, b + count, c + count]);
    }
    for edge in 0..outline.len() {
        let next = (edge + 1) % outline.len();
        let a = u32::try_from(edge).expect("small fixture");
        let b = u32::try_from(next).expect("small fixture");
        indices.extend([a, b, b + count, a, b + count, a + count]);
    }
    Mesh::new(Some("U bridge".to_string()), vertices, indices).expect("closed U bridge")
}

fn split_request(center: Vec3, normal: Vec3, radius_mm: f32) -> BridgeSplitRequest {
    BridgeSplitRequest {
        center,
        normal,
        kerf_mm: 0.05,
        disc_radius_mm: radius_mm,
        max_disc_radius_mm: radius_mm,
    }
}

fn world_gap(
    result: &CoreBridgeSplitResult,
    transform: Affine3A,
    request: BridgeSplitRequest,
) -> f32 {
    let normal = request.normal.normalize();
    let positive_min = result
        .part_a
        .vertices()
        .iter()
        .map(|vertex| {
            (transform.transform_point3(Vec3::from_array(vertex.position)) - request.center)
                .dot(normal)
        })
        .fold(f32::INFINITY, f32::min);
    let negative_max = result
        .part_b
        .vertices()
        .iter()
        .map(|vertex| {
            (transform.transform_point3(Vec3::from_array(vertex.position)) - request.center)
                .dot(normal)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    positive_min - negative_max
}

#[test]
fn robust_fallback_handles_centering_induced_f32_collapse() {
    let source = cube_with_precision_sliver();
    assert!(normalize_bridge_split_input(&source)
        .expect("valid before centering")
        .is_none());
    let transform = Affine3A::from_translation(Vec3::new(1.1, 0.0, 0.0));
    let request = split_request(Vec3::new(2.1, 0.0, 0.0), Vec3::X, 4.0);

    let result = bridge_split_mesh_in_world(&source, transform, request)
        .expect("robust split after f32 collapse");

    assert!((world_gap(&result, transform, request) - request.kerf_mm).abs() < 1e-4);
}

#[test]
fn robust_split_unions_overlaps_and_preserves_logical_side_components() {
    let source = overlapping_shell_bridge();
    let source_vertices = source.vertices().to_vec();
    let source_indices = source.indices().to_vec();
    let request = split_request(Vec3::ZERO, Vec3::X, 4.0);

    let result = bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request)
        .expect("overlapping dental shells split");

    assert_eq!(
        occlu_mesh_edit::validate_bridge_split_part(&mesh_edit_buffers_from_mesh(&result.part_a)),
        Ok(2)
    );
    assert_eq!(
        occlu_mesh_edit::validate_bridge_split_part(&mesh_edit_buffers_from_mesh(&result.part_b)),
        Ok(1)
    );
    assert!((world_gap(&result, Affine3A::IDENTITY, request) - request.kerf_mm).abs() < 1e-4);
    assert_eq!(source.vertices(), source_vertices);
    assert_eq!(source.indices(), source_indices);
}

#[test]
fn prepared_overlap_source_is_thread_safe_and_reused() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PreparedBridgeSplitSource>();

    let source = Arc::new(overlapping_shell_bridge());
    let prepared = prepare_bridge_split_source(Arc::clone(&source)).expect("prepared source");
    let request = split_request(Vec3::ZERO, Vec3::X, 4.0);
    let first = bridge_split_prepared_mesh_in_world(&prepared, Affine3A::IDENTITY, request)
        .expect("first split");
    let second = bridge_split_prepared_mesh_in_world(&prepared, Affine3A::IDENTITY, request)
        .expect("second split");

    assert_eq!(first.part_a.vertices(), second.part_a.vertices());
    assert_eq!(first.part_a.indices(), second.part_a.indices());
    assert_eq!(first.part_b.vertices(), second.part_b.vertices());
    assert_eq!(first.part_b.indices(), second.part_b.indices());
    assert_eq!(source.name(), Some("Overlapping bridge shells"));
}

#[test]
fn finite_disc_ignores_remote_arch_crossings() {
    let source = extruded_u_bridge();
    let request = split_request(Vec3::new(8.0, 0.0, 0.0), Vec3::Y, 3.0);

    let result = bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request)
        .expect("finite disc cuts only the near arm");

    assert_eq!(result.report.disc_radius_mm.to_bits(), 3.0_f32.to_bits());
    let remote_arm_was_cut = [&result.part_a, &result.part_b]
        .into_iter()
        .flat_map(Mesh::vertices)
        .any(|vertex| vertex.position[0] <= -6.0 && vertex.position[1].abs() < 0.1);
    assert!(!remote_arm_was_cut);
}

#[test]
fn finite_disc_preserves_a_disconnected_remote_plane_crossing() {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    append_box(
        &mut vertices,
        &mut indices,
        Vec3::new(-2.0, -0.5, -0.5),
        Vec3::new(2.0, 0.5, 0.5),
    );
    append_box(
        &mut vertices,
        &mut indices,
        Vec3::new(-1.0, 3.0, -0.5),
        Vec3::new(1.0, 4.0, 0.5),
    );
    let source = Mesh::new(Some("Remote shell".to_string()), vertices, indices)
        .expect("closed multi-shell fixture");
    let request = split_request(Vec3::ZERO, Vec3::X, 1.5);

    let result = bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request)
        .expect("remote plane crossing remains outside the finite cut");

    let remote_vertices = [&result.part_a, &result.part_b]
        .into_iter()
        .flat_map(Mesh::vertices)
        .filter(|vertex| vertex.position[1] >= 3.0)
        .count();
    assert_eq!(remote_vertices, 8);
}

#[test]
fn robust_path_enforces_the_selected_disc_safety_ceiling() {
    let source = overlapping_shell_bridge();
    let mut request = split_request(Vec3::ZERO, Vec3::X, 4.0);
    request.max_disc_radius_mm = 0.1;

    let error = bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request)
        .expect_err("disc radius above the safety ceiling");

    assert!(matches!(
        error,
        CoreBridgeSplitError::Kernel(BridgeSplitError::InvalidRequest { .. })
    ));
}

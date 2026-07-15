use super::{
    bridge_split_mesh_in_world, normalize_bridge_split_input, CoreBridgeSplitError, Mesh,
    MeshTexture, Vertex,
};
use glam::{Affine3A, Mat3, Quat, Vec3};
use occlu_mesh_edit::{BridgeSplitError, BridgeSplitRequest};

fn textured_cube() -> Mesh {
    let positions = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let vertices = positions
        .into_iter()
        .enumerate()
        .map(|(index, position)| Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            color: [index as u8 * 20, 100, 180, 255],
            uv: [index as f32 / 8.0, 1.0 - index as f32 / 8.0],
        })
        .collect();
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let mut mesh = Mesh::new(Some("Bridge".to_string()), vertices, indices).expect("cube");
    mesh.set_texture(MeshTexture::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]));
    mesh
}

fn translated_cube(x_offset: f32) -> Mesh {
    let source = textured_cube();
    let vertices = source
        .vertices()
        .iter()
        .copied()
        .map(|mut vertex| {
            vertex.position[0] += x_offset;
            vertex
        })
        .collect();
    Mesh::new(
        source.name().map(str::to_owned),
        vertices,
        source.indices().to_vec(),
    )
    .expect("translated cube")
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

    // The two face fans triangulate the concave U without crossing its opening.
    let face_triangles = [
        [1, 2, 3],
        [1, 3, 4],
        [0, 1, 4],
        [0, 4, 5],
        [0, 5, 6],
        [0, 6, 7],
    ];
    let n = u32::try_from(outline.len()).expect("small fixture");
    let mut indices = Vec::new();
    for [a, b, c] in face_triangles {
        indices.extend([c, b, a]);
        indices.extend([a + n, b + n, c + n]);
    }
    for edge in 0..outline.len() {
        let next = (edge + 1) % outline.len();
        let a = u32::try_from(edge).expect("small fixture");
        let b = u32::try_from(next).expect("small fixture");
        indices.extend([a, b, b + n, a, b + n, a + n]);
    }
    Mesh::new(Some("U bridge".to_string()), vertices, indices).expect("closed U bridge")
}

fn colored_extruded_u_bridge() -> Mesh {
    let source = extruded_u_bridge();
    let mut vertices = source.vertices().to_vec();
    vertices[0].color = [180, 160, 120, 255];
    Mesh::new(
        source.name().map(str::to_owned),
        vertices,
        source.indices().to_vec(),
    )
    .expect("colored closed U bridge")
}

fn request(center: Vec3, normal: Vec3) -> BridgeSplitRequest {
    BridgeSplitRequest {
        center,
        normal,
        kerf_mm: 0.05,
        disc_radius_mm: 60.0,
        max_disc_radius_mm: 60.0,
    }
}

fn split_after_bridge_normalization(source: &Mesh) -> super::CoreBridgeSplitResult {
    let normalized = normalize_bridge_split_input(source).expect("normalizable bridge input");
    bridge_split_mesh_in_world(
        normalized.as_ref().unwrap_or(source),
        Affine3A::IDENTITY,
        request(Vec3::ZERO, Vec3::X),
    )
    .expect("split normalized bridge")
}

fn world_gap(
    result: &super::CoreBridgeSplitResult,
    transform: Affine3A,
    center: Vec3,
    normal: Vec3,
) -> f32 {
    let normal = normal.normalize();
    let positive_min = result
        .part_a
        .vertices()
        .iter()
        .map(|vertex| {
            (transform.transform_point3(Vec3::from_array(vertex.position)) - center).dot(normal)
        })
        .fold(f32::INFINITY, f32::min);
    let negative_max = result
        .part_b
        .vertices()
        .iter()
        .map(|vertex| {
            (transform.transform_point3(Vec3::from_array(vertex.position)) - center).dot(normal)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    positive_min - negative_max
}

fn assert_winding_matches_normals(mesh: &Mesh) {
    for face in mesh.indices().chunks_exact(3) {
        let vertices = [
            mesh.vertices()[face[0] as usize],
            mesh.vertices()[face[1] as usize],
            mesh.vertices()[face[2] as usize],
        ];
        let a = Vec3::from_array(vertices[0].position);
        let b = Vec3::from_array(vertices[1].position);
        let c = Vec3::from_array(vertices[2].position);
        let geometric = (b - a).cross(c - a);
        let average = vertices
            .iter()
            .map(|vertex| Vec3::from_array(vertex.normal))
            .sum::<Vec3>();
        assert!(geometric.dot(average) > 0.0);
    }
}

#[test]
fn identity_split_preserves_world_kerf() {
    let source = textured_cube();
    let result =
        bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::X))
            .expect("identity split");

    assert!((world_gap(&result, Affine3A::IDENTITY, Vec3::ZERO, Vec3::X) - 0.05).abs() < 1e-5);
}

#[test]
fn world_adapter_keeps_multiple_closed_components_in_one_logical_part() {
    let source = colored_extruded_u_bridge();
    let result =
        bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::Y))
            .expect("world adapter must preserve a valid multi-component side");

    assert_eq!(
        occlu_mesh_edit::validate_bridge_split_part(&super::mesh_edit_buffers_from_mesh(
            &result.part_a
        )),
        Ok(2)
    );
    assert_eq!(
        occlu_mesh_edit::validate_bridge_split_part(&super::mesh_edit_buffers_from_mesh(
            &result.part_b
        )),
        Ok(1)
    );
}

#[test]
fn large_translation_is_recentered_before_f32_kernel_storage() {
    let source = textured_cube();
    let transform = Affine3A::from_translation(Vec3::new(1_000_000.0, -2_000_000.0, 3.0));
    let center = transform.transform_point3(Vec3::ZERO);
    let result = bridge_split_mesh_in_world(&source, transform, request(center, Vec3::X))
        .expect("translated split");

    assert!((world_gap(&result, transform, center, Vec3::X) - 0.05).abs() <= 0.125);
    let local_positive_min = result
        .part_a
        .vertices()
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::INFINITY, f32::min);
    assert!((local_positive_min - 0.025).abs() < 1e-4);
}

#[test]
fn rotation_and_nonuniform_scale_keep_world_space_kerf() {
    let source = textured_cube();
    let rotation = Quat::from_rotation_z(0.63) * Quat::from_rotation_y(-0.27);
    let transform = Affine3A::from_scale_rotation_translation(
        Vec3::new(2.0, 0.6, 1.4),
        rotation,
        Vec3::new(12.0, -4.0, 7.0),
    );
    let center = transform.transform_point3(Vec3::ZERO);
    let normal = transform.transform_vector3(Vec3::X).normalize();
    let result = bridge_split_mesh_in_world(&source, transform, request(center, normal))
        .expect("affine split");

    assert!((world_gap(&result, transform, center, normal) - 0.05).abs() < 1e-4);
}

#[test]
fn arbitrary_invertible_shear_keeps_world_space_kerf() {
    let source = textured_cube();
    let matrix = Mat3::from_cols(
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.4, 1.0, 0.0),
        Vec3::new(0.2, 0.1, 1.0),
    );
    let translation = Vec3::new(5.0, -3.0, 2.0);
    let transform = Affine3A::from_mat3_translation(matrix, translation);
    let result = bridge_split_mesh_in_world(&source, transform, request(translation, Vec3::X))
        .expect("sheared split");

    assert!((world_gap(&result, transform, translation, Vec3::X) - 0.05).abs() < 1e-4);
}

#[test]
fn large_well_conditioned_uniform_scale_does_not_degenerate_normals() {
    let source = textured_cube();
    let transform = Affine3A::from_scale(Vec3::splat(100_000_000.0));
    let split_request = BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: Vec3::X,
        kerf_mm: 32.0,
        disc_radius_mm: 300_000_000.0,
        max_disc_radius_mm: 300_000_000.0,
    };
    let result = bridge_split_mesh_in_world(&source, transform, split_request)
        .expect("large uniform scale split");

    assert!((world_gap(&result, transform, Vec3::ZERO, Vec3::X) - 32.0).abs() <= 8.0);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
}

#[test]
fn small_well_conditioned_uniform_scale_is_not_misclassified_as_singular() {
    let source = textured_cube();
    let transform = Affine3A::from_scale(Vec3::splat(1.0e-5));
    let split_request = BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: Vec3::X,
        kerf_mm: 1.0e-6,
        disc_radius_mm: 1.0,
        max_disc_radius_mm: 60.0,
    };

    let result = bridge_split_mesh_in_world(&source, transform, split_request)
        .expect("well-conditioned small uniform scale split");

    assert!(
        (world_gap(&result, transform, Vec3::ZERO, Vec3::X) - 1.0e-6).abs() <= 1.0e-10,
        "the restored world-space kerf must survive the small, valid transform"
    );
}

#[test]
fn large_local_coordinates_refuse_a_split_when_f32_cannot_preserve_the_gap() {
    let source = translated_cube(1_000_000.0);

    assert!(matches!(
        bridge_split_mesh_in_world(
            &source,
            Affine3A::IDENTITY,
            request(Vec3::new(1_000_000.0, 0.0, 0.0), Vec3::X),
        ),
        Err(CoreBridgeSplitError::Conversion { .. })
    ));
}

#[test]
fn restored_f32_rounding_can_use_a_bounded_coordinate_resolution_allowance() {
    let source = translated_cube(10_000.0);
    let result = bridge_split_mesh_in_world(
        &source,
        Affine3A::IDENTITY,
        request(Vec3::new(10_000.0, 0.0, 0.0), Vec3::X),
    )
    .expect("a representable local split must not fail only for f32 rounding");

    let observed_gap = world_gap(
        &result,
        Affine3A::IDENTITY,
        Vec3::new(10_000.0, 0.0, 0.0),
        Vec3::X,
    );
    assert!(
        observed_gap >= 0.0495,
        "gap must remain within 1% of nominal"
    );
}

#[test]
fn invertible_reflection_is_supported_without_flipping_output_orientation() {
    let source = textured_cube();
    let transform = Affine3A::from_scale(Vec3::new(-1.5, 0.8, 1.2));
    let normal = transform.transform_vector3(Vec3::X).normalize();
    let result = bridge_split_mesh_in_world(&source, transform, request(Vec3::ZERO, normal))
        .expect("reflected split");

    assert!((world_gap(&result, transform, Vec3::ZERO, normal) - 0.05).abs() < 1e-4);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
}

#[test]
fn singular_and_non_finite_transforms_are_rejected() {
    let source = textured_cube();
    for transform in [
        Affine3A::from_scale(Vec3::new(1.0, 0.0, 1.0)),
        Affine3A::from_scale(Vec3::new(1.0, 1.0e-9, 1.0)),
        Affine3A::from_translation(Vec3::new(f32::NAN, 0.0, 0.0)),
    ] {
        assert!(matches!(
            bridge_split_mesh_in_world(&source, transform, request(Vec3::ZERO, Vec3::X)),
            Err(CoreBridgeSplitError::InvalidTransform { .. })
        ));
    }
}

#[test]
fn names_texture_colors_and_uvs_are_preserved_for_both_parts() {
    let source = textured_cube();
    let result =
        bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::X))
            .expect("split");

    assert_eq!(result.part_a.name(), Some("Bridge - Part A"));
    assert_eq!(result.part_b.name(), Some("Bridge - Part B"));
    for part in [&result.part_a, &result.part_b] {
        assert!(part.has_vertex_colors());
        assert!(part.has_uvs());
        let texture = part.texture().expect("texture preserved");
        assert_eq!((texture.width, texture.height), (2, 1));
        assert_eq!(texture.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }
}

#[test]
fn explicit_texture_is_preserved_even_when_every_uv_is_zero() {
    let source = textured_cube();
    let vertices = source
        .vertices()
        .iter()
        .map(|vertex| Vertex {
            uv: [0.0, 0.0],
            ..*vertex
        })
        .collect();
    let mut zero_uv = Mesh::new(
        Some("Zero UV Bridge".to_string()),
        vertices,
        source.indices().to_vec(),
    )
    .expect("zero UV cube");
    zero_uv.set_texture(MeshTexture::white_1x1());

    let result =
        bridge_split_mesh_in_world(&zero_uv, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::X))
            .expect("split");
    assert!(result.part_a.texture().is_some());
    assert!(result.part_b.texture().is_some());
}

#[test]
fn source_mesh_is_unchanged_on_success_and_failure() {
    let source = textured_cube();
    let vertices = source.vertices().to_vec();
    let indices = source.indices().to_vec();
    let topology_id = source.topology_id();
    let _ = bridge_split_mesh_in_world(&source, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::X))
        .expect("split");
    let _ = bridge_split_mesh_in_world(
        &source,
        Affine3A::from_scale(Vec3::ZERO),
        request(Vec3::ZERO, Vec3::X),
    );

    assert_eq!(source.vertices(), vertices);
    assert_eq!(source.indices(), indices);
    assert_eq!(source.topology_id(), topology_id);
    assert_eq!(source.name(), Some("Bridge"));
}

#[test]
fn bridge_split_normalizes_redundant_degenerate_import_faces_before_cutting() {
    let source = textured_cube();
    let mut indices = source.indices().to_vec();
    // Some dental exporters leave a zero-area bookkeeping triangle alongside
    // an otherwise watertight surface. It must not make Bridge Split unusable.
    indices.extend([0, 0, 1]);
    let mut degenerate_source = Mesh::new(
        Some("Bridge with import residue".to_string()),
        source.vertices().to_vec(),
        indices,
    )
    .expect("mesh accepts indexed import data");
    degenerate_source.set_texture(source.texture().expect("fixture texture").clone());

    let result = split_after_bridge_normalization(&degenerate_source);

    assert!((world_gap(&result, Affine3A::IDENTITY, Vec3::ZERO, Vec3::X) - 0.05).abs() < 1e-5);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
    assert_eq!(degenerate_source.triangle_count(), 13);
    for part in [&result.part_a, &result.part_b] {
        assert!(part.has_vertex_colors());
        assert!(part.has_uvs());
        assert!(part.texture().is_some());
    }
}

#[test]
fn bridge_split_normalization_leaves_healthy_sources_on_the_fast_path() {
    let clean = textured_cube();
    assert!(normalize_bridge_split_input(&clean)
        .expect("healthy bridge input")
        .is_none());
}

#[test]
fn bridge_split_discards_duplicate_import_faces_before_cutting() {
    let source = textured_cube();
    let mut indices = source.indices().to_vec();
    indices.extend_from_slice(&source.indices()[..3]);
    let duplicate_face_source = Mesh::new(
        Some("Bridge with duplicate import face".to_string()),
        source.vertices().to_vec(),
        indices,
    )
    .expect("mesh accepts indexed import data");

    let result = split_after_bridge_normalization(&duplicate_face_source);

    assert!((world_gap(&result, Affine3A::IDENTITY, Vec3::ZERO, Vec3::X) - 0.05).abs() < 1e-5);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
}

#[test]
fn bridge_split_closes_tiny_import_holes_before_cutting() {
    let source = textured_cube();
    let mut indices = source.indices().to_vec();
    indices.drain(..3);
    let vertices = source
        .vertices()
        .iter()
        .map(|vertex| Vertex {
            position: (Vec3::from_array(vertex.position) * 0.1).to_array(),
            ..*vertex
        })
        .collect();
    let tiny_hole_source = Mesh::new(
        Some("Bridge with tiny import hole".to_string()),
        vertices,
        indices,
    )
    .expect("mesh accepts indexed import data");

    let result = split_after_bridge_normalization(&tiny_hole_source);

    assert!((world_gap(&result, Affine3A::IDENTITY, Vec3::ZERO, Vec3::X) - 0.05).abs() < 1e-5);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
}

#[test]
fn bridge_split_normalization_refuses_large_open_rims() {
    let source = textured_cube();
    let mut indices = source.indices().to_vec();
    indices.drain(..3);
    let large_hole_source = Mesh::new(
        Some("Bridge with large import hole".to_string()),
        source.vertices().to_vec(),
        indices,
    )
    .expect("mesh accepts indexed import data");

    assert!(matches!(
        normalize_bridge_split_input(&large_hole_source),
        Err(CoreBridgeSplitError::Kernel(
            BridgeSplitError::OpenOrNonManifold { .. }
        ))
    ));
}

#[test]
fn bridge_split_reorients_inconsistent_import_faces_before_cutting() {
    let source = textured_cube();
    let mut indices = source.indices().to_vec();
    indices.swap(0, 1);
    let inconsistent_source = Mesh::new(
        Some("Bridge with inconsistent import winding".to_string()),
        source.vertices().to_vec(),
        indices,
    )
    .expect("mesh accepts indexed import data");

    let result = split_after_bridge_normalization(&inconsistent_source);

    assert!((world_gap(&result, Affine3A::IDENTITY, Vec3::ZERO, Vec3::X) - 0.05).abs() < 1e-5);
    assert_winding_matches_normals(&result.part_a);
    assert_winding_matches_normals(&result.part_b);
}

#[test]
fn bridge_split_normalization_never_deletes_a_valid_closed_component() {
    let source = dense_octahedron_with_tetrahedron_debris();

    assert!(matches!(
        normalize_bridge_split_input(&source),
        Err(CoreBridgeSplitError::Kernel(
            BridgeSplitError::DisconnectedInput { .. }
        ))
    ));
}

#[cfg(feature = "robust-csg")]
#[test]
fn robust_repair_keeps_a_valid_tiny_shell_while_removing_a_degenerate_face() {
    let source = dense_octahedron_with_tetrahedron_debris();
    let mut indices = source.indices().to_vec();
    indices.extend([0, 0, 1]);
    let defective = Mesh::new(
        Some("Bridge with a degenerate import face".to_string()),
        source.vertices().to_vec(),
        indices,
    )
    .expect("mesh storage accepts importer residue");

    let result =
        bridge_split_mesh_in_world(&defective, Affine3A::IDENTITY, request(Vec3::ZERO, Vec3::X))
            .expect("safe repair preserves every closed shell");

    let tiny_shell_vertices = [&result.part_a, &result.part_b]
        .into_iter()
        .flat_map(Mesh::vertices)
        .filter(|vertex| vertex.position[0] >= 50.0)
        .count();
    assert_eq!(tiny_shell_vertices, 4);
}

#[test]
fn bridge_split_normalization_refuses_structural_nonmanifold_surgery() {
    let source = textured_cube();
    let mut vertices = source.vertices().to_vec();
    let apex = u32::try_from(vertices.len()).expect("small fixture");
    vertices.push(Vertex::at([0.0, -2.0, 0.0].into()));
    let mut indices = source.indices().to_vec();
    indices.extend([0, 1, apex]);
    let nonmanifold_source = Mesh::new(Some("Structural defect".to_string()), vertices, indices)
        .expect("mesh accepts indexed import data");

    assert!(matches!(
        normalize_bridge_split_input(&nonmanifold_source),
        Err(CoreBridgeSplitError::Kernel(
            BridgeSplitError::OpenOrNonManifold { .. }
        ))
    ));
}

#[test]
fn bridge_split_normalization_refuses_near_coincident_cracks() {
    let source = textured_cube();
    let mut vertices = source.vertices().to_vec();
    let cracked_vertex = u32::try_from(vertices.len()).expect("small fixture");
    let mut duplicate = vertices[0];
    duplicate.position[0] += 1.0e-5;
    vertices.push(duplicate);
    let mut indices = source.indices().to_vec();
    indices[0] = cracked_vertex;
    let cracked_source = Mesh::new(Some("Near crack".to_string()), vertices, indices)
        .expect("mesh accepts indexed import data");

    assert!(matches!(
        normalize_bridge_split_input(&cracked_source),
        Err(CoreBridgeSplitError::Kernel(
            BridgeSplitError::OpenOrNonManifold { .. }
        ))
    ));
}

fn dense_octahedron_with_tetrahedron_debris() -> Mesh {
    let mut vertices = vec![
        Vertex::at([0.0, 0.0, 10.0].into()),
        Vertex::at([0.0, 0.0, -10.0].into()),
        Vertex::at([10.0, 0.0, 0.0].into()),
        Vertex::at([0.0, 10.0, 0.0].into()),
        Vertex::at([-10.0, 0.0, 0.0].into()),
        Vertex::at([0.0, -10.0, 0.0].into()),
    ];
    let mut indices = vec![
        0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 2, 1, 3, 2, 1, 4, 3, 1, 5, 4, 1, 2, 5,
    ];
    for _ in 0..3 {
        let mut next = Vec::with_capacity(indices.len() * 4);
        for face in indices.chunks_exact(3) {
            let a = face[0];
            let b = face[1];
            let c = face[2];
            let midpoint = |first: u32, second: u32, vertices: &mut Vec<Vertex>| {
                let first = Vec3::from_array(vertices[first as usize].position);
                let second = Vec3::from_array(vertices[second as usize].position);
                let index = u32::try_from(vertices.len()).expect("small fixture");
                vertices.push(Vertex::at((first + second) * 0.5));
                index
            };
            let ab = midpoint(a, b, &mut vertices);
            let bc = midpoint(b, c, &mut vertices);
            let ca = midpoint(c, a, &mut vertices);
            next.extend([a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        indices = next;
    }

    let debris = u32::try_from(vertices.len()).expect("small fixture");
    vertices.extend([
        Vertex::at([50.0, 0.0, 0.0].into()),
        Vertex::at([50.01, 0.0, 0.0].into()),
        Vertex::at([50.0, 0.01, 0.0].into()),
        Vertex::at([50.0, 0.0, 0.01].into()),
    ]);
    indices.extend([
        debris,
        debris + 2,
        debris + 1,
        debris,
        debris + 1,
        debris + 3,
        debris,
        debris + 3,
        debris + 2,
        debris + 1,
        debris + 2,
        debris + 3,
    ]);
    Mesh::new(Some("Bridge with debris".to_string()), vertices, indices).expect("closed fixture")
}

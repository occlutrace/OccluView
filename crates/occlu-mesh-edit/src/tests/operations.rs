use super::*;

#[test]
fn point_cloud_face_edit_validation_is_typed() {
    let err = validate_face_edit_buffers(MeshTopology::PointCloud, &[v([0.0, 0.0, 0.0])], &[])
        .expect_err("point clouds are rejected");
    assert_eq!(err, MeshEditError::UnsupportedPointCloud);
}

#[test]
fn selection_length_mismatch_is_typed() {
    let selection = FaceSelection::new(vec![true, false]);
    let err =
        validate_selection_against_triangle_count(1, &selection).expect_err("length mismatch");
    assert_eq!(
        err,
        MeshEditError::InvalidSelectionLength {
            expected: 1,
            actual: 2
        }
    );
}

#[test]
fn delete_selected_faces_removes_selected_triangle_and_reports_count() {
    let mesh = mesh_with_two_triangles();
    let result = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, false]),
        MeshEditOptions::default(),
    )
    .expect("delete selected faces");

    assert_eq!(result.report.input_vertices, 4);
    assert_eq!(result.report.input_triangles, 2);
    assert_eq!(result.report.output_vertices, 4);
    assert_eq!(result.report.output_triangles, 1);
    assert_eq!(result.report.removed_triangles, 1);
    assert!(result.report.warnings.is_empty());
    assert_eq!(result.mesh.indices, vec![1, 3, 2]);
}

#[test]
fn crop_to_selected_faces_keeps_only_selected_triangle() {
    let mesh = mesh_with_two_triangles();
    let result = crop_to_selected_faces(
        &mesh,
        &selected_faces(&[false, true]),
        MeshEditOptions::default(),
    )
    .expect("crop selected faces");

    assert_eq!(result.report.output_triangles, 1);
    assert_eq!(result.report.removed_triangles, 1);
    assert_eq!(result.mesh.indices, vec![1, 3, 2]);
}

#[test]
fn compact_mode_remaps_indices_and_drops_orphan_vertices() {
    let mesh = mesh_with_two_triangles();
    let result = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, false]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("compact delete");

    assert_eq!(result.mesh.vertices.len(), 3);
    assert_eq!(result.mesh.indices, vec![0, 1, 2]);
    assert_eq!(result.mesh.vertices[0].position, mesh.vertices[1].position);
    assert_eq!(result.mesh.vertices[1].position, mesh.vertices[3].position);
    assert_eq!(result.mesh.vertices[2].position, mesh.vertices[2].position);
    assert_eq!(result.mesh.vertices[0].color, mesh.vertices[1].color);
    assert_eq!(result.mesh.vertices[1].uv, mesh.vertices[3].uv);
}

#[test]
fn non_compact_mode_keeps_all_vertices() {
    let mesh = mesh_with_two_triangles();
    let result = crop_to_selected_faces(
        &mesh,
        &selected_faces(&[false, true]),
        MeshEditOptions::default(),
    )
    .expect("non-compact crop");

    assert_eq!(result.mesh.vertices.len(), mesh.vertices.len());
    for (output, input) in result.mesh.vertices.iter().zip(mesh.vertices.iter()) {
        assert_eq!(output.position, input.position);
        assert_eq!(output.color, input.color);
        assert_eq!(output.uv, input.uv);
    }
}

#[test]
fn surviving_colors_and_uvs_are_preserved() {
    let mesh = mesh_with_two_triangles();
    let result = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, false]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("preserve attributes");

    assert_eq!(result.mesh.vertices[0].color, mesh.vertices[1].color);
    assert_eq!(result.mesh.vertices[0].uv, mesh.vertices[1].uv);
    assert_eq!(result.mesh.vertices[1].color, mesh.vertices[3].color);
    assert_eq!(result.mesh.vertices[1].uv, mesh.vertices[3].uv);
}

#[test]
fn same_position_vertices_with_distinct_attributes_are_not_welded() {
    let mesh = mesh_with_duplicate_positions_and_distinct_attributes();
    let result = crop_to_selected_faces(
        &mesh,
        &selected_faces(&[true, true]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("crop duplicate positions");

    assert_eq!(result.mesh.vertices.len(), 6);
    assert_eq!(
        result.mesh.vertices[0].position,
        result.mesh.vertices[3].position
    );
    assert_eq!(
        result.mesh.vertices[1].position,
        result.mesh.vertices[4].position
    );
    assert_eq!(
        result.mesh.vertices[2].position,
        result.mesh.vertices[5].position
    );
    assert_ne!(result.mesh.vertices[0].color, result.mesh.vertices[3].color);
    assert_ne!(result.mesh.vertices[1].uv, result.mesh.vertices[4].uv);
}

#[test]
fn selected_connected_components_split_at_single_shared_vertex() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([2.0, 0.0, 0.0]),
            v([2.0, 1.0, 0.0]),
        ],
        indices: vec![0, 1, 2, 2, 3, 4],
        topology: MeshTopology::TriangleMesh,
    };

    let components = selected_connected_components(&mesh, &selected_faces(&[true, true]))
        .expect("split at shared vertex");

    assert_eq!(components.len(), 2);
    assert_eq!(components[0], vec![0]);
    assert_eq!(components[1], vec![1]);
}

#[test]
fn selected_connected_components_split_disconnected_selected_islands_in_order() {
    let mesh = mesh_with_three_islands();
    let components = selected_connected_components(&mesh, &selected_faces(&[true, false, true]))
        .expect("split selected components");

    assert_eq!(components.len(), 2);
    assert_eq!(components[0], vec![0]);
    assert_eq!(components[1], vec![2]);
}

#[test]
fn selected_connected_components_keep_adjacent_selected_triangles_together() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([1.0, 1.0, 0.0]),
            v([2.0, 0.0, 0.0]),
        ],
        indices: vec![0, 1, 2, 1, 3, 2, 1, 4, 3],
        topology: MeshTopology::TriangleMesh,
    };

    let components = selected_connected_components(&mesh, &selected_faces(&[true, true, false]))
        .expect("split adjacent selection");

    assert_eq!(components.len(), 1);
    assert_eq!(components[0], vec![0, 1]);
}

#[test]
fn selected_connected_components_validate_point_cloud_and_selection_length() {
    let point_cloud = MeshEditBuffers {
        vertices: vec![v([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };
    let selection = selected_faces(&[]);
    let point_cloud_err = selected_connected_components(&point_cloud, &selection)
        .expect_err("point clouds are rejected");
    assert_eq!(point_cloud_err, MeshEditError::UnsupportedPointCloud);

    let mesh = mesh_with_two_triangles();
    let mismatch = selected_faces(&[true]);
    let mismatch_err =
        selected_connected_components(&mesh, &mismatch).expect_err("selection length mismatch");
    assert_eq!(
        mismatch_err,
        MeshEditError::InvalidSelectionLength {
            expected: 2,
            actual: 1
        }
    );
}

#[test]
fn invert_orientation_flips_winding_and_recomputes_normals() {
    let mesh = mesh_with_two_triangles();
    let result = invert_orientation(&mesh, None).expect("invert orientation");

    assert_eq!(result.report.input_triangles, 2);
    assert_eq!(result.report.output_triangles, 2);
    assert_eq!(result.report.removed_triangles, 0);
    assert_eq!(result.mesh.indices, vec![0, 2, 1, 1, 2, 3]);
    assert_eq!(result.mesh.vertices.len(), mesh.vertices.len());
    for (output, input) in result.mesh.vertices.iter().zip(mesh.vertices.iter()) {
        assert_eq!(output.position, input.position);
        assert_eq!(output.color, input.color);
        assert_eq!(output.uv, input.uv);
    }
    assert_eq!(result.mesh.vertices[0].normal, [0.0, 0.0, -1.0]);
    assert_eq!(result.mesh.vertices[1].normal, [0.0, 0.0, -1.0]);
    assert_eq!(result.mesh.vertices[2].normal, [0.0, 0.0, -1.0]);
    assert_eq!(result.mesh.vertices[3].normal, [0.0, 0.0, -1.0]);
}

#[test]
fn invert_orientation_rejects_point_clouds() {
    let point_cloud = MeshEditBuffers {
        vertices: vec![v([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };

    let err = invert_orientation(&point_cloud, None).expect_err("point clouds are rejected");
    assert_eq!(err, MeshEditError::UnsupportedPointCloud);
}

#[test]
fn invert_orientation_can_flip_only_the_selected_faces() {
    let mesh = mesh_with_two_triangles();
    let result = invert_orientation(&mesh, Some(&selected_faces(&[true, false])))
        .expect("partial invert orientation");

    assert_eq!(result.mesh.indices, vec![0, 2, 1, 1, 3, 2]);
    assert_eq!(result.report.output_triangles, 2);
}

#[test]
fn stale_normals_are_recomputed_after_delete_and_crop() {
    let mesh = mesh_with_two_triangles();

    let delete_result = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, false]),
        MeshEditOptions::default(),
    )
    .expect("delete recomputes normals");
    assert_eq!(delete_result.mesh.vertices[1].normal, [0.0, 0.0, 1.0]);
    assert_eq!(delete_result.mesh.vertices[2].normal, [0.0, 0.0, 1.0]);
    assert_eq!(delete_result.mesh.vertices[3].normal, [0.0, 0.0, 1.0]);

    let crop_result = crop_to_selected_faces(
        &mesh,
        &selected_faces(&[false, true]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("crop recomputes normals");
    assert_eq!(crop_result.mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
    assert_eq!(crop_result.mesh.vertices[1].normal, [0.0, 0.0, 1.0]);
    assert_eq!(crop_result.mesh.vertices[2].normal, [0.0, 0.0, 1.0]);
}

#[test]
fn delete_and_crop_validate_point_cloud_and_selection_length() {
    let point_cloud = MeshEditBuffers {
        vertices: vec![v([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };
    let selection = selected_faces(&[]);

    let point_cloud_err =
        delete_selected_faces(&point_cloud, &selection, MeshEditOptions::default())
            .expect_err("point clouds are rejected");
    assert_eq!(point_cloud_err, MeshEditError::UnsupportedPointCloud);

    let mesh = mesh_with_two_triangles();
    let mismatch = selected_faces(&[true]);
    let mismatch_err = crop_to_selected_faces(&mesh, &mismatch, MeshEditOptions::default())
        .expect_err("selection length mismatch");
    assert_eq!(
        mismatch_err,
        MeshEditError::InvalidSelectionLength {
            expected: 2,
            actual: 1
        }
    );
}

#[test]
fn all_selected_delete_returns_empty_triangle_mesh_with_expected_vertices() {
    let mesh = mesh_with_two_triangles();

    let non_compact = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, true]),
        MeshEditOptions::default(),
    )
    .expect("delete all selected non-compact");
    assert!(non_compact.mesh.indices.is_empty());
    assert_eq!(non_compact.mesh.vertices.len(), mesh.vertices.len());
    for (output, input) in non_compact.mesh.vertices.iter().zip(mesh.vertices.iter()) {
        assert_eq!(output.position, input.position);
        assert_eq!(output.color, input.color);
        assert_eq!(output.uv, input.uv);
        assert_eq!(output.normal, [0.0, 0.0, 0.0]);
    }

    let compact = delete_selected_faces(
        &mesh,
        &selected_faces(&[true, true]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("delete all selected compact");
    assert!(compact.mesh.indices.is_empty());
    assert!(compact.mesh.vertices.is_empty());
    assert_eq!(compact.report.output_vertices, 0);
    assert_eq!(compact.report.output_triangles, 0);
}

#[test]
fn explicit_normal_recompute_overwrites_stale_valid_normals() {
    let mut vertices = vec![
        EditVertex {
            normal: [1.0, 0.0, 0.0],
            ..v([0.0, 0.0, 0.0])
        },
        EditVertex {
            normal: [0.0, 1.0, 0.0],
            ..v([1.0, 0.0, 0.0])
        },
        EditVertex {
            normal: [1.0, 1.0, 0.0],
            ..v([0.0, 1.0, 0.0])
        },
    ];

    recompute_all_normals(&mut vertices, &[0, 1, 2]).expect("normals recompute");

    for vertex in vertices {
        assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn fill_holes_closes_a_simple_boundary_loop() {
    // A bare bowl's rim is its only boundary; the scan-border guard is off so
    // the planar-cap mechanics under test actually run.
    let mesh = bowl_mesh();
    let result = fill_holes(
        &mesh,
        None,
        MeshEditOptions {
            compact_vertices: true,
            protect_scan_border: false,
            ..MeshEditOptions::default()
        },
    )
    .expect("fill holes");

    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(result.report.output_vertices, 5);
    assert_eq!(result.report.output_triangles, 6);
    assert_eq!(result.mesh.indices.len(), 18);
}

#[test]
fn fill_holes_respects_selection_scoped_loop_gating() {
    let mesh = bowl_mesh();

    let partial = fill_holes(
        &mesh,
        Some(&selected_faces(&[true, false, false, false])),
        MeshEditOptions::default(),
    )
    .expect("partial fill");
    assert_eq!(partial.report.filled_holes, 0);
    assert_eq!(partial.report.output_triangles, mesh.triangle_count());
    // A partially selected rim staying open is requested behavior, not
    // degeneracy: no warning.
    assert!(partial.report.warnings.is_empty());

    let full = fill_holes(
        &mesh,
        Some(&selected_faces(&[true, true, true, true])),
        MeshEditOptions::default(),
    )
    .expect("fully selected fill");
    assert_eq!(full.report.filled_holes, 1);
    assert_eq!(full.report.output_triangles, 6);
}

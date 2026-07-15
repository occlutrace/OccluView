use super::*;

#[test]

fn attribute_remap_copies_vertex_color_and_uv_without_welding() {
    let vertices = vec![
        EditVertex {
            position: [1.0, 2.0, 3.0],
            color: [1, 2, 3, 4],
            uv: [0.1, 0.2],
            ..EditVertex::default()
        },
        EditVertex {
            position: [1.0, 2.0, 3.0],
            color: [9, 8, 7, 6],
            uv: [0.9, 0.8],
            ..EditVertex::default()
        },
        EditVertex {
            position: [5.0, 6.0, 7.0],
            color: [5, 5, 5, 5],
            uv: [0.5, 0.6],
            ..EditVertex::default()
        },
    ];

    let (copied, remap) = copy_surviving_vertices(&vertices, &[0, 1]).expect("copy survivors");

    assert_eq!(copied.len(), 2);
    assert_eq!(copied[0], vertices[0]);
    assert_eq!(copied[1], vertices[1]);
    assert_eq!(copied[0].position, copied[1].position);
    assert_ne!(copied[0].color, copied[1].color);
    assert_ne!(copied[0].uv, copied[1].uv);
    assert_eq!(remap[0], Some(0));
    assert_eq!(remap[1], Some(1));
    assert_eq!(remap[2], None);
}

#[test]
fn default_options_are_conservative_and_valid() {
    let options = MeshEditOptions::default();
    assert!(!options.compact_vertices);
    assert!(options.max_boundary_loop > 0);
    assert_eq!(
        options.attribute_policy.generated_vertex_policy,
        GeneratedVertexPolicy::InterpolateBoundary
    );
    assert_eq!(
        validate_mesh_edit_options(options).expect("valid options"),
        options
    );
}

#[test]
fn raw_triangle_mesh_validation_rejects_bad_indices() {
    let vertices = vec![v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0])];

    let bad_count = validate_triangle_mesh_data(&vertices, &[0, 1]).expect_err("bad count");
    assert!(matches!(bad_count, MeshEditError::MalformedMesh { .. }));

    let bad_range = validate_triangle_mesh_data(&vertices, &[0, 1, 2]).expect_err("bad range");
    assert!(matches!(bad_range, MeshEditError::MalformedMesh { .. }));
}

#[test]
fn mesh_edit_report_and_result_are_constructible() {
    let report = MeshEditReport {
        input_vertices: 3,
        input_triangles: 1,
        output_vertices: 3,
        output_triangles: 1,
        removed_triangles: 0,
        filled_holes: 0,
        moved_vertices: 0,
        skipped_border_rims: 0,
        skipped_oversize_rims: 0,
        skipped_damaged_rims: 1,
        healed_rims: 0,
        warnings: vec![MeshEditWarning::DegenerateGeometry],
    };
    let result = MeshEditResult {
        mesh: MeshEditBuffers {
            vertices: vec![v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0]), v([0.0, 1.0, 0.0])],
            indices: vec![0, 1, 2],
            topology: MeshTopology::TriangleMesh,
        },
        report,
    };

    assert_eq!(result.mesh.topology, MeshTopology::TriangleMesh);
    assert_eq!(result.report.input_triangles, 1);
}

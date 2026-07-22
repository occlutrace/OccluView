use super::*;
use occlu_mesh_edit::{FaceSelection, MeshEditOptions, MeshTopology};
use std::{mem::size_of, ptr::addr_of};

fn v(x: f32, y: f32, z: f32) -> Vertex {
    Vertex::at(Vec3::new(x, y, z))
}

#[test]
fn valid_mesh_constructs() {
    let mesh = Mesh::new(
        Some("tri".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid mesh");
    assert_eq!(mesh.triangle_count(), 1);
    assert_eq!(mesh.name(), Some("tri"));
    assert!(!mesh.has_vertex_colors());
}

#[test]
fn sculpted_mesh_refits_a_warm_bvh_for_the_next_pick() {
    let mesh = Mesh::new(
        Some("tri".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid mesh");
    mesh.warm_bvh();

    let moved = mesh
        .vertices()
        .iter()
        .map(|vertex| Vertex::at(Vec3::from_array(vertex.position) + Vec3::new(0.0, 0.0, 5.0)))
        .collect();
    let sculpted = mesh
        .with_sculpted_vertices(moved)
        .expect("same vertex count");
    assert!(sculpted.bvh_is_ready());

    let hit = sculpted
        .pick_ray_local(Vec3::new(0.25, 0.25, 10.0), -Vec3::Z, |_| true)
        .expect("refitted tree should hit the moved triangle");
    assert_eq!(hit.0, 0);
    assert!((hit.1.z - 5.0).abs() < 1e-5);
}

#[test]
fn triangle_mesh_computes_normals_when_source_has_none() {
    let mesh = Mesh::new(
        Some("tri".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid mesh");

    for vertex in mesh.vertices() {
        assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn triangle_mesh_repairs_missing_normals_per_vertex() {
    let vertices = vec![
        v(0.0, 0.0, 0.0).with_normal(Vec3::Z),
        v(1.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0).with_normal(Vec3::Z),
    ];

    let mesh = Mesh::new(Some("tri".into()), vertices, vec![0, 1, 2]).expect("valid mesh");

    for vertex in mesh.vertices() {
        assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn duplicate_position_normals_are_smoothed_for_soft_edges() {
    let soft_a = Vec3::new(0.0, 0.0, 1.0);
    let soft_b = Vec3::new(0.0, 0.20, 0.98).normalize();
    let vertices = vec![
        Vertex::at(Vec3::ZERO).with_normal(soft_a),
        Vertex::at(Vec3::X).with_normal(soft_a),
        Vertex::at(Vec3::Y).with_normal(soft_a),
        Vertex::at(Vec3::ZERO).with_normal(soft_b),
        Vertex::at(Vec3::Y).with_normal(soft_b),
        Vertex::at(Vec3::Z).with_normal(soft_b),
    ];

    let mesh =
        Mesh::new(Some("soft".into()), vertices, vec![0, 1, 2, 3, 4, 5]).expect("valid mesh");

    let expected = (soft_a + soft_b).normalize();
    assert_ne!(mesh.vertices()[0].normal, soft_a.to_array());
    assert_ne!(mesh.vertices()[3].normal, soft_b.to_array());
    assert!((Vec3::from_array(mesh.vertices()[0].normal) - expected).length() < 1e-5);
    assert!((Vec3::from_array(mesh.vertices()[3].normal) - expected).length() < 1e-5);
}

#[test]
fn near_duplicate_position_normals_are_smoothed_for_stl_float_noise() {
    let soft_a = Vec3::new(0.0, 0.0, 1.0);
    let soft_b = Vec3::new(0.0, 0.16, 0.987).normalize();
    let noisy_origin = Vec3::new(0.0007, -0.0006, 0.0003);
    let vertices = vec![
        Vertex::at(Vec3::ZERO).with_normal(soft_a),
        Vertex::at(Vec3::X).with_normal(soft_a),
        Vertex::at(Vec3::Y).with_normal(soft_a),
        Vertex::at(noisy_origin).with_normal(soft_b),
        Vertex::at(Vec3::Y).with_normal(soft_b),
        Vertex::at(Vec3::Z).with_normal(soft_b),
    ];

    let mesh =
        Mesh::new(Some("noisy".into()), vertices, vec![0, 1, 2, 3, 4, 5]).expect("valid mesh");

    let expected = (soft_a + soft_b).normalize();
    assert!((Vec3::from_array(mesh.vertices()[0].normal) - expected).length() < 1e-5);
    assert!((Vec3::from_array(mesh.vertices()[3].normal) - expected).length() < 1e-5);
}

#[test]
fn duplicate_position_normals_preserve_sharp_edges() {
    let vertices = vec![
        Vertex::at(Vec3::ZERO).with_normal(Vec3::X),
        Vertex::at(Vec3::Y).with_normal(Vec3::X),
        Vertex::at(Vec3::Z).with_normal(Vec3::X),
        Vertex::at(Vec3::ZERO).with_normal(Vec3::Y),
        Vertex::at(Vec3::X).with_normal(Vec3::Y),
        Vertex::at(Vec3::Z).with_normal(Vec3::Y),
    ];

    let mesh =
        Mesh::new(Some("sharp".into()), vertices, vec![0, 1, 2, 3, 4, 5]).expect("valid mesh");

    assert_eq!(mesh.vertices()[0].normal, Vec3::X.to_array());
    assert_eq!(mesh.vertices()[3].normal, Vec3::Y.to_array());
}

#[test]
fn bad_index_count_is_rejected() {
    let err = Mesh::new(None, vec![v(0.0, 0.0, 0.0)], vec![0, 1]).unwrap_err();
    assert!(matches!(
        err,
        CoreError::IndexCountNotMultipleOfThree { .. }
    ));
}

#[test]
fn out_of_range_index_is_rejected() {
    let err = Mesh::new(None, vec![v(0.0, 0.0, 0.0)], vec![0, 1, 5]).unwrap_err();
    assert!(matches!(err, CoreError::IndexOutOfRange { .. }));
}

#[test]
fn bbox_is_computed_and_cached() {
    let mut mesh = Mesh::new(
        None,
        vec![v(-1.0, -2.0, 0.0), v(3.0, 4.0, 0.0), v(0.0, 0.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    let b = mesh.bbox();
    assert_eq!(b.min, Vec3::new(-1.0, -2.0, 0.0));
    assert_eq!(b.max, Vec3::new(3.0, 4.0, 0.0));
    // Cached: second call must return the same value.
    assert_eq!(mesh.bbox(), b);
}

#[test]
fn vertex_color_is_detected() {
    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO).with_color([10, 20, 30, 255]),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
        ],
        vec![0, 1, 2],
    )
    .expect("valid");
    assert!(mesh.has_vertex_colors());
}

#[test]
fn builder_round_trip() {
    let mut b = MeshBuilder::new().with_name("built").reserve(3, 3);
    let a = b.push_vertex(v(0.0, 0.0, 0.0));
    let c = b.push_vertex(v(1.0, 0.0, 0.0));
    let d = b.push_vertex(v(0.0, 1.0, 0.0));
    b.push_triangle(a, c, d);
    let mesh = b.build().expect("valid");
    assert_eq!(mesh.name(), Some("built"));
    assert_eq!(mesh.triangle_count(), 1);
}

#[test]
fn vertex_uv_is_detected() {
    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO).with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)).with_uv([1.0, 0.0]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)).with_uv([0.0, 1.0]),
        ],
        vec![0, 1, 2],
    )
    .expect("valid");
    assert!(mesh.has_uvs());
}

#[test]
fn vertex_no_uv_is_not_detected() {
    let mesh = Mesh::new(
        None,
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    assert!(!mesh.has_uvs());
}

#[test]
fn vertex_layout_has_uv_appended() {
    // Adding `uv` ([f32;2] = 8 bytes) after `color` grew the struct from
    // 28 to 36 bytes. The layout is position@0, normal@12, color@24,
    // uv@28 — no padding holes, all naturally aligned (max align = 4).
    assert_eq!(size_of::<Vertex>(), 36);
    let sample = Vertex {
        position: [1.0, 2.0, 3.0],
        normal: [4.0, 5.0, 6.0],
        color: [7, 8, 9, 10],
        uv: [11.0, 12.0],
    };
    let base = addr_of!(sample) as usize;
    assert_eq!(addr_of!(sample.position) as usize - base, 0);
    assert_eq!(addr_of!(sample.normal) as usize - base, 12);
    assert_eq!(addr_of!(sample.color) as usize - base, 24);
    assert_eq!(addr_of!(sample.uv) as usize - base, 28);
}

#[test]
fn mesh_texture_white_1x1() {
    let t = MeshTexture::white_1x1();
    assert_eq!(t.width, 1);
    assert_eq!(t.height, 1);
    assert_eq!(t.rgba, vec![255, 255, 255, 255]);
}

#[test]
fn set_texture_attaches() {
    let mut mesh = Mesh::new(
        None,
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    assert!(mesh.texture().is_none());
    mesh.set_texture(MeshTexture::white_1x1());
    assert!(mesh.texture().is_some());
}

#[test]
fn bbox_uncached_matches_cached() {
    let mut mesh = Mesh::new(
        None,
        vec![v(-1.0, -2.0, 0.0), v(3.0, 4.0, 0.0), v(0.0, 0.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    let uncached = mesh.bbox_uncached();
    let cached = mesh.bbox();
    assert_eq!(uncached, cached);
}

#[test]
fn constructor_populates_read_only_bbox_cache() {
    let mesh = Mesh::new(
        None,
        vec![v(-1.0, -2.0, 0.0), v(3.0, 4.0, 0.0), v(0.0, 0.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");

    assert_eq!(mesh.bbox_cached(), mesh.bbox_uncached());
}

#[test]
fn topology_id_survives_clone_but_changes_for_new_mesh() {
    let mesh = Mesh::new(
        None,
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    let cloned = mesh.clone();
    let rebuilt = Mesh::new(
        None,
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");

    assert_eq!(mesh.topology_id(), cloned.topology_id());
    assert_ne!(mesh.topology_id(), rebuilt.topology_id());
}

#[test]
fn colored_and_uv_vertices_round_trip_through_buffers() {
    let mesh = Mesh::new(
        Some("edit-me".into()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0))
                .with_normal(Vec3::X)
                .with_color([10, 20, 30, 255])
                .with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0))
                .with_normal(Vec3::Y)
                .with_color([40, 50, 60, 255])
                .with_uv([1.0, 0.0]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0))
                .with_normal(Vec3::Z)
                .with_color([70, 80, 90, 255])
                .with_uv([0.0, 1.0]),
        ],
        vec![0, 1, 2],
    )
    .expect("valid");
    let texture = MeshTexture::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let mut mesh = mesh;
    mesh.set_texture(texture.clone());

    let buffers = mesh_edit_buffers_from_mesh(&mesh);
    let rebuilt = mesh_from_edit_buffers_like(&mesh, buffers).expect("round trip");

    assert_eq!(rebuilt.name(), Some("edit-me"));
    assert_eq!(rebuilt.vertices(), mesh.vertices());
    let rebuilt_texture = rebuilt.texture().expect("texture restored");
    assert_eq!(rebuilt_texture.width, texture.width);
    assert_eq!(rebuilt_texture.height, texture.height);
    assert_eq!(rebuilt_texture.rgba, texture.rgba);
}

#[test]
fn same_position_vertices_with_distinct_attributes_remain_distinct() {
    let mesh = Mesh::new(
        Some("duplicate".into()),
        vec![
            Vertex::at(Vec3::new(1.0, 1.0, 1.0))
                .with_normal(Vec3::X)
                .with_color([1, 2, 3, 4])
                .with_uv([0.1, 0.2]),
            Vertex::at(Vec3::new(1.0, 1.0, 1.0))
                .with_normal(Vec3::Y)
                .with_color([9, 8, 7, 6])
                .with_uv([0.9, 0.8]),
            Vertex::at(Vec3::new(2.0, 0.0, 0.0)).with_normal(Vec3::Z),
        ],
        vec![0, 1, 2],
    )
    .expect("valid");

    let buffers = mesh_edit_buffers_from_mesh(&mesh);
    assert_eq!(buffers.vertices[0].position, buffers.vertices[1].position);
    assert_ne!(buffers.vertices[0].color, buffers.vertices[1].color);
    assert_ne!(buffers.vertices[0].uv, buffers.vertices[1].uv);
    assert_ne!(buffers.vertices[0], buffers.vertices[1]);

    let rebuilt = mesh_from_edit_buffers_like(&mesh, buffers).expect("round trip");
    assert_eq!(
        rebuilt.vertices()[0].position,
        rebuilt.vertices()[1].position
    );
    assert_ne!(rebuilt.vertices()[0].color, rebuilt.vertices()[1].color);
    assert_ne!(rebuilt.vertices()[0].uv, rebuilt.vertices()[1].uv);
}

#[test]
fn point_cloud_round_trip_stays_point_cloud() {
    let mesh = Mesh::point_cloud(
        Some("cloud".into()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0)).with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 2.0, 3.0)).with_color([5, 6, 7, 255]),
        ],
    );

    let buffers = mesh_edit_buffers_from_mesh(&mesh);
    assert_eq!(buffers.topology, MeshTopology::PointCloud);
    assert!(buffers.indices.is_empty());

    let rebuilt = mesh_from_edit_buffers_like(&mesh, buffers).expect("round trip");
    assert!(rebuilt.is_point_cloud());
    assert_eq!(rebuilt.name(), Some("cloud"));
    assert_eq!(rebuilt.vertices(), mesh.vertices());
}

#[test]
fn invalid_triangle_indices_are_rejected() {
    let source = Mesh::new(
        Some("tri".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid");
    let mut buffers = mesh_edit_buffers_from_mesh(&source);
    buffers.indices = vec![0, 1, 99];

    let err = mesh_from_edit_buffers_like(&source, buffers).expect_err("invalid indices");
    assert!(matches!(err, CoreError::IndexOutOfRange { .. }));
}

#[test]
fn point_cloud_buffers_with_indices_are_rejected() {
    let source = Mesh::point_cloud(
        Some("cloud".into()),
        vec![Vertex::at(Vec3::new(0.0, 0.0, 0.0))],
    );
    let mut buffers = mesh_edit_buffers_from_mesh(&source);
    buffers.indices = vec![0, 0, 0];

    let err = mesh_from_edit_buffers_like(&source, buffers).expect_err("point cloud has indices");
    assert!(matches!(err, CoreError::Geometry(message) if message.contains("point cloud")));
}

#[test]
fn core_delete_selected_faces_preserves_mesh_metadata_and_reports() {
    let mut mesh = Mesh::new(
        Some("editable".into()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0))
                .with_color([10, 20, 30, 255])
                .with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0))
                .with_color([40, 50, 60, 255])
                .with_uv([1.0, 0.0]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0))
                .with_color([70, 80, 90, 255])
                .with_uv([0.0, 1.0]),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0))
                .with_color([100, 110, 120, 255])
                .with_uv([1.0, 1.0]),
        ],
        vec![0, 1, 2, 1, 3, 2],
    )
    .expect("valid mesh");
    let texture = MeshTexture::new(1, 1, vec![1, 2, 3, 4]);
    mesh.set_texture(texture.clone());

    let output = delete_selected_faces_in_mesh(
        &mesh,
        &FaceSelection::new(vec![true, false]),
        MeshEditOptions {
            compact_vertices: true,
            ..MeshEditOptions::default()
        },
    )
    .expect("delete through core");

    assert_eq!(output.report.input_triangles, 2);
    assert_eq!(output.report.output_triangles, 1);
    assert_eq!(output.report.removed_triangles, 1);
    assert_eq!(output.mesh.name(), Some("editable"));
    assert_eq!(output.mesh.indices(), &[0, 1, 2]);
    assert_eq!(output.mesh.vertices().len(), 3);
    assert_eq!(output.mesh.vertices()[0].color, [40, 50, 60, 255]);
    assert_eq!(output.mesh.vertices()[0].uv, [1.0, 0.0]);
    let output_texture = output.mesh.texture().expect("texture preserved");
    assert_eq!(output_texture.width, texture.width);
    assert_eq!(output_texture.height, texture.height);
    assert_eq!(output_texture.rgba, texture.rgba);
    assert_ne!(output.mesh.topology_id(), mesh.topology_id());
}

#[test]
fn core_edit_drops_texture_when_surviving_uvs_are_all_zero() {
    // Documented adapter behavior: [0,0] UVs mean "absent" for dental formats,
    // so a texture with no surviving non-zero UVs is not re-attached.
    let mut mesh = Mesh::new(
        Some("zero-uv".into()),
        vec![
            Vertex::at(Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh");
    mesh.set_texture(MeshTexture::new(1, 1, vec![9, 9, 9, 255]));

    let output = invert_mesh_orientation(&mesh, None).expect("invert through core");

    assert!(output.mesh.texture().is_none());
}

#[test]
fn core_invert_orientation_is_an_app_facing_mesh_edit() {
    let mesh = Mesh::new(
        Some("islands".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(1.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 1, 3, 2],
    )
    .expect("valid mesh");

    let inverted = invert_mesh_orientation(&mesh, None).expect("invert through core");
    assert_eq!(inverted.report.removed_triangles, 0);
    assert_eq!(inverted.mesh.indices(), &[0, 2, 1, 1, 2, 3]);
    assert_eq!(inverted.mesh.name(), Some("islands"));
    assert_eq!(inverted.mesh.vertices()[0].normal, [0.0, 0.0, -1.0]);
}

#[test]
fn core_face_edit_wrappers_reject_point_clouds() {
    let cloud = Mesh::point_cloud(Some("cloud".into()), vec![v(0.0, 0.0, 0.0)]);

    let err = invert_mesh_orientation(&cloud, None).expect_err("point cloud rejected");

    assert!(matches!(err, CoreError::Geometry(message) if message.contains("point cloud")));
}

#[test]
fn core_repair_cleans_defective_mesh_and_preserves_name() {
    // A watertight outward-oriented tetrahedron with one face duplicated:
    // the repair pipeline must drop the duplicate while the adapter
    // preserves the mesh name and the closed hull stays intact.
    let mesh = Mesh::new(
        Some("dirty scan".into()),
        vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
        ],
        vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2, 0, 2, 1],
    )
    .expect("valid mesh");

    let output =
        repair_mesh_in_mesh(&mesh, occlu_mesh_edit::RepairOptions::default()).expect("repair");

    assert!(output.report.changed_content());
    assert_eq!(output.report.removed_duplicate_triangles, 1);
    assert_eq!(output.mesh.name(), Some("dirty scan"));
    assert_eq!(output.mesh.triangle_count(), 4);
}

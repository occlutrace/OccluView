//! Mesh-edit round trips through the core-mesh adapter.
//!
//! These cover buffer conversion, the two rebuild normal policies, metadata
//! preservation, and the error surface. The core mesh model's own
//! construction, caching, and normal-repair tests stay in `occluview-core`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::cast_possible_wrap
)]

use glam::Vec3;
use occluview_core::{CoreError, Mesh, MeshTexture, Vertex};
use occluview_mesh_edit::{
    delete_selected_faces_in_mesh, invert_mesh_orientation, mesh_edit_buffers_from_mesh,
    mesh_from_edit_buffers_like, mesh_from_sculpt_session_like, repair_mesh_in_mesh, EditVertex,
    FaceSelection, MeshEditOptions, RepairOptions, SculptSessionBuffers,
};

/// A sculpt session exposes its live buffers to the whole-layer rebuild.
struct Session {
    vertices: Vec<EditVertex>,
    indices: Vec<u32>,
}

impl SculptSessionBuffers for Session {
    fn sculpt_vertices(&self) -> &[EditVertex] {
        &self.vertices
    }

    fn sculpt_indices(&self) -> &[u32] {
        &self.indices
    }
}

fn v(x: f32, y: f32, z: f32) -> Vertex {
    Vertex::at(Vec3::new(x, y, z))
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
    assert_eq!(
        buffers.topology,
        occluview_mesh_edit::MeshTopology::PointCloud
    );
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

    let output = repair_mesh_in_mesh(&mesh, RepairOptions::default()).expect("repair");

    assert!(output.report.changed_content());
    assert_eq!(output.report.removed_duplicate_triangles, 1);
    assert_eq!(output.mesh.name(), Some("dirty scan"));
    assert_eq!(output.mesh.triangle_count(), 4);
}

/// A sculpt commit rebuilds the layer from the session's own buffers, so a
/// stroke that remeshed keeps every row the display already showed: appended
/// vertices, rows an edit left unreferenced, and the exact triangle list. A
/// weld or a dropped row here would move the surface at the stroke's end,
/// which is the tear this pins.
#[test]
fn a_sculpt_commit_keeps_the_sessions_rows_and_indices() {
    let source = Mesh::new(
        Some("sculpt-commit".into()),
        vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    )
    .expect("valid source");

    // Two coincident rows survive as two rows, which a positional weld would
    // collapse, and the extra rows are unreferenced.
    let session = Session {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([0.0, 1.0, 0.0]),
            EditVertex::at([0.5, 0.0, 0.0]),
            EditVertex::at([0.5, 0.0, 0.0]),
        ],
        indices: vec![0, 1, 2],
    };

    let rebuilt =
        mesh_from_sculpt_session_like(&source, &session).expect("a session commit rebuilds");
    assert_eq!(rebuilt.vertices().len(), 5);
    assert_eq!(rebuilt.indices(), &[0, 1, 2]);
    assert_eq!(
        rebuilt.vertices()[3].position,
        rebuilt.vertices()[4].position
    );
}

#[test]
fn sculpt_commit_preserves_authored_vertex_rows_at_coincident_positions() {
    let authored = vec![
        Vertex::at(Vec3::ZERO)
            .with_normal(Vec3::Z)
            .with_color([20, 30, 40, 255])
            .with_uv([0.1, 0.2]),
        Vertex::at(Vec3::ZERO)
            .with_normal(Vec3::new(0.0, 0.2, 1.0).normalize())
            .with_color([90, 80, 70, 255])
            .with_uv([0.9, 0.8]),
        Vertex::at(Vec3::X)
            .with_normal(Vec3::Z)
            .with_color([1, 2, 3, 255])
            .with_uv([1.0, 0.0]),
        Vertex::at(Vec3::Y)
            .with_normal(Vec3::NEG_Z)
            .with_color([4, 5, 6, 255])
            .with_uv([0.0, 1.0]),
    ];
    let indices = vec![0, 2, 3, 1, 3, 2];
    let mut source = Mesh::new_for_preview(
        Some("sculpt authored rows".into()),
        authored.clone(),
        indices.clone(),
    )
    .expect("valid source preserves imported normals");
    source.set_texture(MeshTexture::new(1, 1, vec![12, 34, 56, 255]));

    let generic = Mesh::new(None, authored.clone(), indices.clone())
        .expect("generic mesh repair remains available");
    assert_eq!(generic.vertices()[0].normal, generic.vertices()[1].normal);
    assert_ne!(generic.vertices()[0].normal, authored[0].normal);
    assert_ne!(generic.vertices()[1].normal, authored[1].normal);

    let session = Session {
        vertices: authored
            .iter()
            .map(|vertex| EditVertex {
                position: vertex.position,
                normal: vertex.normal,
                color: vertex.color,
                uv: vertex.uv,
            })
            .collect(),
        indices: indices.clone(),
    };
    let rebuilt = mesh_from_sculpt_session_like(&source, &session)
        .expect("sculpt commit preserves the kernel's rows");

    assert_eq!(rebuilt.vertices(), authored);
    assert_eq!(rebuilt.indices(), indices);
    assert_eq!(rebuilt.name(), source.name());
    assert_eq!(
        rebuilt.texture().map(|texture| &texture.rgba),
        source.texture().map(|texture| &texture.rgba)
    );
    assert_ne!(rebuilt.topology_id(), source.topology_id());
    assert_ne!(rebuilt.geometry_id(), source.geometry_id());
}

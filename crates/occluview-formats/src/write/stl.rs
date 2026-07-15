use super::{
    mesh_vertex_position, triangle_normal, write_f32_le, MeshWriteFormat, MeshWriteOptions,
    MeshWriteReport, MeshWriteWarning,
};
use crate::error::FormatError;
use occluview_core::{Mesh, MeshKind};
use std::io::Write;

pub(super) fn write_mesh<W: Write>(
    writer: &mut W,
    mesh: &Mesh,
    _options: MeshWriteOptions,
    report: &mut MeshWriteReport,
) -> Result<(), FormatError> {
    if mesh.kind() != MeshKind::TriangleMesh {
        report.warn(MeshWriteWarning::PointCloudRejectedForStl);
        return Err(FormatError::Malformed {
            format: MeshWriteFormat::StlBinary.label(),
            offset: 0,
            reason: "STL export requires a triangle mesh; point clouds are not supported"
                .to_string(),
        });
    }

    if mesh.has_vertex_colors() {
        report.warn(MeshWriteWarning::VertexColorsNotWritten);
    }
    if mesh.has_uvs() {
        report.warn(MeshWriteWarning::UvsNotWritten);
    }
    if mesh.texture().is_some() {
        report.warn(MeshWriteWarning::TextureImageNotWritten);
    }

    let triangle_count =
        u32::try_from(mesh.triangle_count()).map_err(|_| FormatError::Malformed {
            format: MeshWriteFormat::StlBinary.label(),
            offset: 0,
            reason: "triangle count exceeds STL limit".to_string(),
        })?;

    let mut header = [0_u8; 80];
    let label = b"OccluView binary STL export";
    header[..label.len()].copy_from_slice(label);
    writer.write_all(&header)?;
    writer.write_all(&triangle_count.to_le_bytes())?;

    for triangle in mesh.indices().chunks_exact(3) {
        let a = mesh_vertex_position(mesh, triangle[0])?;
        let b = mesh_vertex_position(mesh, triangle[1])?;
        let c = mesh_vertex_position(mesh, triangle[2])?;
        let normal = triangle_normal(a, b, c);
        for value in normal.to_array() {
            write_f32_le(writer, value)?;
        }
        for value in a.to_array() {
            write_f32_le(writer, value)?;
        }
        for value in b.to_array() {
            write_f32_le(writer, value)?;
        }
        for value in c.to_array() {
            write_f32_le(writer, value)?;
        }
        writer.write_all(&[0, 0])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{Mesh, Vertex};

    fn triangle_mesh() -> Mesh {
        Mesh::new(
            Some("sample".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("sample mesh")
    }

    #[test]
    fn rejects_point_clouds() {
        let mesh = Mesh::point_cloud(
            Some("cloud".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            ],
        );
        let mut bytes = Vec::new();
        let mut report = MeshWriteReport {
            format: MeshWriteFormat::StlBinary,
            vertices: 3,
            triangles: 0,
            warnings: Vec::new(),
        };

        let error = write_mesh(&mut bytes, &mesh, MeshWriteOptions::default(), &mut report)
            .expect_err("point cloud should be rejected");
        assert!(error.to_string().contains("triangle mesh"));
        assert!(report
            .warnings
            .contains(&MeshWriteWarning::PointCloudRejectedForStl));
    }

    #[test]
    fn writes_expected_binary_length_for_one_triangle() {
        let mesh = triangle_mesh();
        let mut bytes = Vec::new();
        let report = crate::write::write_mesh(
            &mut bytes,
            &mesh,
            MeshWriteFormat::StlBinary,
            MeshWriteOptions::default(),
        )
        .expect("write stl");

        assert_eq!(report.format, MeshWriteFormat::StlBinary);
        assert_eq!(report.vertices, 3);
        assert_eq!(report.triangles, 1);
        assert_eq!(bytes.len(), 80 + 4 + 50);
        assert_eq!(&bytes[80..84], &1u32.to_le_bytes());
    }
}

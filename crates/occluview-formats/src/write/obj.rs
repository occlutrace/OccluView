use super::{fmt_f32, sanitize_obj_name, MeshWriteOptions, MeshWriteReport, MeshWriteWarning};
use crate::error::FormatError;
use occluview_core::{Mesh, MeshKind};
use std::io::Write;

pub(super) fn write_mesh<W: Write>(
    writer: &mut W,
    mesh: &Mesh,
    options: MeshWriteOptions,
    report: &mut MeshWriteReport,
) -> Result<(), FormatError> {
    if mesh.has_vertex_colors() && !options.include_vertex_colors {
        report.warn(MeshWriteWarning::VertexColorsNotWritten);
    }
    if mesh.has_uvs() && !options.include_uvs {
        report.warn(MeshWriteWarning::UvsNotWritten);
    }
    if mesh.texture().is_some() {
        report.warn(MeshWriteWarning::TextureImageNotWritten);
    }

    if let Some(name) = mesh.name() {
        writeln!(writer, "o {}", sanitize_obj_name(name))?;
    } else {
        writeln!(writer, "o OccluViewExport")?;
    }

    for vertex in mesh.vertices() {
        if options.include_vertex_colors && mesh.has_vertex_colors() {
            writeln!(
                writer,
                "v {} {} {} {} {} {}",
                fmt_f32(vertex.position[0]),
                fmt_f32(vertex.position[1]),
                fmt_f32(vertex.position[2]),
                vertex.color[0],
                vertex.color[1],
                vertex.color[2],
            )?;
        } else {
            writeln!(
                writer,
                "v {} {} {}",
                fmt_f32(vertex.position[0]),
                fmt_f32(vertex.position[1]),
                fmt_f32(vertex.position[2]),
            )?;
        }
    }

    if options.include_uvs && mesh.has_uvs() {
        for vertex in mesh.vertices() {
            writeln!(
                writer,
                "vt {} {}",
                fmt_f32(vertex.uv[0]),
                fmt_f32(vertex.uv[1]),
            )?;
        }
    }

    let include_normals = options.include_normals;
    if include_normals {
        for vertex in mesh.vertices() {
            writeln!(
                writer,
                "vn {} {} {}",
                fmt_f32(vertex.normal[0]),
                fmt_f32(vertex.normal[1]),
                fmt_f32(vertex.normal[2]),
            )?;
        }
    }

    if mesh.kind() == MeshKind::TriangleMesh {
        for triangle in mesh.indices().as_chunks::<3>().0 {
            let a = triangle[0] + 1;
            let b = triangle[1] + 1;
            let c = triangle[2] + 1;
            match (options.include_uvs && mesh.has_uvs(), include_normals) {
                (true, true) => writeln!(writer, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?,
                (true, false) => writeln!(writer, "f {a}/{a} {b}/{b} {c}/{c}")?,
                (false, true) => writeln!(writer, "f {a}//{a} {b}//{b} {c}//{c}")?,
                (false, false) => writeln!(writer, "f {a} {b} {c}")?,
            }
        }
    } else if !mesh.vertices().is_empty() {
        // OBJ's point element is the lossless counterpart of our point-cloud
        // mesh. Omitting it used to produce a file that looked successful but
        // loaded back with zero vertices because the reader only materialized
        // vertices while parsing faces.
        write!(writer, "p")?;
        for index in 1..=mesh.vertices().len() {
            write!(writer, " {index}")?;
        }
        writeln!(writer)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshWriteFormat;
    use occluview_core::{MeshTexture, Vertex};
    use std::str;

    fn sample_triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new(
            Some("sample".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0))
                    .with_normal(glam::Vec3::new(0.0, 0.0, 1.0))
                    .with_color([210, 180, 120, 255])
                    .with_uv([0.0, 0.0]),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0))
                    .with_normal(glam::Vec3::new(0.0, 0.0, 1.0))
                    .with_color([220, 170, 110, 255])
                    .with_uv([1.0, 0.0]),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0))
                    .with_normal(glam::Vec3::new(0.0, 0.0, 1.0))
                    .with_color([230, 160, 100, 255])
                    .with_uv([0.0, 1.0]),
            ],
            vec![0, 1, 2],
        )
        .expect("sample mesh");
        mesh.set_texture(MeshTexture::white_1x1());
        mesh
    }

    #[test]
    fn preserves_uv_face_indexing_and_warns_on_texture_loss() {
        let mesh = sample_triangle_mesh();
        let mut bytes = Vec::new();
        let written = crate::write::write_mesh(
            &mut bytes,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("write obj");
        assert_eq!(written.format, MeshWriteFormat::Obj);
        assert_eq!(written.vertices, 3);
        assert_eq!(written.triangles, 1);
        assert_eq!(
            written.warnings,
            vec![MeshWriteWarning::TextureImageNotWritten]
        );

        let text = str::from_utf8(&bytes).expect("obj text");
        assert!(text.contains("vt 0.000000 0.000000"));
        assert!(text.contains("f 1/1/1 2/2/2 3/3/3"));
    }

    #[test]
    fn preserves_point_cloud_vertices_with_obj_point_elements() {
        let mesh = Mesh::point_cloud(
            Some("cloud".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(1.0, 2.0, 3.0))
                    .with_normal(glam::Vec3::Z)
                    .with_color([11, 22, 33, 255]),
                Vertex::at(glam::Vec3::new(4.0, 5.0, 6.0)).with_normal(glam::Vec3::Z),
            ],
        );
        let mut bytes = Vec::new();
        crate::write::write_mesh(
            &mut bytes,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("write point-cloud obj");
        let text = str::from_utf8(&bytes).expect("obj text");
        assert!(text.contains("p 1 2"));

        let round_trip = crate::obj::read(&bytes).expect("read point-cloud obj");
        assert!(round_trip.is_point_cloud());
        assert_eq!(round_trip.vertices().len(), 2);
        assert_eq!(
            round_trip.vertices()[0].position,
            mesh.vertices()[0].position
        );
        assert_eq!(round_trip.vertices()[0].color, [11, 22, 33, 255]);
    }
}

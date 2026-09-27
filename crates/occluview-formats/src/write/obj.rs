use super::vertex_color::baked_colors;
use super::{sanitize_obj_name, FmtF32, MeshWriteOptions, MeshWriteReport, MeshWriteWarning};
use crate::error::FormatError;
use occluview_core::{Mesh, MeshKind};
use std::io::Write;

pub(super) fn write_mesh<W: Write>(
    writer: &mut W,
    mesh: &Mesh,
    options: MeshWriteOptions,
    report: &mut MeshWriteReport,
) -> Result<(), FormatError> {
    // OBJ cannot hold an image, so an attached atlas is baked into per-vertex
    // RGB. See `vertex_color`.
    let baked = baked_colors(mesh, &options);
    let write_colors =
        options.include_vertex_colors && (baked.is_some() || mesh.has_vertex_colors());
    // No image means no `vt`: a reader that sees `v/vt` faces and no image
    // enters an empty texture mode and draws the scan white. A scan with no
    // atlas keeps its coordinates.
    let write_uvs = options.include_uvs && mesh.has_uvs() && baked.is_none();
    // The baked atlas is what the renderer shows by default, so it is the
    // colour written. A mesh that also carries its own per-vertex colours (a
    // painted or occlusion-marked scan) loses them, and has to say so.
    if mesh.has_vertex_colors() && (!options.include_vertex_colors || baked.is_some()) {
        report.warn(MeshWriteWarning::VertexColorsNotWritten);
    }
    if mesh.has_uvs() && !write_uvs {
        report.warn(MeshWriteWarning::UvsNotWritten);
    }
    if mesh.texture().is_some() && baked.is_none() {
        report.warn(MeshWriteWarning::TextureImageNotWritten);
    }

    if let Some(name) = mesh.name() {
        writeln!(writer, "o {}", sanitize_obj_name(name))?;
    } else {
        writeln!(writer, "o OccluViewExport")?;
    }

    // OBJ carries RGB per vertex and nothing for alpha. PLY and the app's mesh
    // keep RGBA, and every writer reports a lossy conversion, so alpha < 255
    // exported to OBJ raises a warning.
    let alpha_present =
        write_colors
            && mesh.vertices().iter().enumerate().any(|(index, vertex)| {
                baked.as_ref().map_or(vertex.color[3], |b| b[index][3]) != 255
            });
    for (index, vertex) in mesh.vertices().iter().enumerate() {
        if write_colors {
            let color = baked.as_ref().map_or(vertex.color, |baked| baked[index]);
            writeln!(
                writer,
                "v {} {} {} {} {} {}",
                FmtF32(vertex.position[0]),
                FmtF32(vertex.position[1]),
                FmtF32(vertex.position[2]),
                color[0],
                color[1],
                color[2],
            )?;
        } else {
            writeln!(
                writer,
                "v {} {} {}",
                FmtF32(vertex.position[0]),
                FmtF32(vertex.position[1]),
                FmtF32(vertex.position[2]),
            )?;
        }
    }

    if write_uvs {
        for vertex in mesh.vertices() {
            writeln!(
                writer,
                "vt {} {}",
                FmtF32(vertex.uv[0]),
                FmtF32(vertex.uv[1]),
            )?;
        }
    }

    let include_normals = options.include_normals;
    if include_normals {
        for vertex in mesh.vertices() {
            writeln!(
                writer,
                "vn {} {} {}",
                FmtF32(vertex.normal[0]),
                FmtF32(vertex.normal[1]),
                FmtF32(vertex.normal[2]),
            )?;
        }
    }

    if mesh.kind() == MeshKind::TriangleMesh {
        for triangle in mesh.indices().as_chunks::<3>().0 {
            let a = triangle[0] + 1;
            let b = triangle[1] + 1;
            let c = triangle[2] + 1;
            match (write_uvs, include_normals) {
                (true, true) => writeln!(writer, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?,
                (true, false) => writeln!(writer, "f {a}/{a} {b}/{b} {c}/{c}")?,
                (false, true) => writeln!(writer, "f {a}//{a} {b}//{b} {c}//{c}")?,
                (false, false) => writeln!(writer, "f {a} {b} {c}")?,
            }
        }
    } else if !mesh.vertices().is_empty() {
        // OBJ's point element is the lossless counterpart of our point-cloud
        // mesh. Without it a vertex-only file loads back with zero vertices in
        // readers that only materialize vertices referenced by an element.
        write!(writer, "p")?;
        for index in 1..=mesh.vertices().len() {
            write!(writer, " {index}")?;
        }
        writeln!(writer)?;
    }

    if alpha_present {
        report.warn(MeshWriteWarning::VertexAlphaNotWritten);
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

    /// An atlas is baked into per-vertex RGB, so an OBJ export keeps the
    /// scan's colour in its one file instead of dropping it. The sample mesh
    /// also carries its own colours, which the atlas replaces, so the loss is
    /// reported rather than passing silently. OBJ has no alpha; this white 1x1
    /// atlas leaves every vertex opaque.
    #[test]
    fn bakes_an_atlas_into_vertex_colour_and_reports_the_colour_it_replaces() {
        let mesh = sample_triangle_mesh();
        assert!(
            mesh.has_vertex_colors(),
            "the fixture must carry colours for this test to mean anything"
        );
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
            vec![
                MeshWriteWarning::VertexColorsNotWritten,
                MeshWriteWarning::UvsNotWritten
            ],
            "the export says what the atlas replaced and the coordinates it left behind"
        );

        let text = str::from_utf8(&bytes).expect("obj text");
        // The white atlas samples to white on every vertex.
        assert!(
            text.contains("v 0.000000 0.000000 0.000000 255 255 255"),
            "the baked colour goes on the vertex:\n{text}"
        );
        assert!(
            !text.contains("210 180 120"),
            "the replaced colour is not written as well:\n{text}"
        );
        // No image travels, so `vt` would reference nothing and pull a reader
        // into an empty texture mode that draws the scan white.
        assert!(
            !text.contains("vt "),
            "no dangling texture coordinates are written:\n{text}"
        );
        assert!(text.contains("f 1//1 2//2 3//3"));
    }

    /// A mesh whose own colours are the only colour keeps them: with no atlas
    /// there is nothing to bake, and the vertex colours go out unchanged.
    #[test]
    fn keeps_vertex_colours_when_there_is_no_atlas() {
        let mut mesh = sample_triangle_mesh();
        mesh.clear_texture();
        let mut bytes = Vec::new();
        let written = crate::write::write_mesh(
            &mut bytes,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("write obj");
        assert!(
            written.warnings.is_empty(),
            "nothing was lost: {:?}",
            written.warnings
        );
        let text = str::from_utf8(&bytes).expect("obj text");
        assert!(text.contains("v 0.000000 0.000000 0.000000 210 180 120"));
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

    /// Coordinate formatting must preserve rounding and negative zero.
    #[test]
    fn coordinate_rendering_is_pinned() {
        let mesh = Mesh::new(
            Some("pin".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, -0.0, 1.0 / 3.0)).with_uv([0.0, 1.0 / 7.0]),
                Vertex::at(glam::Vec3::new(-1.0, 2.123_456_7, -1e-7)),
                Vertex::at(glam::Vec3::new(12_345.679, f32::MIN_POSITIVE, 1.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("pin mesh");
        let mut bytes = Vec::new();
        crate::write::write_mesh(
            &mut bytes,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("write obj");
        let text = String::from_utf8(bytes).expect("utf-8");

        assert!(text.contains("v 0.000000 -0.000000 0.333333"), "{text}");
        assert!(text.contains("v -1.000000 2.123457 -0.000000"), "{text}");
        assert!(text.contains("vt 0.000000 0.142857"), "{text}");
        assert!(text.contains("v 12345.678711 0.000000 1.000000"), "{text}");
    }
}

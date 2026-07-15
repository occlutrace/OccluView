//! Shared mesh export writers.
//!
//! The writer contract is intentionally smaller than the reader surface: only
//! the export formats used by the CLI go through here, and each writer reports
//! lossy conversions explicitly via [`MeshWriteWarning`].

mod obj;
mod ply;
mod stl;

use crate::error::FormatError;
use occluview_core::{Mesh, MeshKind};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

/// Mesh export format supported by the shared writer contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MeshWriteFormat {
    /// Binary STL. Triangle mesh only.
    StlBinary,
    /// Binary little-endian PLY.
    PlyBinaryLittleEndian,
    /// Wavefront OBJ.
    Obj,
}

impl MeshWriteFormat {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::StlBinary => "STL",
            Self::PlyBinaryLittleEndian => "PLY",
            Self::Obj => "OBJ",
        }
    }
}

/// Options that control which optional mesh payloads are written.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MeshWriteOptions {
    /// Write per-vertex normals when the format supports them.
    pub include_normals: bool,
    /// Write per-vertex RGBA colors when the format supports them.
    pub include_vertex_colors: bool,
    /// Write UV coordinates when the format supports them.
    pub include_uvs: bool,
}

impl Default for MeshWriteOptions {
    fn default() -> Self {
        Self {
            include_normals: true,
            include_vertex_colors: true,
            include_uvs: true,
        }
    }
}

/// Non-fatal export warnings emitted by the writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeshWriteWarning {
    /// STL does not support point clouds.
    PointCloudRejectedForStl,
    /// Vertex colors were present but not written.
    VertexColorsNotWritten,
    /// UVs were present but not written.
    UvsNotWritten,
    /// A texture image was attached but not written.
    TextureImageNotWritten,
}

/// Summary of a successful mesh write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshWriteReport {
    /// The format that was written.
    pub format: MeshWriteFormat,
    /// Number of vertices emitted.
    pub vertices: usize,
    /// Number of triangles emitted.
    pub triangles: usize,
    /// Any non-fatal warnings produced during the write.
    pub warnings: Vec<MeshWriteWarning>,
}

impl MeshWriteReport {
    fn new(format: MeshWriteFormat, mesh: &Mesh) -> Self {
        Self {
            format,
            vertices: mesh.vertices().len(),
            triangles: if mesh.kind() == MeshKind::TriangleMesh {
                mesh.triangle_count()
            } else {
                0
            },
            warnings: Vec::new(),
        }
    }

    pub(super) fn warn(&mut self, warning: MeshWriteWarning) {
        self.warnings.push(warning);
    }
}

/// Write a mesh to any `Write` sink.
///
/// # Errors
///
/// Returns a [`FormatError`] if the sink fails, the mesh cannot be written in
/// the requested format, or a format-specific value overflows the target
/// representation.
pub fn write_mesh<W: Write>(
    writer: &mut W,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    let mut report = MeshWriteReport::new(format, mesh);
    match format {
        MeshWriteFormat::StlBinary => stl::write_mesh(writer, mesh, options, &mut report)?,
        MeshWriteFormat::PlyBinaryLittleEndian => {
            ply::write_mesh(writer, mesh, options, &mut report)?;
        }
        MeshWriteFormat::Obj => obj::write_mesh(writer, mesh, options, &mut report)?,
    }
    Ok(report)
}

/// Write a mesh to a newly-created file.
///
/// Uses `create_new` semantics so an existing path is treated as an error.
///
/// # Errors
///
/// Returns a [`FormatError`] if the file cannot be opened or the write fails.
pub fn write_mesh_to_new_file(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    write_mesh_file(path, mesh, format, options, true)
}

/// Write a mesh to a file, truncating any existing content.
///
/// # Errors
///
/// Returns a [`FormatError`] if the file cannot be opened or the write fails.
pub fn write_mesh_overwrite(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    write_mesh_file(path, mesh, format, options, false)
}

fn write_mesh_file(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
    create_new: bool,
) -> Result<MeshWriteReport, FormatError> {
    let file = if create_new {
        OpenOptions::new().write(true).create_new(true).open(path)?
    } else {
        File::create(path)?
    };
    let mut writer = BufWriter::new(file);
    let report = write_mesh(&mut writer, mesh, format, options)?;
    writer.flush()?;
    Ok(report)
}

pub(super) fn write_f32_le(writer: &mut impl Write, value: f32) -> Result<(), FormatError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

pub(super) fn write_i32_le(writer: &mut impl Write, value: i32) -> Result<(), FormatError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

pub(super) fn mesh_vertex_position(mesh: &Mesh, index: u32) -> Result<glam::Vec3, FormatError> {
    let vertex = mesh
        .vertices()
        .get(index as usize)
        .ok_or_else(|| FormatError::Malformed {
            format: "mesh export",
            offset: 0,
            reason: format!("mesh index {index} out of range during export"),
        })?;
    Ok(glam::Vec3::from_array(vertex.position))
}

pub(super) fn triangle_normal(a: glam::Vec3, b: glam::Vec3, c: glam::Vec3) -> glam::Vec3 {
    let normal = (b - a).cross(c - a);
    if normal.is_finite() && normal.length_squared() > f32::EPSILON {
        normal.normalize()
    } else {
        glam::Vec3::Z
    }
}

pub(super) fn sanitize_obj_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "OccluViewExport".to_string();
    }
    trimmed
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

pub(super) fn fmt_f32(value: f32) -> String {
    format!("{value:.6}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{Mesh, Vertex};
    use tempfile::NamedTempFile;

    fn triangle_mesh() -> Mesh {
        Mesh::new(
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
        .expect("sample mesh")
    }

    #[test]
    fn overwrite_semantics_truncate_existing_file() {
        let mesh = triangle_mesh();
        let file = NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), b"stale bytes").expect("seed file");

        let report = write_mesh_overwrite(
            file.path(),
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("overwrite");

        assert_eq!(report.format, MeshWriteFormat::Obj);
        let bytes = std::fs::read(file.path()).expect("read back");
        assert!(!bytes.starts_with(b"stale bytes"));
    }
}

//! Shared mesh export writers.
//!
//! The writer contract is intentionally smaller than the reader surface: only
//! the export formats used by the CLI go through here, and each writer reports
//! lossy conversions explicitly via [`MeshWriteWarning`].

mod file;
mod obj;
mod ply;
mod stl;
mod vertex_color;

pub use file::{resolve_overwrite_destination, write_mesh_overwrite, write_mesh_to_new_file};

use crate::error::FormatError;
use occluview_core::{Mesh, MeshKind};
use std::io::Write;

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
// Four independent yes/no choices rather than a state: each one is a property
// of the requested export, so a struct of flags models them directly.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshWriteOptions {
    /// Write per-vertex normals when the format supports them.
    pub include_normals: bool,
    /// Write per-vertex RGBA colors when the format supports them.
    pub include_vertex_colors: bool,
    /// Write UV coordinates when the format supports them.
    pub include_uvs: bool,
    /// Sample an attached texture onto the vertices when the format cannot
    /// hold an image (PLY, OBJ).
    pub include_texture: bool,
}

impl Default for MeshWriteOptions {
    fn default() -> Self {
        Self {
            include_normals: true,
            include_vertex_colors: true,
            include_uvs: true,
            include_texture: true,
        }
    }
}

/// Non-fatal export warnings emitted by the writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeshWriteWarning {
    /// The mesh's own per-vertex colors were present but not written, either
    /// because colors were excluded or because an atlas was baked over them.
    VertexColorsNotWritten,
    /// UVs were present but not written.
    UvsNotWritten,
    /// A texture image was attached but could not be carried.
    TextureImageNotWritten,
    /// Per-vertex alpha was present but the format carries only RGB.
    VertexAlphaNotWritten,
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
    ensure_format_can_represent(mesh, format, &options)?;
    write_mesh_unchecked(writer, mesh, format, options)
}

fn write_mesh_unchecked<W: Write>(
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

/// Reject a mesh the requested format cannot represent, before any file is
/// touched.
///
/// `File::create` truncates, so a rejection discovered inside the writer would
/// leave the destination at zero bytes: exporting a point-cloud layer as `.stl`
/// over an existing scan would destroy that scan and return an error. The only
/// such rejection is STL's, and it depends on the mesh kind alone, so it can be
/// answered before opening anything.
fn ensure_format_can_represent(
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: &MeshWriteOptions,
) -> Result<(), FormatError> {
    let malformed = |reason: &str| FormatError::Malformed {
        format: format.label(),
        offset: 0,
        reason: reason.to_owned(),
    };
    if mesh.vertices().is_empty() {
        return Err(malformed("mesh contains no vertices"));
    }
    if mesh
        .vertices()
        .iter()
        .any(|vertex| vertex.position.iter().any(|value| !value.is_finite()))
    {
        return Err(malformed("mesh contains a non-finite vertex position"));
    }
    if options.include_normals
        && mesh
            .vertices()
            .iter()
            .any(|vertex| vertex.normal.iter().any(|value| !value.is_finite()))
    {
        return Err(malformed("mesh contains a non-finite vertex normal"));
    }
    if options.include_uvs
        && mesh.has_uvs()
        && mesh
            .vertices()
            .iter()
            .any(|vertex| vertex.uv.iter().any(|value| !value.is_finite()))
    {
        return Err(malformed("mesh contains a non-finite texture coordinate"));
    }
    if mesh.kind() == MeshKind::TriangleMesh {
        if !mesh.indices().len().is_multiple_of(3) {
            return Err(malformed("triangle index count is not a multiple of three"));
        }
        let vertex_count = u32::try_from(mesh.vertices().len())
            .map_err(|_| malformed("mesh has more vertices than 32-bit indices allow"))?;
        if mesh.indices().iter().any(|index| *index >= vertex_count) {
            return Err(malformed("triangle index is outside the vertex array"));
        }
    }
    if format == MeshWriteFormat::StlBinary && mesh.kind() != MeshKind::TriangleMesh {
        return Err(malformed(
            "STL export requires a triangle mesh; point clouds are not supported",
        ));
    }
    Ok(())
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
    let a = a.as_dvec3();
    let normal = (b.as_dvec3() - a).cross(c.as_dvec3() - a);
    if normal.is_finite() && normal.length_squared() > 0.0 {
        normal.normalize().as_vec3()
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

/// Format an OBJ coordinate with six decimal places without allocating.
pub(super) struct FmtF32(pub(super) f32);

impl std::fmt::Display for FmtF32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.6}", self.0)
    }
}

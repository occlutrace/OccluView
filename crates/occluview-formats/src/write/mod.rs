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
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
// of the export the operator asked for, so a struct of flags is the honest
// shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshWriteOptions {
    /// Write per-vertex normals when the format supports them.
    pub include_normals: bool,
    /// Write per-vertex RGBA colors when the format supports them.
    pub include_vertex_colors: bool,
    /// Write UV coordinates when the format supports them.
    pub include_uvs: bool,
    /// Write a texture image when the format carries one.
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
/// A `path` that is a symbolic link is followed: the file it points at receives
/// the new mesh and the link itself survives, which is what truncating writes
/// did before publishing became a rename.
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

/// Reject a mesh the requested format cannot represent, before any file is
/// touched.
///
/// `File::create` truncates, so a rejection discovered inside the writer left
/// the destination at zero bytes: exporting a point-cloud layer as `.stl` over
/// an existing scan destroyed that scan and returned an error having written
/// nothing. The only such rejection is STL's, and it depends on the mesh kind
/// alone, so it can be answered before opening anything.
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
    if format == MeshWriteFormat::Obj
        && options.include_uvs
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

fn write_mesh_file(
    path: &Path,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
    create_new: bool,
) -> Result<MeshWriteReport, FormatError> {
    ensure_format_can_represent(mesh, format, &options)?;
    if create_new {
        // Write beside the destination and publish with a no-replace hard
        // link. Opening the destination with `create_new` first still exposed
        // a partially written file to Explorer and crash recovery; a hard
        // link makes the completed inode visible in one operation while
        // retaining create-new collision semantics.
        let (temporary, file) = create_export_temp(path)?;
        let result = write_mesh_to_file(file, mesh, format, options);
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        if let Err(error) = publish_new_export_file(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error.into());
        }
        return Ok(report);
    }

    // An overwrite is a transaction: the old destination remains readable
    // until the complete new mesh has been flushed and the same-directory
    // rename commits it. Writing the target directly used to turn a disk-full
    // or interrupted export into an empty/partial scan.
    //
    // Resolve a symlink destination first. `rename` replaces the link itself
    // rather than the file it points at, so publishing straight onto the
    // operator's `CASE/upper.ply` shortcut would leave the archive copy
    // untouched while the app reported a successful export. Resolving also
    // keeps the temporary beside the file the rename lands on.
    let destination = resolve_overwrite_destination(path)?;
    let (temporary, file) = create_export_temp(&destination)?;
    let result = write_mesh_to_file(file, mesh, format, options);
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = replace_export_file(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(report)
}

/// Follow a destination symlink chain to the file an overwrite targets.
///
/// A regular file costs one `symlink_metadata` probe and is returned
/// unchanged.
///
/// A chain that is still pointing at a link when the bound runs out - a loop,
/// or deeper than any case-folder shortcut should be - is an error. The publish
/// step is a rename, which would replace the link inode and leave the file it
/// pointed at with the previous geometry while reporting success.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidInput`] when the chain cannot be
/// resolved to a regular path.
fn resolve_overwrite_destination(path: &Path) -> std::io::Result<PathBuf> {
    /// Enough for the "case folder is a link into the archive" layouts this
    /// exists for, without letting a long chain walk somewhere unexpected.
    const MAX_DESTINATION_LINKS: usize = 8;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_DESTINATION_LINKS {
        // A missing path is not an error: `create_export_temp` and the rename
        // create it, and that is what the caller wants for a new file.
        let Ok(metadata) = std::fs::symlink_metadata(&current) else {
            return Ok(current);
        };
        if !metadata.file_type().is_symlink() {
            return Ok(current);
        }
        let target = std::fs::read_link(&current)?;
        current = if target.is_absolute() {
            target
        } else {
            match current.parent() {
                Some(parent) => parent.join(target),
                None => return Ok(current),
            }
        };
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "destination {} is a symbolic-link chain that does not resolve to a file after {MAX_DESTINATION_LINKS} steps",
            path.display()
        ),
    ))
}

fn write_mesh_to_file(
    file: File,
    mesh: &Mesh,
    format: MeshWriteFormat,
    options: MeshWriteOptions,
) -> Result<MeshWriteReport, FormatError> {
    let mut writer = BufWriter::new(file);
    let report = write_mesh_unchecked(&mut writer, mesh, format, options)?;
    writer.flush()?;
    writer
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?
        .sync_all()?;
    Ok(report)
}

static NEXT_EXPORT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn create_export_temp(path: &Path) -> Result<(PathBuf, File), FormatError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("mesh"));
    for _ in 0..16 {
        let id = NEXT_EXPORT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".occluview-{id}.tmp"));
        let temporary = parent.join(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve a temporary export path",
    )
    .into())
}

#[cfg(not(windows))]
fn publish_new_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // `hard_link` fails with AlreadyExists instead of replacing a file, which
    // is the filesystem-level equivalent of the public create-new contract.
    match std::fs::hard_link(temporary, destination) {
        Ok(()) => {
            // The destination now owns the complete inode. A cleanup failure
            // must not turn a successfully published export into a false
            // failure.
            let _ = std::fs::remove_file(temporary);
            Ok(())
        }
        Err(error) if !linkless_publish_required(&error) => Err(error),
        // exFAT, vfat, and link-disabled network mounts have no `link(2)` at
        // all, so the publish step failed for a reason that has nothing to do
        // with the destination name. The create-new contract is about the
        // name, not about how it is claimed: fall back to reserving the
        // destination exclusively and copying the finished bytes in.
        Err(_) => publish_by_exclusive_copy(temporary, destination),
    }
}

/// Whether a failed link-based publish has to take the link-free path.
///
/// A collision is the create-new contract working: it must stay classified so
/// the batch exporter can advance to the next numbered name. Every other
/// failure means the filesystem could not link at all.
#[cfg(not(windows))]
fn linkless_publish_required(error: &std::io::Error) -> bool {
    error.kind() != std::io::ErrorKind::AlreadyExists
}

/// Publish a finished temporary into a reserved destination name, without
/// links.
///
/// This is the fallback for filesystems that cannot hard-link. It keeps the
/// contract that matters — an existing destination is never replaced, and a
/// collision still reports [`std::io::ErrorKind::AlreadyExists`] — at the cost
/// of atomicity: the destination becomes visible while it is being filled, so
/// a crash mid-copy can leave a partial export where the link path would have
/// left none. Refusing to export into a writable folder the operator picked is
/// the worse failure.
#[cfg(not(windows))]
fn publish_by_exclusive_copy(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    let mut source = File::open(temporary)?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = std::io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
    drop(target);
    match copied {
        Ok(()) => {
            let _ = std::fs::remove_file(temporary);
            Ok(())
        }
        Err(error) => {
            // A half-written file must not be mistaken for the export.
            let _ = std::fs::remove_file(destination);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn publish_new_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // MoveFileEx without REPLACE_EXISTING preserves create-new semantics while
    // avoiding the hard-link requirement on FAT, network, and restricted
    // Windows volumes. The temporary file is already complete and is moved
    // within the destination directory.
    move_export_file(temporary, destination, false)
}

#[cfg(not(windows))]
fn replace_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // Both paths are created in the destination directory, so rename is an
    // atomic replacement on the supported Unix filesystems.
    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
fn replace_export_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    move_export_file(temporary, destination, true)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn move_export_file(
    temporary: &Path,
    destination: &Path,
    replace_existing: bool,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut flags = MOVEFILE_WRITE_THROUGH;
    if replace_existing {
        flags |= MOVEFILE_REPLACE_EXISTING;
    }
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            flags,
        )
    }
    // Without `MOVEFILE_REPLACE_EXISTING`, an existing destination fails here
    // with ERROR_ALREADY_EXISTS (or ERROR_FILE_EXISTS). Callers detect a
    // create-new collision by `ErrorKind::AlreadyExists`, so a flat
    // `ErrorKind::Other` made the batch retry treat every collision as a hard
    // failure on Windows. Keep the Win32 text for the operator either way.
    .map_err(|error| {
        let code = error.code();
        if code == ERROR_ALREADY_EXISTS.to_hresult() || code == ERROR_FILE_EXISTS.to_hresult() {
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, error.to_string())
        } else {
            std::io::Error::other(error.to_string())
        }
    })
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

/// Format an OBJ coordinate with six decimal places without allocating.
pub(super) struct FmtF32(pub(super) f32);

impl std::fmt::Display for FmtF32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.6}", self.0)
    }
}

#[cfg(test)]
mod tests {
    /// An export is one file. The image travels inside it, so nothing is
    /// written beside it and nothing has to be kept together with it.
    /// A payload this build does not understand is left alone rather than
    /// decoded on a guess.
    #[test]
    fn a_payload_with_an_unknown_format_is_not_decoded() {
        let header = "ply\nformat ascii 1.0\n\
             comment OccluViewTextureFormat webp\n\
             comment OccluViewTextureBase64 aGVsbG8=\n\
             element vertex 3\n\
             property float x\nproperty float y\nproperty float z\n\
             property float s\nproperty float t\n\
             end_header\n0 0 0 0 1\n1 0 0 1 1\n0 1 0 0 0\n";
        let mesh = crate::ply::read(header.as_bytes()).expect("the mesh still reads");
        assert!(
            mesh.texture().is_none(),
            "an unknown payload is not decoded"
        );
    }

    #[test]
    fn an_exported_ply_carries_its_texture_inside_itself() {
        use occluview_core::{MeshTexture, Vertex};

        let mut mesh = Mesh::new(
            Some("arch".to_string()),
            vec![
                Vertex::at(glam::Vec3::ZERO).with_uv([0.0, 1.0]),
                Vertex::at(glam::Vec3::X).with_uv([1.0, 1.0]),
                Vertex::at(glam::Vec3::Y).with_uv([0.0, 0.0]),
            ],
            vec![0, 1, 2],
        )
        .expect("a triangle mesh");
        mesh.set_texture(MeshTexture::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]));

        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("upper-edited.ply");
        let report = write_mesh_to_new_file(
            &path,
            &mesh,
            MeshWriteFormat::PlyBinaryLittleEndian,
            MeshWriteOptions::default(),
        )
        .expect("write the export");

        assert!(
            !report
                .warnings
                .contains(&MeshWriteWarning::TextureImageNotWritten),
            "the image was written"
        );
        let entries: Vec<String> = std::fs::read_dir(directory.path())
            .expect("read the folder")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["upper-edited.ply".to_string()],
            "an export must leave exactly one file behind, found {entries:?}"
        );

        let bytes = std::fs::read(&path).expect("the exported ply");
        let header_end = bytes
            .windows(b"end_header\n".len())
            .position(|window| window == b"end_header\n")
            .expect("end header");
        let header = String::from_utf8_lossy(&bytes[..header_end]);
        assert!(
            !header.contains("TextureFile"),
            "no image sits beside this file, so nothing may name one:\n{header}"
        );
        assert!(
            header.contains("comment OccluViewTextureFormat png")
                && header.contains("comment OccluViewTextureBase64 "),
            "the image must travel in the header:\n{header}"
        );
        assert!(
            header.contains("property list uchar float texcoord"),
            "the faces must carry the coordinates that apply the image"
        );

        let read = crate::ply::read(&bytes).expect("read the export back");
        let texture = read.texture().expect("the texture came back");
        assert_eq!((texture.width, texture.height), (2, 1));
        assert_eq!(texture.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(read.has_uvs());
    }

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
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.obj");
        std::fs::write(&destination, b"stale bytes").expect("seed file");

        let report = write_mesh_overwrite(
            &destination,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("overwrite");

        assert_eq!(report.format, MeshWriteFormat::Obj);
        let bytes = std::fs::read(&destination).expect("read back");
        assert!(!bytes.starts_with(b"stale bytes"));
    }

    #[test]
    fn overwrite_commits_a_complete_file_without_leaving_a_sibling_temp() {
        let mesh = triangle_mesh();
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.obj");
        std::fs::write(&destination, b"previous export").expect("seed file");

        write_mesh_overwrite(
            &destination,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("overwrite");

        let bytes = std::fs::read(&destination).expect("read complete export");
        assert!(bytes.starts_with(b"o sample\n"));
        assert!(!bytes.starts_with(b"previous export"));
        assert!(std::fs::read_dir(directory.path())
            .expect("read directory")
            .filter_map(Result::ok)
            .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
    }

    #[test]
    fn overwrite_temp_files_are_unique_siblings_of_the_destination() {
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.ply");
        let (first_path, first_file) = create_export_temp(&destination).expect("first temp");
        let (second_path, second_file) = create_export_temp(&destination).expect("second temp");
        drop(first_file);
        drop(second_file);

        assert_ne!(first_path, second_path);
        assert_eq!(first_path.parent(), destination.parent());
        assert_eq!(second_path.parent(), destination.parent());
        std::fs::remove_file(first_path).expect("remove first temp");
        std::fs::remove_file(second_path).expect("remove second temp");
    }

    #[test]
    fn new_file_publishes_only_the_complete_export() {
        let mesh = triangle_mesh();
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.obj");

        let report = write_mesh_to_new_file(
            &destination,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("new export");

        assert_eq!(report.format, MeshWriteFormat::Obj);
        assert!(std::fs::read(&destination)
            .expect("read complete export")
            .starts_with(b"o sample\n"));
        assert!(std::fs::read_dir(directory.path())
            .expect("read directory")
            .filter_map(Result::ok)
            .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
    }

    #[test]
    fn new_file_collision_leaves_the_existing_export_and_no_temp_behind() {
        let mesh = triangle_mesh();
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("scan.obj");
        let seed = b"operator export already exists";
        std::fs::write(&destination, seed).expect("seed destination");

        let result = write_mesh_to_new_file(
            &destination,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        );

        let error = result.expect_err("create-new export must reject collisions");
        // The batch exporter retries the next numbered name when it sees this
        // kind. On Windows the collision is only detectable at publish time,
        // inside `MoveFileExW`, so the classification has to survive the Win32
        // error conversion there.
        assert!(
            matches!(
                &error,
                FormatError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists
            ),
            "a create-new collision must be classified for retry, got {error:?}"
        );
        assert_eq!(std::fs::read(&destination).expect("read seed"), seed);
        assert!(std::fs::read_dir(directory.path())
            .expect("read directory")
            .filter_map(Result::ok)
            .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
    }

    /// Case folders are often symlinks into a lab archive. The publish step is
    /// a rename, and rename replaces the link itself, so an operator overwriting
    /// `CASE/upper.ply` would get a success message while the archive copy kept
    /// the previous geometry.
    #[cfg(unix)]
    #[test]
    fn overwriting_a_symlink_updates_its_target_and_keeps_the_link() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temp directory");
        let target = directory.path().join("archive.obj");
        let link = directory.path().join("case.obj");
        std::fs::write(&target, b"previous scan").expect("seed target");
        symlink(&target, &link).expect("create symlink");

        let mesh = triangle_mesh();
        write_mesh_overwrite(
            &link,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        )
        .expect("overwrite through the link");

        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink(),
            "the operator's link must survive the export"
        );
        assert!(
            std::fs::read(&target)
                .expect("read target")
                .starts_with(b"o sample\n"),
            "the file the link points at must receive the new export"
        );
        assert!(
            std::fs::read_dir(directory.path())
                .expect("read directory")
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains(".occluview-")),
            "a published overwrite leaves no temporary behind"
        );
    }

    /// Some filesystems cannot hard-link at all. The fallback has to keep the
    /// contract that matters there: an existing destination is never replaced,
    /// and a collision is still reported as such so the batch exporter can
    /// advance to the next numbered name. The fallback never runs on Windows,
    /// which has its own no-replace publish path.
    #[cfg(not(windows))]
    #[test]
    fn a_publish_without_link_support_still_never_replaces_a_destination() {
        let directory = tempfile::tempdir().expect("temp directory");
        let temporary = directory.path().join("staged.tmp");
        let destination = directory.path().join("scan.obj");
        std::fs::write(&temporary, b"complete export").expect("stage temporary");

        publish_by_exclusive_copy(&temporary, &destination).expect("first publish");
        assert_eq!(
            std::fs::read(&destination).expect("read export"),
            b"complete export"
        );
        assert!(
            !temporary.exists(),
            "a published export consumes its staged temporary"
        );

        std::fs::write(&temporary, b"second export").expect("stage second temporary");
        let error = publish_by_exclusive_copy(&temporary, &destination)
            .expect_err("an existing destination is never replaced");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::AlreadyExists,
            "a collision must stay classifiable for the batch retry"
        );
        assert_eq!(
            std::fs::read(&destination).expect("read export"),
            b"complete export",
            "the first export survives the second publish"
        );
    }

    /// The fallback must not swallow a real collision: the batch exporter has
    /// to see `AlreadyExists` to advance to the next numbered name, while every
    /// other link failure means the filesystem cannot link at all.
    #[cfg(not(windows))]
    #[test]
    fn a_link_collision_stays_classified_and_other_failures_fall_back() {
        assert!(!linkless_publish_required(&std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "a file with that name exists",
        )));
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::Unsupported,
            std::io::ErrorKind::Other,
        ] {
            assert!(
                linkless_publish_required(&std::io::Error::new(kind, "link unavailable")),
                "{kind:?} means the filesystem could not link, so the fallback applies"
            );
        }
    }

    /// A chain that never reaches a regular file must fail the export instead
    /// of renaming onto a link: the rename would replace the link inode and
    /// leave the file it pointed at with the previous geometry, which is the
    /// silent divergence the symlink resolution exists to prevent. The link is
    /// left exactly as it was.
    #[cfg(unix)]
    #[test]
    fn an_unresolvable_link_chain_fails_instead_of_replacing_the_link() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temp directory");
        let first = directory.path().join("a.obj");
        let second = directory.path().join("b.obj");
        symlink(&second, &first).expect("first link");
        symlink(&first, &second).expect("closing the loop");

        let error = resolve_overwrite_destination(&first)
            .expect_err("a link loop must not resolve to a file");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

        let mesh = triangle_mesh();
        let outcome = write_mesh_overwrite(
            &first,
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        );
        assert!(
            outcome.is_err(),
            "exporting onto an unresolvable link must fail, not report success"
        );
        assert!(
            std::fs::symlink_metadata(&first)
                .expect("link metadata")
                .file_type()
                .is_symlink(),
            "a failed export must leave the link alone"
        );
    }

    #[test]
    fn a_rejected_export_leaves_the_destination_untouched() {
        // A PLY point cloud is a loadable layer, so "export this layer as .stl
        // over an existing scan" is an ordinary action, and the rejection has
        // to land before `File::create` truncates.
        let file = NamedTempFile::new().expect("temp file");
        let seed = b"an existing scan the operator still needs";
        std::fs::write(file.path(), seed).expect("seed destination");

        let cloud = Mesh::point_cloud(
            Some("cloud".to_string()),
            vec![Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0))],
        );
        let result = write_mesh_overwrite(
            file.path(),
            &cloud,
            MeshWriteFormat::StlBinary,
            MeshWriteOptions::default(),
        );

        assert!(result.is_err(), "STL cannot represent a point cloud");
        let after = std::fs::read(file.path()).expect("read back");
        assert_eq!(
            after, seed,
            "a failed export must leave the destination exactly as it found it"
        );
    }

    #[test]
    fn an_empty_mesh_is_rejected_before_touching_the_destination() {
        let file = NamedTempFile::new().expect("temp file");
        let seed = b"previous export";
        std::fs::write(file.path(), seed).expect("seed destination");

        let result = write_mesh_overwrite(
            file.path(),
            &Mesh::empty(),
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        );

        assert!(result.is_err(), "an empty placeholder is not an export");
        assert_eq!(std::fs::read(file.path()).expect("read destination"), seed);
    }

    #[test]
    fn a_non_finite_position_is_rejected_before_touching_the_destination() {
        let file = NamedTempFile::new().expect("temp file");
        let seed = b"previous export";
        std::fs::write(file.path(), seed).expect("seed destination");
        let mesh = Mesh::new(
            Some("bad".to_owned()),
            vec![
                Vertex::at(glam::Vec3::new(f32::NAN, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("shape is valid even though its payload is not");

        let result = write_mesh_overwrite(
            file.path(),
            &mesh,
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        );

        assert!(result.is_err(), "non-finite positions are not exportable");
        assert_eq!(std::fs::read(file.path()).expect("read destination"), seed);
    }

    #[test]
    fn a_point_cloud_still_writes_where_the_format_supports_it() {
        let directory = tempfile::tempdir().expect("temp directory");
        let destination = directory.path().join("cloud.ply");
        let cloud = Mesh::point_cloud(
            Some("cloud".to_string()),
            vec![Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0))],
        );

        let report = write_mesh_overwrite(
            &destination,
            &cloud,
            MeshWriteFormat::PlyBinaryLittleEndian,
            MeshWriteOptions::default(),
        )
        .expect("PLY carries point clouds");

        assert_eq!(report.format, MeshWriteFormat::PlyBinaryLittleEndian);
        assert!(!std::fs::read(&destination).expect("read back").is_empty());
    }

    #[test]
    fn public_sink_entry_point_rejects_an_empty_mesh_before_writing() {
        let mut bytes = Vec::from(b"prefix".as_slice());

        let result = write_mesh(
            &mut bytes,
            &Mesh::empty(),
            MeshWriteFormat::Obj,
            MeshWriteOptions::default(),
        );

        assert!(result.is_err(), "an empty mesh is not an export");
        assert_eq!(bytes, b"prefix");
    }
}

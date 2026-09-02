//! Dispatch a file to its format reader by extension.
//!
//! This is the integration seam used by `occluview-app`, `occluview-cli`, and
//! `occluview-shell`. Each concrete reader is wired in here.

use crate::error::FormatError;
use crate::hps::HpsKeyProvider;
use crate::probe::FormatKind;
use occluview_core::{Mesh, Scene, SceneMesh};
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};

enum FileBytesStorage {
    Mapped(memmap2::Mmap),
    Owned(Vec<u8>),
}

/// File bytes loaded from disk with best-effort memory mapping.
///
/// Callers borrow the bytes via [`FileBytes::as_slice`] without needing to
/// care whether they came from an `mmap` or an owned fallback buffer.
pub struct FileBytes {
    extension: String,
    storage: FileBytesStorage,
}

impl FileBytes {
    /// Return the normalized lowercase extension used for dispatch.
    #[must_use]
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// Borrow the file contents as a byte slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match &self.storage {
            FileBytesStorage::Mapped(mmap) => mmap,
            FileBytesStorage::Owned(bytes) => bytes,
        }
    }

    /// Dispatch the loaded bytes through the canonical format readers.
    ///
    /// # Errors
    /// See [`dispatch_by_extension`].
    pub fn dispatch(&self) -> Result<Mesh, FormatError> {
        self.dispatch_with_key_provider(&crate::hps::NoHpsKeyProvider)
    }

    /// Dispatch the loaded bytes through the canonical format readers with an
    /// HPS key provider.
    ///
    /// # Errors
    /// See [`dispatch_by_extension_with_key_provider`].
    pub fn dispatch_with_key_provider(
        &self,
        key_provider: &dyn HpsKeyProvider,
    ) -> Result<Mesh, FormatError> {
        dispatch_by_extension_with_key_provider(self.extension(), self.as_slice(), key_provider)
    }
}

/// Read `bytes` as the format indicated by `kind`, returning a [`Mesh`].
///
/// # Errors
/// - [`FormatError::Malformed`] for recognized formats whose reader is
///   intentionally deferred (currently 3MF).
pub fn dispatch_by_kind(kind: FormatKind, bytes: &[u8]) -> Result<Mesh, FormatError> {
    dispatch_by_kind_with_key_provider(kind, bytes, &crate::hps::NoHpsKeyProvider)
}

/// Read `bytes` as the format indicated by `kind`, using `key_provider` for
/// encrypted HPS `CE` sources.
///
/// # Errors
/// See [`dispatch_by_kind`].
pub fn dispatch_by_kind_with_key_provider(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    dispatch_by_kind_shaded(kind, bytes, key_provider, crate::MeshShading::Reconstructed)
}

/// As [`dispatch_by_kind_with_key_provider`], choosing how vertex normals are
/// produced.
///
/// Only the three formats a scanner writes take the policy; the rest are read
/// the one way they have always been read, because they are rare enough on the
/// thumbnail path that the plumbing would cost more than the milliseconds.
///
/// # Errors
/// See [`FormatError`].
pub fn dispatch_by_kind_shaded(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    match kind {
        FormatKind::Stl => crate::stl::read_shaded(bytes, shading),
        FormatKind::Ply => crate::ply::read_shaded(bytes, shading),
        FormatKind::Obj => crate::obj::read_shaded(bytes, shading),
        FormatKind::Gltf => crate::gltf::read(bytes),
        FormatKind::Off => crate::off::read(bytes),
        // Implement natively when demand appears.
        FormatKind::Threemf => Err(FormatError::Malformed {
            format: "occluview-formats",
            offset: 0,
            reason: format!("reader for {kind:?} not yet implemented"),
        }),
        FormatKind::Hps => crate::hps::read_with_key_provider(bytes, key_provider),
    }
}

/// Convenience: read `bytes` using the reader selected by file extension.
///
/// # Errors
/// See [`FormatError`] and [`dispatch_by_kind`].
/// Convenience: read `bytes` using the reader selected by file extension.
///
/// **Magic wins over extension.** Real-world dental files are frequently
/// mislabeled (a re-export renames `.stl` to `.ply`, or vice versa). We probe
/// the leading bytes first; only if the magic is silent do we trust the
/// extension. This is the same heuristic `stl`/`ply`/`solid`-byte check that
/// the STL reader uses internally, centralized here so every caller benefits.
///
/// # Errors
/// See [`FormatError`] and [`dispatch_by_kind`].
pub fn dispatch_by_extension(extension: &str, bytes: &[u8]) -> Result<Mesh, FormatError> {
    dispatch_by_extension_with_key_provider(extension, bytes, &crate::hps::NoHpsKeyProvider)
}

/// Convenience: read `bytes` by extension/magic with an HPS key provider.
///
/// # Errors
/// See [`dispatch_by_extension`].
pub fn dispatch_by_extension_with_key_provider(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    dispatch_by_extension_shaded(
        extension,
        bytes,
        key_provider,
        crate::MeshShading::Reconstructed,
    )
}

/// As [`dispatch_by_extension_with_key_provider`], choosing how vertex normals
/// are produced.
///
/// # Errors
/// See [`FormatError`].
pub fn dispatch_by_extension_shaded(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    // Magic-first: if the bytes declare a format, honor it over the extension.
    // `probe` falls back to the extension when the magic is ambiguous (e.g.
    // binary STL with a zero header), so this is safe.
    let kind = match crate::probe::probe(Some(extension), bytes) {
        Ok(kind) => kind,
        // probe only fails when neither magic nor extension match; surface that.
        Err(e) => match e {
            FormatError::Unsupported { .. } => {
                // probe rejected the extension too — preserve the original
                // "unsupported extension" error.
                return Err(FormatError::Unsupported {
                    extension: extension.to_string(),
                });
            }
            other => return Err(other),
        },
    };
    dispatch_by_kind_shaded(kind, bytes, key_provider, shading)
}

fn normalized_extension(path: &Path) -> Result<String, FormatError> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(FormatError::Unsupported {
            extension: String::new(),
        })
}

#[allow(unsafe_code)] // see lib.rs: lone mmap kernel-FFI, behind this helper.
fn read_file_bytes_storage(
    mut file: std::fs::File,
    path: &Path,
) -> Result<FileBytesStorage, FormatError> {
    // mmap is best-effort in two ways: the storage has to be safe to map at
    // all, and the call itself may still fail.
    //
    // SAFETY: memmap2 requires that the mapped file not be modified or
    // truncated while the mapping lives; violating that raises
    // EXCEPTION_IN_PAGE_ERROR or SIGBUS, neither of which unwinds, so no
    // `catch_unwind` above this point can turn it back into an error. The
    // guarantee is obtained by refusing to map anything that can be withdrawn
    // mid-parse -- see `crate::mappable` -- which leaves local, non-removable
    // storage, where a concurrent writer is the only remaining hazard and is
    // out of scope for a read-only viewer.
    if crate::mappable::is_mappable_storage(path) {
        if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
            return Ok(FileBytesStorage::Mapped(mmap));
        }
    }

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(FormatError::Io)?;
    Ok(FileBytesStorage::Owned(bytes))
}

/// Read a file from disk into a byte carrier backed by `mmap` when possible.
///
/// The returned [`FileBytes`] owns either the memory mapping or the fallback
/// byte buffer, so callers can safely inspect the contents without touching
/// `unsafe` or deciding which storage strategy succeeded.
///
/// # Errors
/// - [`FormatError::Io`] if the file cannot be opened or read.
/// - [`FormatError::Unsupported`] when the file has no UTF-8 extension.
pub fn read_file_bytes(path: &Path) -> Result<FileBytes, FormatError> {
    let extension = normalized_extension(path)?;
    let file = std::fs::File::open(path).map_err(FormatError::Io)?;
    let storage = read_file_bytes_storage(file, path)?;
    Ok(FileBytes { extension, storage })
}

/// Read a file from disk via memory-mapping, then dispatch by extension.
///
/// Memory-mapping avoids a full-file `read_to_end` copy for large dental
/// scans (the corpus has 50 MB+ STLs). The mmap is held for the duration of
/// the parse; the returned `Mesh` owns its own vertex/index buffers
/// (decoupled from the mapping), so the file can be closed afterwards.
///
/// Falls back to a regular `read` if mmap fails (e.g. on a pipe or a
/// zero-length file).
///
/// # Errors
/// - [`FormatError::Io`] if the file cannot be opened or mapped.
/// - See [`dispatch_by_extension`] for parse errors.
pub fn read_file(path: &Path) -> Result<Mesh, FormatError> {
    read_file_with_key_provider(path, &crate::hps::NoHpsKeyProvider)
}

/// Read a file from disk via memory-mapping with an HPS key provider.
///
/// # Errors
/// See [`read_file`].
pub fn read_file_with_key_provider(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    read_file_shaded(path, key_provider, crate::MeshShading::Reconstructed)
}

/// As [`read_file_with_key_provider`], choosing how vertex normals are
/// produced.
///
/// # Errors
/// See [`FormatError`].
pub fn read_file_shaded(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    let bytes = read_file_bytes(path)?;
    dispatch_by_extension_shaded(bytes.extension(), bytes.as_slice(), key_provider, shading)
}

/// Read multiple files into a [`Scene`], wrapping each [`Mesh`] in a
/// [`SceneMesh`]. The canonical dental use case is loading an upper + lower
/// arch pair as a two-mesh scene.
///
/// Each mesh is placed at the origin with an identity transform; the caller
/// (app / thumbnail framer) repositions them as needed via `SceneMesh`'s
/// transform field, or just relies on `Scene::bbox()` to frame the union.
///
/// **Fail-fast:** returns the first `(path, error)` pair encountered. The
/// caller decides whether to abort or offer "skip + continue" — for v1 we
/// abort, which keeps the error path simple and predictable.
///
/// # Errors
/// - The `Err` variant carries the path that failed and its [`FormatError`].
pub fn read_files(paths: &[PathBuf]) -> Result<Scene, (PathBuf, FormatError)> {
    read_files_with_key_provider(paths, &crate::hps::NoHpsKeyProvider)
}

/// Read multiple files into a [`Scene`], using an HPS key provider.
///
/// # Errors
/// See [`read_files`].
pub fn read_files_with_key_provider(
    paths: &[PathBuf],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Scene, (PathBuf, FormatError)> {
    let mut scene = Scene::new();
    if let [path] = paths {
        let mesh =
            read_file_with_key_provider(path, key_provider).map_err(|e| (path.clone(), e))?;
        scene.add(SceneMesh::new(mesh));
        return Ok(scene);
    }

    let meshes = paths
        .par_iter()
        .map(|path| read_file_with_key_provider(path, key_provider).map_err(|e| (path.clone(), e)))
        .collect::<Vec<_>>();

    for result in meshes {
        let mesh = result?;
        scene.add(SceneMesh::new(mesh));
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal valid binary STL: 1 triangle in the XY plane, normal +Z.
    fn one_triangle_binary_stl() -> Vec<u8> {
        one_triangle_binary_stl_with_x_offset(0.0)
    }

    fn one_triangle_binary_stl_with_x_offset(x_offset: f32) -> Vec<u8> {
        let mut out = vec![0u8; 84];
        out[80..84].copy_from_slice(&1u32.to_le_bytes());
        let tri: [f32; 12] = [
            0.0,
            0.0,
            1.0, // normal
            x_offset,
            0.0,
            0.0, // v0
            x_offset + 1.0,
            0.0,
            0.0, // v1
            x_offset,
            1.0,
            0.0, // v2
        ];
        for f in tri {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]); // attribute byte count
        out
    }

    fn zip_with_file(path: &str, bytes: &[u8]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            archive.start_file(path, options).expect("start zip file");
            archive.write_all(bytes).expect("write zip file");
            archive.finish().expect("finish zip file");
        }
        cursor.into_inner()
    }

    #[test]
    fn stl_dispatches_and_reads() {
        let bytes = one_triangle_binary_stl();
        let mesh = dispatch_by_extension("stl", &bytes).expect("STL should read");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn unimplemented_reader_returns_malformed() {
        // 3MF is the remaining stub (PK zip magic; content that won't satisfy
        // any implemented reader's parser either, so it routes by extension to
        // the stub arm).
        let res = dispatch_by_extension("3mf", &[0xA5u8; 16]);
        let Err(FormatError::Malformed { reason, .. }) = res else {
            panic!("expected Malformed stub error, got {res:?}");
        };
        assert!(reason.contains("not yet implemented"));
    }

    #[test]
    fn raw_cc_hps_sources_are_parsed() {
        let hps = br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
</HPS>"#;
        let mesh = dispatch_by_extension("hps", hps).expect("raw CC HPS should parse");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.indices(), &[0, 1, 2]);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices()[2].position, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn ce_hps_sources_remain_deferred_until_key_provider_exists() {
        let hps = br"<HPS><Schema>CE</Schema></HPS>";
        let res = dispatch_by_extension("hps", hps);
        assert!(matches!(
            res,
            Err(FormatError::Deferred { format, .. }) if format == "HPS"
        ));

        let zip_hps = zip_with_file("scan/geometry.hps", hps);
        let res = dispatch_by_extension(crate::LEGACY_HPS_EXTENSION, &zip_hps);
        assert!(matches!(
            res,
            Err(FormatError::Deferred { format, .. }) if format == "HPS"
        ));

        let invalid_zip_hps = [0x50, 0x4B, 0x03, 0x04, 0x00, 0x00];
        let res = dispatch_by_extension(crate::LEGACY_HPS_EXTENSION, &invalid_zip_hps);
        assert!(matches!(res, Err(FormatError::Malformed { .. })));
    }

    #[test]
    fn obj_dispatches_and_reads() {
        // A minimal OBJ with one triangle and vertex colors (dental CAD extension).
        let obj = b"v 0 0 0 255 128 0\nv 1 0 0 0 255 0\nv 0 1 0 0 0 255\nf 1 2 3\n";
        let mesh = dispatch_by_extension("obj", obj).expect("OBJ should read");
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.has_vertex_colors());
    }

    #[test]
    fn ply_dispatches_and_reads() {
        // A minimal ASCII PLY with one colored vertex.
        let ply = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 0\nproperty list uchar int vertex_indices\nend_header\n1.0 2.0 3.0 255 128 0\n";
        let mesh = dispatch_by_extension("ply", ply).expect("PLY should read");
        assert_eq!(mesh.vertices().len(), 1);
        assert!(mesh.has_vertex_colors());
    }

    #[test]
    fn mislabeled_extension_falls_back_to_magic() {
        // Real-world case from the OccluTrace corpus: a binary STL renamed to
        // `.ply`. The 80-byte header carries an arbitrary ASCII label
        // ("OccluTrace Native binary STL"); the file is binary STL underneath.
        // Magic-first dispatch must route it to the STL reader, not the PLY
        // reader (which would reject it as bad signature).
        let mut bytes = vec![0u8; 84];
        let label = b"OccluTrace Native binary STL";
        bytes[..label.len()].copy_from_slice(label);
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        // One triangle: normal +Z, three vertices.
        let tri: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        for f in tri {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);

        let mesh = dispatch_by_extension("ply", &bytes).expect("magic wins over extension");
        assert_eq!(mesh.triangle_count(), 1, "STL content must parse as STL");
    }

    #[test]
    fn unknown_extension_is_unsupported() {
        let res = dispatch_by_extension("xyz", &[0u8; 4]);
        assert!(matches!(res, Err(FormatError::Unsupported { .. })));
    }

    #[test]
    fn read_file_mmaps_and_parses() {
        // Write a minimal binary STL to a temp file and read it back via mmap.
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.stl");
        std::fs::write(&path, &bytes).expect("write");
        let mesh = read_file(&path).expect("read_file should mmap + parse");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn read_file_bytes_returns_extension_and_contents() {
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.STL");
        std::fs::write(&path, &bytes).expect("write");

        let file_bytes = read_file_bytes(&path).expect("read file bytes");

        assert_eq!(file_bytes.extension(), "stl");
        assert_eq!(file_bytes.as_slice(), bytes.as_slice());
    }

    #[test]
    fn read_file_bytes_dispatches_with_its_extension() {
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.stl");
        std::fs::write(&path, &bytes).expect("write");

        let file_bytes = read_file_bytes(&path).expect("read file bytes");
        let mesh = file_bytes.dispatch().expect("dispatch file bytes");

        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn read_file_missing_extension_is_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("noext");
        std::fs::write(&path, b"x").expect("write");
        assert!(matches!(
            read_file(&path),
            Err(FormatError::Unsupported { .. })
        ));
    }

    #[test]
    fn read_file_bytes_missing_extension_is_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("noext");
        std::fs::write(&path, b"x").expect("write");
        assert!(matches!(
            read_file_bytes(&path),
            Err(FormatError::Unsupported { .. })
        ));
    }

    #[test]
    fn read_files_preserves_input_layer_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first.stl");
        let second = dir.path().join("second.stl");
        std::fs::write(&first, one_triangle_binary_stl_with_x_offset(0.0)).expect("write first");
        std::fs::write(&second, one_triangle_binary_stl_with_x_offset(10.0)).expect("write second");

        let scene = read_files(&[first, second]).expect("read files");

        assert_eq!(scene.meshes().len(), 2);
        assert_eq!(scene.meshes()[0].mesh.bbox_uncached().min.x, 0.0);
        assert_eq!(scene.meshes()[1].mesh.bbox_uncached().min.x, 10.0);
    }

    #[test]
    fn read_files_single_path_returns_one_mesh_scene() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("single.stl");
        std::fs::write(&path, one_triangle_binary_stl()).expect("write");

        let scene = read_files(&[path]).expect("read files");

        assert_eq!(scene.meshes().len(), 1);
        assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    }
}

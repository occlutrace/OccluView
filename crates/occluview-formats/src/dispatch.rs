//! Dispatch a file to its format reader by extension.
//!
//! This is the integration seam used by `occluview-app`, `occluview-cli`, and
//! `occluview-shell`. Each concrete reader (to be implemented in dedicated PRs
//! per the roadmap) is wired in here.

use crate::error::FormatError;
use crate::probe::FormatKind;
use occluview_core::Mesh;
use std::path::Path;

/// Read `bytes` as the format indicated by `kind`, returning a [`Mesh`].
///
/// # Errors
/// - [`FormatError::Unsupported`] for formats whose reader is not implemented
///   yet (this stub returns that for everything until the readers land).
pub fn dispatch_by_kind(kind: FormatKind, bytes: &[u8]) -> Result<Mesh, FormatError> {
    match kind {
        FormatKind::Stl => crate::stl::read(bytes),
        FormatKind::Ply => crate::ply::read(bytes),
        FormatKind::Obj => crate::obj::read(bytes),
        FormatKind::Gltf => crate::gltf::read(bytes),
        FormatKind::Off => crate::off::read(bytes),
        // TODO(formats/threemf): implement natively when demand appears (ADR-0010 pattern).
        FormatKind::Threemf => Err(FormatError::Malformed {
            format: "occluview-formats",
            offset: 0,
            reason: format!("reader for {kind:?} not yet implemented (see ROADMAP and ADR-0004)"),
        }),
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
    dispatch_by_kind(kind, bytes)
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
#[allow(unsafe_code)] // see lib.rs: lone mmap kernel-FFI, behind this helper.
pub fn read_file(path: &Path) -> Result<Mesh, FormatError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(FormatError::Unsupported {
            extension: String::new(),
        })?;

    let file = std::fs::File::open(path).map_err(FormatError::Io)?;
    // mmap is best-effort: fall back to read if it fails.
    // SAFETY: the file is opened read-only and we hold the File handle for the
    // lifetime of the Mmap; the bytes are parsed synchronously before return.
    if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
        return dispatch_by_extension(&ext, &mmap);
    }
    let bytes = std::fs::read(path).map_err(FormatError::Io)?;
    dispatch_by_extension(&ext, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid binary STL: 1 triangle in the XY plane, normal +Z.
    fn one_triangle_binary_stl() -> Vec<u8> {
        let mut out = vec![0u8; 84];
        out[80..84].copy_from_slice(&1u32.to_le_bytes());
        let tri: [f32; 12] = [
            0.0, 0.0, 1.0, // normal
            0.0, 0.0, 0.0, // v0
            1.0, 0.0, 0.0, // v1
            0.0, 1.0, 0.0, // v2
        ];
        for f in tri {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]); // attribute byte count
        out
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
    fn obj_dispatches_and_reads() {
        // A minimal OBJ with one triangle and vertex colors (exocad extension).
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
    fn read_file_missing_extension_is_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("noext");
        std::fs::write(&path, b"x").expect("write");
        assert!(matches!(
            read_file(&path),
            Err(FormatError::Unsupported { .. })
        ));
    }
}

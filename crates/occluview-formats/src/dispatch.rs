//! Dispatch a file to its format reader by extension.
//!
//! This is the integration seam used by `occluview-app`, `occluview-cli`, and
//! `occluview-shell`. Each concrete reader (to be implemented in dedicated PRs
//! per the roadmap) is wired in here.

use crate::error::FormatError;
use crate::probe::FormatKind;
use occluview_core::Mesh;

/// Read `bytes` as the format indicated by `kind`, returning a [`Mesh`].
///
/// # Errors
/// - [`FormatError::Unsupported`] for formats whose reader is not implemented
///   yet (this stub returns that for everything until the readers land).
pub fn dispatch_by_kind(kind: FormatKind, bytes: &[u8]) -> Result<Mesh, FormatError> {
    match kind {
        FormatKind::Stl => crate::stl::read(bytes),
        FormatKind::Ply => crate::ply::read(bytes),
        // TODO(formats/obj):  implement in `obj` module (+ lenient MTL, fan triangulation).
        // TODO(formats/gltf): implement in `gltf` module via cgltf (zip-slip-safe).
        // TODO(formats/threemf): implement in `threemf` module via lib3mf FFI.
        FormatKind::Obj | FormatKind::Gltf | FormatKind::Threemf => Err(FormatError::Malformed {
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
pub fn dispatch_by_extension(extension: &str, bytes: &[u8]) -> Result<Mesh, FormatError> {
    let kind = crate::probe::by_extension(extension).ok_or(FormatError::Unsupported {
        extension: extension.to_string(),
    })?;
    dispatch_by_kind(kind, bytes)
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
        // PLY is now implemented; use OBJ (still a stub) as the not-yet-implemented
        // sentinel.
        let res = dispatch_by_extension("obj", &[0u8; 84]);
        let Err(FormatError::Malformed { reason, .. }) = res else {
            panic!("expected Malformed stub error, got {res:?}");
        };
        assert!(reason.contains("not yet implemented"));
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
    fn unknown_extension_is_unsupported() {
        let res = dispatch_by_extension("xyz", &[0u8; 4]);
        assert!(matches!(res, Err(FormatError::Unsupported { .. })));
    }
}

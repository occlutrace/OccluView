//! `occluview-formats` — 3D file format readers (and eventually writers).
//!
//! Each format has its own module implementing the [`FormatReader`] trait, so a
//! new format is added by writing a module + registering it in [`dispatch`] — no
//! changes to `core`, `render`, or `app`.
//!
//! ## Invariants
//!
//! - Parsers return [`FormatError`] on malformed input; they never panic.
//! - Path traversal is forbidden before any external resource format is exposed
//!   to the app or shell (`.gltf` JSON and 3MF are deferred from v1 surfaces).
//! - Coordinate-frame conversion to OccluView's Y-up RH frame happens here, not
//!   in the renderer (see [`occluview_core::frame`]).
//! - Units: STL/OBJ declare none (assume mm, surfaced in UI); GLB declares
//!   meters but scanner exports vary, so v1 keeps coordinates unchanged.
//!
//! ## Status
//!
//! v1 open surfaces intentionally expose only implemented, product-approved
//! readers: STL, PLY, OBJ, GLB, and HPS.

// `deny(unsafe_code)` (not `forbid`): the mmap streaming path
// (dispatch::read_file) needs one `unsafe` block for memmap2::Mmap::map,
// which is the audited kernel-FFI for memory-mapping. All format PARSERS
// remain safe; the lone unsafe lives behind the read_file helper.
#![deny(unsafe_code)]
// Test-only relaxation of strict lints; production parser code stays stricter.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_lossless,
        clippy::cast_possible_wrap,
    )
)]

pub mod dispatch;
pub mod error;
pub mod glb_writer;
pub mod gltf;
pub mod hps;
pub mod obj;
pub mod off;
pub mod ply;
pub mod probe;
pub mod stl;
mod texture_decode;
pub mod write;

/// Legacy file extension accepted as an alias for HPS packages.
pub const LEGACY_HPS_EXTENSION: &str = "dcm";

/// File extensions OccluView intentionally exposes in the v1 user-facing
/// open/import surfaces.
///
/// This is narrower than every parser that may exist in the crate: v1 only
/// promises formats that are implemented and product-approved for the native
/// viewer and shell integration.
pub const V1_OPEN_EXTENSIONS: &[&str] = &["stl", "ply", "obj", "glb", "hps", LEGACY_HPS_EXTENSION];

/// The common interface every format reader implements.
///
/// A reader takes a byte stream and produces an [`occluview_core::Mesh`]. The
/// caller decides I/O (file, mmap, in-memory); the reader does not touch the
/// filesystem, which keeps it trivial to fuzz and to reuse in the thumbnail
/// provider.
pub trait FormatReader {
    /// Human-readable format name, e.g. `"STL (binary)"`.
    fn format_name(&self) -> &'static str;

    /// Parse `bytes` into a mesh.
    ///
    /// # Errors
    /// See [`FormatError`].
    fn read(&self, bytes: &[u8]) -> Result<occluview_core::Mesh, FormatError>;
}

pub use dispatch::{dispatch_by_extension, read_file, read_files, read_files_with_key_provider};
pub use error::FormatError;
pub use glb_writer::write_textured_glb;
pub use probe::{probe, FormatKind};
pub use write::{
    write_mesh, write_mesh_overwrite, write_mesh_to_new_file, MeshWriteFormat, MeshWriteOptions,
    MeshWriteReport, MeshWriteWarning,
};

#[cfg(test)]
mod tests {
    use super::{LEGACY_HPS_EXTENSION, V1_OPEN_EXTENSIONS};

    #[test]
    fn v1_open_extensions_match_public_format_promise() {
        assert_eq!(
            V1_OPEN_EXTENSIONS,
            ["stl", "ply", "obj", "glb", "hps", LEGACY_HPS_EXTENSION]
        );

        let readme = include_str!("../../../README.md");
        let public_promise = "`.hps` and `.dcm` - HPS mesh containers";
        assert!(readme.contains(public_promise));
    }
}

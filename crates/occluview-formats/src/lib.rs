//! `occluview-formats` — 3D file format readers (and eventually writers).
//!
//! Each format has its own module implementing the [`FormatReader`] trait, so a
//! new format is added by writing a module + registering it in [`dispatch`] — no
//! changes to `core`, `render`, or `app` (see `ARCHITECTURE.md` §9).
//!
//! ## Invariants
//!
//! - Parsers return [`FormatError`] on malformed input; they never panic.
//! - Path traversal / zip-slip is forbidden for archive formats (glTF-GLB, 3MF).
//! - Coordinate-frame conversion to OccluView's Y-up RH frame happens here, not
//!   in the renderer (see [`occluview_core::frame`]).
//! - Units: STL/OBJ declare none (assume mm, surfaced in UI); glTF declares
//!   meters; 3MF declares units. Each loader normalizes to mm.
//!
//! ## Status
//!
//! This is a stub. The P0 loaders (STL, PLY, OBJ, glTF) land in dedicated PRs
//! per the roadmap, each with property tests and fuzz targets.

#![forbid(unsafe_code)]
// Test-only relaxation of the slop lints — see occluview-core/src/lib.rs.
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
pub mod obj;
pub mod ply;
pub mod probe;
pub mod stl;

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

pub use dispatch::dispatch_by_extension;
pub use error::FormatError;
pub use probe::{probe, FormatKind};

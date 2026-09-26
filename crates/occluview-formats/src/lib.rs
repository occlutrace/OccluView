//! `occluview-formats` — 3D file format readers and writers.
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
//! - Coordinate-frame conversion happens in readers, not in the renderer.
//! - Units: STL/OBJ/PLY/OFF/HPS declare none (read as mm, see
//!   [`units::policy_for`]); GLB declares meters but scanner exports vary,
//!   so coordinates are kept unchanged and the layer is flagged ambiguous
//!   ([`units::UnitInterpretation`]) instead of being silently scaled.
//!
//! ## Status
//!
//! v1 open surfaces intentionally expose only implemented, product-approved
//! readers: STL, PLY, OBJ, GLB, and HPS.

// File input and all format parsers use owned bytes and safe Rust.
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

mod companions;
pub mod dispatch;
pub mod error;
pub mod glb_writer;
pub mod gltf;
pub mod hps;
#[cfg(test)]
mod load_perf_tests;
pub mod obj;
pub mod off;
pub mod ply;
pub mod probe;
pub mod stl;
mod texture_decode;
pub mod units;
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
/// caller decides the I/O — the file loaders in this crate read the whole file
/// into an owned buffer — and the reader never touches the filesystem, which
/// keeps it trivial to fuzz and to reuse in the thumbnail provider.
pub trait FormatReader {
    /// Human-readable format name, e.g. `"STL (binary)"`.
    fn format_name(&self) -> &'static str;

    /// Parse `bytes` into a mesh.
    ///
    /// # Errors
    /// See [`FormatError`].
    fn read(&self, bytes: &[u8]) -> Result<occluview_core::Mesh, FormatError>;
}

/// Whether a reader reconstructs vertex normals or keeps what the file wrote.
///
/// Reading a scan is mostly not parsing: it is
/// [`Mesh::new`](occluview_core::Mesh::new) welding vertices that share a
/// position and averaging normals across each run, so that a scan carrying one
/// flat normal per facet shades smoothly in the viewport. On a 326 000
/// triangle arch that reconstruction is 135 ms of a 172 ms read.
///
/// Nothing drawn at thumbnail or preview-pane size can show the difference --
/// a full arch puts about a tenth of a pixel under each triangle -- so those
/// callers ask for [`MeshShading::AsWritten`] and get the file's own normals,
/// filled in cheaply only where it wrote none.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MeshShading {
    /// Weld and average, for a mesh that will be looked at closely or edited.
    #[default]
    Reconstructed,
    /// Keep the normals in the file; fill in absent ones with one pass.
    AsWritten,
}

impl MeshShading {
    pub(crate) fn build(
        self,
        builder: occluview_core::MeshBuilder,
    ) -> Result<occluview_core::Mesh, occluview_core::CoreError> {
        match self {
            Self::Reconstructed => builder.build(),
            Self::AsWritten => builder.build_for_preview(),
        }
    }
}

pub use dispatch::{
    dispatch_by_extension, read_file, read_file_loaded_with_key_provider, read_files,
    read_files_with_key_provider, LoadedMesh,
};
pub use error::FormatError;
pub use glb_writer::write_textured_glb;
pub use probe::{probe, FormatKind};
pub use write::{
    resolve_overwrite_destination, write_mesh, write_mesh_overwrite, write_mesh_to_new_file,
    MeshWriteFormat, MeshWriteOptions, MeshWriteReport, MeshWriteWarning,
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

        // Pin the claim, not two substrings. "`.hps` and `.dcm` appear
        // somewhere in the file" passes on a README stating the opposite, and
        // `.dcm` appears here several times for other reasons -- including a
        // paragraph about not claiming the extension.
        let readme = include_str!("../../../README.md");
        let promise = readme
            .lines()
            .find(|line| line.starts_with("- `.hps` and `.dcm`"));
        assert!(
            promise.is_some(),
            "the README's supported-format list must carry an .hps/.dcm entry"
        );
        let Some(promise) = promise else {
            return;
        };
        assert!(
            promise.contains("medical DICOM is not supported"),
            "the entry must keep saying that medical DICOM is refused, since \
             the reader refuses it: {promise}"
        );
        assert!(
            promise.contains("DICM"),
            "the entry should name the marker the reader actually tests: {promise}"
        );
    }
}

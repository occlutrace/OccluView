//! Format-error type shared by all readers.

use thiserror::Error;

/// Errors raised by a format reader.
///
/// Variants are narrow and carry context (offset, reason) so the caller can
/// show a useful message and we can write targeted fuzz regression tests.
#[derive(Debug, Error)]
pub enum FormatError {
    /// The first bytes did not match the format's signature / magic.
    #[error("not a {format} file: bad signature at offset {offset}")]
    BadSignature {
        /// The format that was being attempted.
        format: &'static str,
        /// Byte offset where the mismatch was detected.
        offset: usize,
    },

    /// The file was truncated — fewer bytes than the header promised.
    #[error("truncated {format} file: expected {expected} bytes, got {got}")]
    Truncated {
        /// The format that was being read.
        format: &'static str,
        /// Expected total length.
        expected: usize,
        /// Actual length available.
        got: usize,
    },

    /// A structural field had an implausible value (e.g. negative count).
    #[error("malformed {format} at offset {offset}: {reason}")]
    Malformed {
        /// The format that was being read.
        format: &'static str,
        /// Byte offset of the offending field, where known.
        offset: usize,
        /// Human-readable reason.
        reason: String,
    },

    /// A path inside an archive (glTF-GLB / 3MF) tried to escape the file's dir.
    #[error("unsafe path in {format}: {path} attempts directory traversal")]
    UnsafePath {
        /// The format that was being read.
        format: &'static str,
        /// The offending path string.
        path: String,
    },

    /// The extension/magic did not match any known format.
    #[error("unsupported format (extension={extension:?})")]
    Unsupported {
        /// The file extension that was attempted, lowercase, without dot.
        extension: String,
    },

    /// The format was recognized, but its reader is intentionally not exposed yet.
    #[error("{format} support is recognized but not enabled yet: {reason}")]
    Deferred {
        /// The recognized format family.
        format: &'static str,
        /// Human-readable reason.
        reason: String,
    },

    /// An error propagated from `occluview-core` (e.g. bad indices).
    #[error(transparent)]
    Core(#[from] occluview_core::CoreError),

    /// An I/O error from the caller's byte source.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

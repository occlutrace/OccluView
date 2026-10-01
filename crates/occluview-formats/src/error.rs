//! Format-error type shared by all readers.

use thiserror::Error;

/// Errors raised by a format reader.
///
/// Variants are narrow and carry context (offset, reason) so the caller can
/// show a useful message and we can write targeted fuzz tests.
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

    /// The file is larger than this build reads.
    ///
    /// An unbounded read is a resource attack whether or not anyone meant it:
    /// the viewer holds the whole file in memory before parsing it, and a
    /// folder dropped on the window parses several files at once. The limit is
    /// far above any real scan (see `MAX_IMPORT_BYTES`), so reaching it means
    /// the file is not a scan this viewer should open.
    #[error("file is {bytes} bytes, larger than the {limit} byte limit this build reads")]
    TooLarge {
        /// Size of the file that was refused.
        bytes: u64,
        /// The limit it exceeded.
        limit: u64,
    },

    /// The estimated scene and active import exceed the memory budget.
    #[error(
        "import needs an estimated {estimated_bytes} bytes, above the {limit} byte memory limit"
    )]
    MemoryBudgetExceeded {
        /// Estimated bytes held by the scene and active import.
        estimated_bytes: u64,
        /// Maximum bytes allowed for the scene and active import.
        limit: u64,
    },

    /// The extension/magic did not match any known format.
    #[error("unsupported format (extension={extension:?})")]
    Unsupported {
        /// The file extension that was attempted, lowercase, without dot.
        extension: String,
    },

    /// The format was recognized, but this build does not read it.
    ///
    /// Four situations arrive here and one message has to fit all of them: a
    /// fast thumbnail path handing back to the full reader, an encrypted HPS
    /// package with no key configured, a `.gltf` (JSON) file that the GLB
    /// reader declines so the caller exports it as `.glb` instead, and a 3MF
    /// container that has no reader at all. "Not enabled yet" fits the first
    /// but would tell the others that their file type is unsupported, which is
    /// wrong, so the reason is always a sentence the operator can act on.
    #[error("{format} was recognized but not read: {reason}")]
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

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn variant_name(error: &FormatError) -> &'static str {
        match error {
            FormatError::BadSignature { .. } => "BadSignature",
            FormatError::Truncated { .. } => "Truncated",
            FormatError::Malformed { .. } => "Malformed",
            FormatError::UnsafePath { .. } => "UnsafePath",
            FormatError::TooLarge { .. } => "TooLarge",
            FormatError::MemoryBudgetExceeded { .. } => "MemoryBudgetExceeded",
            FormatError::Unsupported { .. } => "Unsupported",
            FormatError::Deferred { .. } => "Deferred",
            FormatError::Core(_) => "Core",
            FormatError::Io(_) => "Io",
        }
    }

    #[test]
    fn every_format_error_variant_renders_an_operator_message() {
        let cause = occluview_core::CoreError::Geometry("a triangle is degenerate".to_string());
        let variants = [
            FormatError::BadSignature {
                format: "STL",
                offset: 0,
            },
            FormatError::Truncated {
                format: "STL",
                expected: 84,
                got: 12,
            },
            FormatError::Malformed {
                format: "PLY",
                offset: 4,
                reason: "vertex count is negative".to_string(),
            },
            FormatError::UnsafePath {
                format: "3MF",
                path: "../outside".to_string(),
            },
            FormatError::TooLarge {
                bytes: 2_147_483_648,
                limit: 1_073_741_824,
            },
            FormatError::MemoryBudgetExceeded {
                estimated_bytes: 2_147_483_648,
                limit: 1_073_741_824,
            },
            FormatError::Unsupported {
                extension: "xyz".to_string(),
            },
            FormatError::Deferred {
                format: "3MF",
                reason: "this build has no 3MF reader".to_string(),
            },
            FormatError::Core(cause),
            FormatError::Io(std::io::Error::other("the source closed early")),
        ];
        for variant in &variants {
            assert!(!variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }
}

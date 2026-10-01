//! Core error type.
//!
//! `occluview-core` is panic-free. Every fallible operation
//! returns one of these variants. The variants stay narrow and well-named so the
//! caller can react appropriately (e.g. a malformed file → user-visible message,
//! not a crash).

use thiserror::Error;

/// Errors raised by `occluview-core`.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A triangle index was outside the vertex array.
    #[error("index out of range at position {at_index}: {value} >= vertex_count {vertex_count}")]
    IndexOutOfRange {
        /// Position in the index array where the bad value was found.
        at_index: usize,
        /// The offending index value.
        value: u32,
        /// Number of vertices available.
        vertex_count: u32,
    },

    /// An index array's length was not a multiple of 3.
    #[error("index count {index_count} is not a multiple of 3")]
    IndexCountNotMultipleOfThree {
        /// The offending length.
        index_count: usize,
    },

    /// A geometry invariant was violated (degenerate triangle, NaN, etc.).
    #[error("geometry invariant violated: {0}")]
    Geometry(String),

    /// The mesh grew past anything the file it came from could describe.
    #[error(
        "mesh outgrew its source: {mesh_bytes} bytes of geometry from {input_bytes} bytes of file"
    )]
    MeshOutgrewItsSource {
        /// Bytes of vertices and indices built so far.
        mesh_bytes: u64,
        /// Bytes of file the reader was given.
        input_bytes: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_human_readable() {
        let e = CoreError::IndexOutOfRange {
            at_index: 4,
            value: 99,
            vertex_count: 10,
        };
        let s = format!("{e}");
        assert!(s.contains("99"));
        assert!(s.contains("10"));
    }

    #[test]
    fn index_count_error_carries_value() {
        let e = CoreError::IndexCountNotMultipleOfThree { index_count: 7 };
        assert!(format!("{e}").contains('7'));
    }
}

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn variant_name(error: &CoreError) -> &'static str {
        match error {
            CoreError::IndexOutOfRange { .. } => "IndexOutOfRange",
            CoreError::IndexCountNotMultipleOfThree { .. } => "IndexCountNotMultipleOfThree",
            CoreError::Geometry(_) => "Geometry",
            CoreError::MeshOutgrewItsSource { .. } => "MeshOutgrewItsSource",
        }
    }

    #[test]
    fn every_core_error_variant_renders_an_operator_message() {
        let variants = [
            CoreError::IndexOutOfRange {
                at_index: 4,
                value: 99,
                vertex_count: 10,
            },
            CoreError::IndexCountNotMultipleOfThree { index_count: 7 },
            CoreError::Geometry("a triangle is degenerate".to_string()),
            CoreError::MeshOutgrewItsSource {
                mesh_bytes: 32,
                input_bytes: 8,
            },
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

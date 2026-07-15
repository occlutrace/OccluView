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

    /// A numeric conversion failed (e.g. a size overflowed `u32`).
    #[error("numeric conversion failed: {0}")]
    NumericOverflow(#[from] std::num::TryFromIntError),

    /// A geometry invariant was violated (degenerate triangle, NaN, etc.).
    #[error("geometry invariant violated: {0}")]
    Geometry(String),
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

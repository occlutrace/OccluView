use crate::error::FormatError;

pub(super) fn malformed(reason: &str) -> FormatError {
    FormatError::Malformed {
        format: "glTF",
        offset: 0,
        reason: reason.to_string(),
    }
}

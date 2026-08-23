//! Product-neutral parsing for HPS dental surfaces.

#![forbid(unsafe_code)]

mod base64;
mod crypto;
mod error;
mod faces;
mod key;
mod parser;
mod surface;
#[cfg(test)]
mod tests;
mod texture;
#[cfg(test)]
mod texture_tests;
mod xml;

pub use error::{HpsError, ReadError};
pub use key::{
    EnvHpsKeyProvider, HpsKeyProvider, HpsSecretKey, NoHpsKeyProvider, RuntimeHpsKeyProvider,
};
pub use parser::{read, read_with_key_provider};
pub use surface::{DecodedSurface, DecodedSurfaceParts, DecodedTexture};

/// Semantic version of the HPS parser implementation.
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn malformed(reason: impl Into<String>) -> HpsError {
    HpsError::BadContainer {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod public_contract_tests {
    use super::PARSER_VERSION;

    #[test]
    fn parser_version_is_a_usable_semver_triple() {
        // Comparing PARSER_VERSION with env!("CARGO_PKG_VERSION") only restated
        // its own definition. What a consumer needs is that the constant is
        // shaped like a version they can compare against, which a workspace
        // version bump could break without anyone reading this crate.
        let parts: Vec<&str> = PARSER_VERSION.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "PARSER_VERSION should be major.minor.patch, got {PARSER_VERSION}"
        );
        for part in parts {
            assert!(
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()),
                "PARSER_VERSION component {part:?} is not numeric in {PARSER_VERSION}"
            );
        }
    }
}

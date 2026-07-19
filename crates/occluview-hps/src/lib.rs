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
    fn parser_version_matches_package_metadata() {
        assert_eq!(PARSER_VERSION, env!("CARGO_PKG_VERSION"));
    }
}

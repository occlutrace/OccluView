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
pub use texture::{MAX_TEXTURE_DIMENSION_PX, MAX_TEXTURE_RGBA_BYTES};

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
    fn the_embedded_key_layout_is_derived_only_from_committed_inputs() {
        // The build script used to mix `SystemTime::now()` and
        // `std::process::id()` into the seed for the obfuscated key layout, so
        // the same source, key and toolchain produced a different binary every
        // time. Everything else in the supply chain answers "this came from our
        // pipeline"; that made "this matches this source" unanswerable, even
        // for a rebuild of an old tag.
        let build_script = include_str!("../build.rs");
        for nondeterminism in ["SystemTime::now", "process::id", "UNIX_EPOCH"] {
            assert!(
                !build_script.contains(nondeterminism),
                "the generated key layout must not depend on {nondeterminism}"
            );
        }
        assert!(
            build_script.contains("OCCLUVIEW_HPS_KEY_SALT"),
            "diversifying the layout should be an explicit release input"
        );
        assert!(
            build_script.contains("rerun-if-env-changed=OCCLUVIEW_HPS_KEY_SALT"),
            "cargo must rebuild when the salt changes"
        );
    }

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

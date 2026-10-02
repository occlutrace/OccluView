//! Typed parser failures.

use thiserror::Error;

/// Failures produced while decoding a dental HPS surface.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HpsError {
    /// The input is a medical DICOM file, not a supported HPS container.
    #[error("medical DICOM is not a supported HPS container")]
    MedicalDicom,

    /// The bytes do not have a recognized raw HPS signature.
    #[error("not a recognized HPS container")]
    BadSignature,

    /// The raw HPS payload uses an unsupported text encoding.
    #[error("unsupported HPS XML encoding: {reason}")]
    UnsupportedEncoding {
        /// Human-readable encoding failure.
        reason: String,
    },

    /// The container or decoded geometry is structurally invalid.
    #[error("invalid HPS container: {reason}")]
    BadContainer {
        /// Human-readable structural failure without secret material.
        reason: String,
    },

    /// Encrypted `CE` data was encountered without a usable key.
    #[error("encrypted CE schema needs a configured key provider")]
    KeyMissing,

    /// Configured key bytes cannot be used by the HPS cipher.
    #[error("invalid CE encryption key: {reason}")]
    InvalidKey {
        /// Human-readable validation failure without key material.
        reason: String,
    },

    /// Decrypted data did not match its integrity marker.
    #[error("HPS integrity check failed: {reason}")]
    IntegrityFailure {
        /// Human-readable integrity failure without key material.
        reason: String,
    },

    /// The package requires lock metadata or key material unavailable to the caller.
    #[error("HPS package is locked: {reason}")]
    PackageLocked {
        /// Human-readable package-lock failure without key material.
        reason: String,
    },

    /// Texture metadata or decoded pixels are inconsistent.
    #[error("malformed HPS texture: {reason}")]
    TextureMalformed {
        /// Human-readable texture failure.
        reason: String,
    },

    /// A bounded parser resource would exceed its configured limit.
    #[error("HPS {resource} exceeds limit {limit}")]
    ResourceLimit {
        /// Resource whose allocation or expansion was rejected.
        resource: &'static str,
        /// Maximum accepted size in the resource's natural unit.
        limit: u64,
    },
}

/// A parser failure or an error returned unchanged by a caller-supplied key provider.
#[derive(Debug, Error)]
pub enum ReadError<E> {
    /// HPS detection, decoding, or validation failed.
    #[error(transparent)]
    Parser(#[from] HpsError),
    /// The caller-supplied key provider failed.
    #[error("HPS key provider failed: {0}")]
    KeyProvider(E),
}

#[cfg(test)]
mod error_contract {
    use super::*;

    /// Names every variant so a new one cannot be added without a case here.
    fn hps_variant_name(error: &HpsError) -> &'static str {
        match error {
            HpsError::MedicalDicom => "MedicalDicom",
            HpsError::BadSignature => "BadSignature",
            HpsError::UnsupportedEncoding { .. } => "UnsupportedEncoding",
            HpsError::BadContainer { .. } => "BadContainer",
            HpsError::KeyMissing => "KeyMissing",
            HpsError::InvalidKey { .. } => "InvalidKey",
            HpsError::IntegrityFailure { .. } => "IntegrityFailure",
            HpsError::PackageLocked { .. } => "PackageLocked",
            HpsError::TextureMalformed { .. } => "TextureMalformed",
            HpsError::ResourceLimit { .. } => "ResourceLimit",
        }
    }

    /// Names every variant so a new one cannot be added without a case here.
    fn read_error_variant_name<E>(error: &ReadError<E>) -> &'static str {
        match error {
            ReadError::Parser(_) => "Parser",
            ReadError::KeyProvider(_) => "KeyProvider",
        }
    }

    #[test]
    fn every_hps_error_variant_renders_an_operator_message() {
        let variants = [
            HpsError::MedicalDicom,
            HpsError::BadSignature,
            HpsError::UnsupportedEncoding {
                reason: "utf-16 is not supported".to_string(),
            },
            HpsError::BadContainer {
                reason: "the payload is truncated".to_string(),
            },
            HpsError::KeyMissing,
            HpsError::InvalidKey {
                reason: "the key is too short".to_string(),
            },
            HpsError::IntegrityFailure {
                reason: "the marker does not match".to_string(),
            },
            HpsError::PackageLocked {
                reason: "the lock metadata is absent".to_string(),
            },
            HpsError::TextureMalformed {
                reason: "the texture stride is wrong".to_string(),
            },
            HpsError::ResourceLimit {
                resource: "texture bytes",
                limit: 1_048_576,
            },
        ];
        for variant in &variants {
            assert!(!hps_variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }

    #[test]
    fn every_read_error_variant_renders_an_operator_message() {
        let variants = [
            ReadError::<String>::Parser(HpsError::BadSignature),
            ReadError::<String>::KeyProvider("the provider returned no key".to_string()),
        ];
        for variant in &variants {
            assert!(!read_error_variant_name(variant).is_empty());
            let message = variant.to_string();
            assert!(!message.trim().is_empty(), "empty message for {variant:?}");
            assert!(
                !message.contains("occlu-") && !message.contains("occlu_"),
                "operator message names an internal crate: {message:?}"
            );
        }
    }
}

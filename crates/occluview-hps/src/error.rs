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

use super::error::HpsError;
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Legacy key container retained for source compatibility; readers do not use it.
///
/// The bytes are redacted in `Debug` and zeroized on drop. This does not make
/// an offline client secret impossible to extract; it just keeps accidental
/// leaks out of logs and crash text.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct HpsSecretKey {
    bytes: Vec<u8>,
}

impl HpsSecretKey {
    /// Construct a Blowfish-compatible HPS key.
    ///
    /// # Errors
    /// Returns [`HpsError::InvalidKey`] when the key length is outside
    /// Blowfish's 4..=56 byte range.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, HpsError> {
        let mut bytes = bytes;
        if let Err(error) = validate_key_len(bytes.len()) {
            bytes.zeroize();
            return Err(error);
        }
        Ok(Self { bytes })
    }

    /// Parse the same human-friendly key syntax used by the legacy native
    /// parser: decimal byte CSV (`1,2,3`) or raw UTF-8 bytes.
    ///
    /// # Errors
    /// Returns [`HpsError::InvalidKey`] if the resulting key is not
    /// Blowfish-compatible.
    pub fn from_config_value(value: &str) -> Result<Self, HpsError> {
        if value.contains(',') {
            let parsed: Option<Vec<u8>> = value
                .split(',')
                .map(str::trim)
                .map(|token| token.parse::<u8>().ok())
                .collect();
            if let Some(bytes) = parsed.filter(|bytes| !bytes.is_empty()) {
                return Self::from_bytes(bytes);
            }
        }
        Self::from_bytes(value.as_bytes().to_vec())
    }

    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for HpsSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HpsSecretKey")
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Legacy provider interface. Readers never consult it.
pub trait HpsKeyProvider: Sync {
    /// Provider-specific error returned unchanged by [`super::ReadError`].
    type Error;

    /// Return the base secret key, or `None` when this build/user has no key.
    ///
    /// # Errors
    /// Providers should return an error when the configured key exists but is
    /// malformed or unavailable due to an OS credential-store failure.
    fn base_key(&self) -> Result<Option<HpsSecretKey>, Self::Error>;
}

/// Default public-build provider: no secret material is shipped.
#[derive(Debug, Default, Copy, Clone)]
pub struct NoHpsKeyProvider;

impl HpsKeyProvider for NoHpsKeyProvider {
    type Error = std::convert::Infallible;

    fn base_key(&self) -> Result<Option<HpsSecretKey>, Self::Error> {
        Ok(None)
    }
}

/// Legacy provider retained for source compatibility. No keys are read.
#[derive(Debug, Default, Copy, Clone)]
pub struct EnvHpsKeyProvider;

impl HpsKeyProvider for EnvHpsKeyProvider {
    type Error = HpsError;

    fn base_key(&self) -> Result<Option<HpsSecretKey>, Self::Error> {
        Ok(None)
    }
}

/// Runtime provider. Encrypted HPS input is unsupported in every build.
#[derive(Debug, Default, Copy, Clone)]
pub struct RuntimeHpsKeyProvider;

impl HpsKeyProvider for RuntimeHpsKeyProvider {
    type Error = HpsError;

    fn base_key(&self) -> Result<Option<HpsSecretKey>, Self::Error> {
        Ok(None)
    }
}

pub(super) fn validate_key_len(len: usize) -> Result<(), HpsError> {
    if !(4..=56).contains(&len) {
        return Err(HpsError::InvalidKey {
            reason: "CE encryption key must be 4..56 bytes for Blowfish".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn config_value_accepts_decimal_csv_bytes() {
        let key = HpsSecretKey::from_config_value("1, 2, 3, 4").expect("valid CSV key");
        assert_eq!(key.as_bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn config_value_accepts_raw_utf8_bytes() {
        let key = HpsSecretKey::from_config_value("abcd").expect("valid raw key");
        assert_eq!(key.as_bytes(), b"abcd");
    }
}

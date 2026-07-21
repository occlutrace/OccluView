//! OccluView compatibility adapter for the product-neutral HPS parser.

mod mesh;

use crate::error::FormatError;
use occluview_core::Mesh;
use std::fmt;

const FORMAT: &str = "HPS";

/// Parser version exposed through the formats facade.
pub const PARSER_VERSION: &str = occluview_hps::PARSER_VERSION;

/// Stable HPS failure categories for consumers that must not depend on the
/// product-neutral parser crate directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HpsReadFailure {
    /// The input is a medical DICOM file.
    MedicalDicom,
    /// The bytes do not contain a recognized HPS payload.
    BadSignature,
    /// The HPS container is structurally invalid.
    MalformedInput,
    /// The payload uses an unsupported encoding.
    UnsupportedEncoding,
    /// The encrypted payload has no configured key.
    KeyMissing,
    /// The configured key is invalid.
    InvalidKey,
    /// The decoded data failed integrity validation.
    IntegrityFailure,
    /// The package is locked or otherwise unavailable.
    PackageLocked,
    /// Texture metadata or pixels are malformed.
    TextureMalformed,
    /// A bounded parser resource exceeded its limit.
    ResourceLimit,
    /// A key provider failed outside normal parser validation.
    KeyProviderFailed,
}

/// Detailed HPS read failures for headless callers that need stable error
/// classification without depending on the leaf parser directly.
#[derive(Debug)]
pub enum HpsReadError {
    /// The input failed HPS parsing or validation.
    Parser(HpsReadFailure),
    /// The runtime key provider failed while decoding encrypted input.
    KeyProvider(HpsReadFailure),
    /// The decoded surface could not be represented by the core mesh model.
    Surface(FormatError),
}

/// HPS failures that can occur before conversion into the viewer mesh model.
#[derive(Debug)]
pub enum HpsDecodedReadError {
    /// The input failed HPS parsing or validation.
    Parser(HpsReadFailure),
    /// The runtime key provider failed while decoding encrypted input.
    KeyProvider(HpsReadFailure),
}

impl From<HpsDecodedReadError> for HpsReadError {
    fn from(error: HpsDecodedReadError) -> Self {
        match error {
            HpsDecodedReadError::Parser(error) => Self::Parser(error),
            HpsDecodedReadError::KeyProvider(error) => Self::KeyProvider(error),
        }
    }
}

/// Secret bytes used to decrypt encrypted HPS `CE` blocks.
///
/// This compatibility wrapper preserves the original formats API while the
/// neutral parser owns key storage, redaction, and zeroization.
#[derive(Clone)]
pub struct HpsSecretKey(occluview_hps::HpsSecretKey);

impl HpsSecretKey {
    /// Construct a Blowfish-compatible HPS key.
    ///
    /// # Errors
    /// Returns [`FormatError::Malformed`] when the key length is outside
    /// Blowfish's 4..=56 byte range.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, FormatError> {
        occluview_hps::HpsSecretKey::from_bytes(bytes)
            .map(Self)
            .map_err(map_parser_error)
    }

    /// Parse decimal byte CSV (`1,2,3`) or raw UTF-8 key bytes.
    ///
    /// # Errors
    /// Returns [`FormatError::Malformed`] if the resulting key is not
    /// Blowfish-compatible.
    pub fn from_config_value(value: &str) -> Result<Self, FormatError> {
        occluview_hps::HpsSecretKey::from_config_value(value)
            .map(Self)
            .map_err(map_parser_error)
    }
}

impl fmt::Debug for HpsSecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HpsSecretKey")
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Supplies the base HPS key for encrypted `CE` files.
pub trait HpsKeyProvider: Sync {
    /// Return the base secret key, or `None` when this build/user has no key.
    ///
    /// # Errors
    /// Providers should return an error when configured key material is invalid
    /// or unavailable.
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError>;
}

/// Default public-build provider: no secret material is shipped.
#[derive(Debug, Default, Copy, Clone)]
pub struct NoHpsKeyProvider;

impl HpsKeyProvider for NoHpsKeyProvider {
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError> {
        Ok(None)
    }
}

/// Reads an HPS key from the supported process environment variables.
#[derive(Debug, Default, Copy, Clone)]
pub struct EnvHpsKeyProvider;

impl HpsKeyProvider for EnvHpsKeyProvider {
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError> {
        leaf_provider_key(&occluview_hps::EnvHpsKeyProvider::default())
    }
}

/// Runtime provider used by the app, CLI, and shell paths.
#[derive(Debug, Default, Copy, Clone)]
pub struct RuntimeHpsKeyProvider;

impl HpsKeyProvider for RuntimeHpsKeyProvider {
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError> {
        leaf_provider_key(&occluview_hps::RuntimeHpsKeyProvider)
    }
}

fn leaf_provider_key<P>(provider: &P) -> Result<Option<HpsSecretKey>, FormatError>
where
    P: occluview_hps::HpsKeyProvider<Error = occluview_hps::HpsError>,
{
    provider
        .base_key()
        .map(|key| key.map(HpsSecretKey))
        .map_err(map_parser_error)
}

struct ProviderAdapter<'a>(&'a dyn HpsKeyProvider);

impl occluview_hps::HpsKeyProvider for ProviderAdapter<'_> {
    type Error = FormatError;

    fn base_key(&self) -> Result<Option<occluview_hps::HpsSecretKey>, Self::Error> {
        self.0.base_key().map(|key| key.map(|key| key.0))
    }
}

/// Read raw HPS XML or a dental HPS package into an OccluView mesh.
///
/// # Errors
/// Returns [`FormatError::Deferred`] for encrypted `CE` without a key provider,
/// [`FormatError::Unsupported`] for medical DICOM, and typed malformed format
/// errors for invalid dental HPS data.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    read_decoded_surface(bytes).and_then(mesh::build_mesh)
}

/// Read raw HPS XML or a dental HPS package while preserving source topology.
///
/// This is the export-facing form of the HPS reader. Its indices reference the
/// original positions, while textured UVs remain indexed by triangle corner.
/// Use [`read`] when a render-ready [`Mesh`] is required.
///
/// # Errors
/// Returns [`FormatError`] when the input is not a supported HPS payload or
/// fails parser validation.
pub fn read_decoded_surface(bytes: &[u8]) -> Result<occluview_hps::DecodedSurface, FormatError> {
    occluview_hps::read(bytes).map_err(map_parser_error)
}

/// Read raw HPS XML or a dental HPS package with an explicit key provider.
///
/// # Errors
/// Parser failures map to the existing [`FormatError`] contract. Errors returned
/// by `key_provider` are propagated unchanged.
pub fn read_with_key_provider(
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    read_decoded_surface_with_key_provider(bytes, key_provider).and_then(mesh::build_mesh)
}

/// Read HPS bytes with an explicit key provider while preserving source
/// topology and corner-indexed texture coordinates.
///
/// # Errors
/// Returns [`FormatError`] when parsing fails or the key provider rejects the
/// encrypted payload.
pub fn read_decoded_surface_with_key_provider(
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<occluview_hps::DecodedSurface, FormatError> {
    let provider = ProviderAdapter(key_provider);
    match occluview_hps::read_with_key_provider(bytes, &provider) {
        Ok(surface) => Ok(surface),
        Err(occluview_hps::ReadError::Parser(error)) => Err(map_parser_error(error)),
        Err(occluview_hps::ReadError::KeyProvider(error)) => Err(error),
    }
}

/// Read HPS bytes with the runtime environment/embedded key provider.
///
/// This is the headless conversion facade: callers receive the parser version
/// and structured parser/key-provider classification without importing the
/// product-neutral leaf crate.
///
/// # Errors
/// Returns a stable HPS failure category when parsing, key resolution, or
/// conversion into the core mesh model fails.
pub fn read_bytes_with_runtime_key_provider(bytes: &[u8]) -> Result<Mesh, HpsReadError> {
    let surface = read_decoded_surface_bytes_with_runtime_key_provider(bytes)?;
    mesh::build_mesh(surface).map_err(HpsReadError::Surface)
}

/// Read HPS bytes with the runtime key provider while preserving the source
/// CAD topology for a lossless geometry export.
///
/// # Errors
/// Returns [`HpsDecodedReadError`] when parsing or runtime key resolution fails.
pub fn read_decoded_surface_bytes_with_runtime_key_provider(
    bytes: &[u8],
) -> Result<occluview_hps::DecodedSurface, HpsDecodedReadError> {
    match occluview_hps::read_with_key_provider(bytes, &occluview_hps::RuntimeHpsKeyProvider) {
        Ok(surface) => Ok(surface),
        Err(occluview_hps::ReadError::Parser(error)) => {
            Err(HpsDecodedReadError::Parser(classify_parser_error(&error)))
        }
        Err(occluview_hps::ReadError::KeyProvider(error)) => Err(HpsDecodedReadError::KeyProvider(
            classify_key_provider_error(&error),
        )),
    }
}

/// Convert a validated product-neutral HPS surface into an OccluView mesh.
///
/// This is the only public bridge from [`occluview_hps::DecodedSurface`]
/// into the viewer mesh model. Parsing and key handling remain in
/// `occluview-hps`.
///
/// # Errors
/// Returns [`FormatError`] when the validated surface cannot be represented by
/// [`Mesh`].
pub fn mesh_from_decoded_surface(
    surface: occluview_hps::DecodedSurface,
) -> Result<Mesh, FormatError> {
    mesh::build_mesh(surface)
}

/// Convert a decoded HPS surface into a source-topology mesh for geometry
/// writers and mesh-processing operations.
///
/// # Errors
/// Returns [`FormatError`] when the validated surface cannot be represented by
/// the core mesh model.
pub fn geometry_mesh_from_decoded_surface(
    surface: &occluview_hps::DecodedSurface,
) -> Result<Mesh, FormatError> {
    mesh::build_geometry_mesh(surface)
}

fn map_parser_error(error: occluview_hps::HpsError) -> FormatError {
    use occluview_hps::HpsError;

    match error {
        HpsError::MedicalDicom => FormatError::Unsupported {
            extension: "dicom".to_string(),
        },
        HpsError::BadSignature => FormatError::BadSignature {
            format: FORMAT,
            offset: 0,
        },
        HpsError::UnsupportedEncoding { reason } | HpsError::PackageLocked { reason } => {
            FormatError::Deferred {
                format: "HPS",
                reason,
            }
        }
        HpsError::KeyMissing => FormatError::Deferred {
            format: "HPS",
            reason: "encrypted CE schema needs a configured key provider".to_string(),
        },
        HpsError::BadContainer { reason }
        | HpsError::InvalidKey { reason }
        | HpsError::IntegrityFailure { reason } => malformed(FORMAT, reason),
        HpsError::TextureMalformed { reason } => malformed("HPS", reason),
        HpsError::ResourceLimit { resource, limit } => {
            let format = if resource.starts_with("texture") {
                "HPS"
            } else {
                FORMAT
            };
            malformed(format, format!("{resource} exceeds limit {limit}"))
        }
    }
}

fn classify_parser_error(error: &occluview_hps::HpsError) -> HpsReadFailure {
    use occluview_hps::HpsError;

    match error {
        HpsError::MedicalDicom => HpsReadFailure::MedicalDicom,
        HpsError::BadSignature => HpsReadFailure::BadSignature,
        HpsError::BadContainer { .. } => HpsReadFailure::MalformedInput,
        HpsError::UnsupportedEncoding { .. } => HpsReadFailure::UnsupportedEncoding,
        HpsError::KeyMissing => HpsReadFailure::KeyMissing,
        HpsError::InvalidKey { .. } => HpsReadFailure::InvalidKey,
        HpsError::IntegrityFailure { .. } => HpsReadFailure::IntegrityFailure,
        HpsError::PackageLocked { .. } => HpsReadFailure::PackageLocked,
        HpsError::TextureMalformed { .. } => HpsReadFailure::TextureMalformed,
        HpsError::ResourceLimit { .. } => HpsReadFailure::ResourceLimit,
    }
}

fn classify_key_provider_error(error: &occluview_hps::HpsError) -> HpsReadFailure {
    match error {
        occluview_hps::HpsError::InvalidKey { .. }
        | occluview_hps::HpsError::BadContainer { .. } => HpsReadFailure::InvalidKey,
        _ => HpsReadFailure::KeyProviderFailed,
    }
}

fn malformed(format: &'static str, reason: impl Into<String>) -> FormatError {
    FormatError::Malformed {
        format,
        offset: 0,
        reason: reason.into(),
    }
}

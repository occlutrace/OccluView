//! External-style compile and runtime checks for the public HPS compatibility API.

#![allow(clippy::expect_used)]

use occluview_core::Mesh;
use occluview_formats::{
    hps::{
        mesh_from_decoded_surface, read, read_with_key_provider, HpsKeyProvider, HpsSecretKey,
        NoHpsKeyProvider,
    },
    FormatError,
};
use occluview_formats::hps::parser::DecodedSurface;

struct ExternalProvider;

impl HpsKeyProvider for ExternalProvider {
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError> {
        Ok(None)
    }
}

struct FailingProvider;

impl HpsKeyProvider for FailingProvider {
    fn base_key(&self) -> Result<Option<HpsSecretKey>, FormatError> {
        Err(FormatError::UnsafePath {
            format: "provider-test",
            path: "sentinel".to_string(),
        })
    }
}

#[test]
fn hps_public_signatures_accept_external_providers() {
    let read_signature: fn(&[u8]) -> Result<Mesh, FormatError> = read;
    let provider_signature: fn(&[u8], &dyn HpsKeyProvider) -> Result<Mesh, FormatError> =
        read_with_key_provider;
    let key_constructor: fn(Vec<u8>) -> Result<HpsSecretKey, FormatError> =
        HpsSecretKey::from_bytes;

    let xml = br#"<HPS><Schema>CC</Schema><Facets facet_count="1">BA==</Facets><Vertices vertex_count="3">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices></HPS>"#;
    let mesh = provider_signature(xml, &ExternalProvider).expect("custom provider should compile");
    assert_eq!(mesh.indices(), &[0, 1, 2]);

    let mesh = read_signature(xml).expect("read signature should remain compatible");
    assert_eq!(mesh.indices(), &[0, 1, 2]);
    assert!(key_constructor(vec![1, 2, 3, 4]).is_ok());
    assert!(NoHpsKeyProvider.base_key().is_ok());
}

#[test]
fn neutral_surface_adapter_is_the_minimal_public_mesh_bridge() {
    let adapter: fn(DecodedSurface) -> Result<Mesh, FormatError> = mesh_from_decoded_surface;
    let surface = DecodedSurface::new(
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        vec![0, 1, 2],
        None,
        None,
        None,
    )
    .expect("valid neutral surface");

    let mesh = adapter(surface).expect("neutral surface should adapt");

    assert_eq!(mesh.indices(), &[0, 1, 2]);
    assert!(mesh.vertices()[1]
        .position
        .into_iter()
        .zip([1.0, 0.0, 0.0])
        .all(|(actual, expected)| (actual - expected).abs() < f32::EPSILON));
}

#[test]
fn hps_custom_provider_errors_propagate_unchanged() {
    let error = read_with_key_provider(b"<HPS><Schema>CE</Schema></HPS>", &FailingProvider)
        .expect_err("custom provider error should propagate");
    assert!(matches!(
        error,
        FormatError::UnsafePath { format, path }
            if format == "provider-test" && path == "sentinel"
    ));
}

#[test]
fn hps_secret_debug_output_is_redacted() {
    let key = HpsSecretKey::from_bytes(vec![1, 2, 3, 4]).expect("valid synthetic key");
    assert_eq!(format!("{key:?}"), "HpsSecretKey { bytes: \"<redacted>\" }");
}

#[test]
fn hps_non_utf8_xml_keeps_deferred_classification_with_current_copy() {
    let error = read(&[0xff]).expect_err("non-UTF-8 raw HPS should be deferred");
    assert!(matches!(
        error,
        FormatError::Deferred { format, reason }
            if format == "HPS"
                && reason.contains("raw HPS XML")
                && !reason.contains("package extraction")
    ));
}

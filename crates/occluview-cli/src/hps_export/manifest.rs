use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub(crate) struct Manifest {
    schema_version: u32,
    parser_version: &'static str,
    geometry: ManifestArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<ManifestArtifact>,
}

#[derive(Debug, Serialize)]
struct ManifestArtifact {
    format: &'static str,
    path: &'static str,
    sha256: String,
}

impl Manifest {
    pub(crate) fn new(geometry: &[u8], preview: Option<&[u8]>) -> Self {
        Self {
            schema_version: 2,
            parser_version: occluview_formats::hps::PARSER_VERSION,
            geometry: ManifestArtifact::new("ply", "surface.ply", geometry),
            preview: preview.map(|bytes| ManifestArtifact::new("glb", "surface.glb", bytes)),
        }
    }
}

impl ManifestArtifact {
    fn new(format: &'static str, path: &'static str, bytes: &[u8]) -> Self {
        Self {
            format,
            path,
            sha256: digest_hex(&Sha256::digest(bytes)),
        }
    }
}

fn digest_hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[usize::from(*byte >> 4)] as char);
        encoded.push(HEX[usize::from(*byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::Manifest;

    #[test]
    fn manifest_v2_contains_geometry_and_optional_preview_hashes() {
        let manifest = Manifest::new(b"artifact", Some(b"preview"));
        let json = serde_json::to_value(manifest).expect("serialize manifest");

        assert_eq!(json["schema_version"], 2);
        assert_eq!(json["parser_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["geometry"]["format"], "ply");
        assert_eq!(json["geometry"]["path"], "surface.ply");
        assert_eq!(
            json["geometry"]["sha256"],
            "c7c5c1d70c5dec4416ab6158afd0b223ef40c29b1dc1f97ed9428b94d4cadb1c"
        );
        assert_eq!(json["preview"]["format"], "glb");
        assert_eq!(json["preview"]["path"], "surface.glb");
        assert_eq!(
            json["preview"]["sha256"],
            "5975cf1bba432391c94667f5886225f69377c0aa8b9fa21fddfb21c89bcf9092"
        );
    }

    #[test]
    fn manifest_v2_omits_preview_when_surface_is_untextured() {
        let manifest = Manifest::new(b"artifact", None);
        let json = serde_json::to_value(manifest).expect("serialize manifest");

        assert_eq!(json["schema_version"], 2);
        assert!(json.get("geometry").is_some());
        assert!(json.get("preview").is_none());
    }
}

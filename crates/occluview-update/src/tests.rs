//! Verification-path tests: signature roundtrip (sign with the full minisign
//! crate, verify with the shipping verify-only path), version gating, and
//! manifest-shape errors.

use super::*;

fn test_keypair() -> (minisign::KeyPair, String) {
    let keypair = minisign::KeyPair::generate_unencrypted_keypair().expect("generate test keypair");
    let pubkey = keypair.pk.to_base64();
    (keypair, pubkey)
}

fn sign(keypair: &minisign::KeyPair, message: &[u8]) -> String {
    let signature = minisign::sign(None, &keypair.sk, std::io::Cursor::new(message), None, None)
        .expect("sign test payload");
    signature.to_string()
}

#[test]
fn signature_roundtrip_accepts_signed_and_rejects_tampered() {
    let (keypair, pubkey) = test_keypair();
    let message = b"manifest payload";
    let signature = sign(&keypair, message);

    assert!(verify_signature(&pubkey, message, signature.as_bytes()).is_ok());
    assert!(matches!(
        verify_signature(&pubkey, b"tampered payload", signature.as_bytes()),
        Err(UpdateError::BadSignature)
    ));
    let (_, other_pubkey) = test_keypair();
    assert!(matches!(
        verify_signature(&other_pubkey, message, signature.as_bytes()),
        Err(UpdateError::BadSignature)
    ));
}

#[test]
fn version_parsing_accepts_v_prefix_and_rejects_garbage() {
    assert_eq!(
        parse_version("v1.2.3").expect("v-prefixed"),
        semver::Version::new(1, 2, 3)
    );
    assert_eq!(
        parse_version("0.1.14").expect("plain"),
        semver::Version::new(0, 1, 14)
    );
    assert!(matches!(
        parse_version("latest"),
        Err(UpdateError::BadManifest(_))
    ));
}

#[test]
fn manifest_parses_platform_entries() {
    let manifest: Manifest = serde_json::from_str(
        r#"{
            "version": "0.2.0",
            "notes": "fixes",
            "platforms": {
                "windows-x86_64": {
                    "url": "https://example.invalid/OccluView-0.2.0-x86_64.msi",
                    "signature": "sig",
                    "sha256": "AB12"
                }
            }
        }"#,
    )
    .expect("manifest parses");
    assert_eq!(manifest.version, "0.2.0");
    assert_eq!(manifest.notes.as_deref(), Some("fixes"));
    assert!(manifest.platforms.contains_key("windows-x86_64"));
}

#[test]
fn check_announces_release_without_platform_artifact() {
    let (keypair, pubkey) = test_keypair();
    let manifest = br#"{"version": "9.9.9", "platforms": {}}"#.to_vec();
    let signature = sign(&keypair, &manifest).into_bytes();
    let manifest_url = serve_once(manifest, "/latest.json");
    let sig_url = serve_once(signature, "/latest.json.minisig");

    let update = check_with(&manifest_url, &sig_url, &pubkey, "0.1.0")
        .expect("check succeeds")
        .expect("newer version must be announced even without a platform asset");
    assert!(!update.downloadable());
    assert!(update.url().is_none());
    assert!(matches!(
        download_with(&update, &pubkey, &std::env::temp_dir(), &mut |_, _| {}),
        Err(UpdateError::NoPlatformAsset)
    ));
}

/// Serve `body` once over a throwaway local HTTP listener; returns the URL.
fn serve_once(body: Vec<u8>, path: &str) -> String {
    use std::io::{Read as _, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut discard = [0u8; 4096];
            let _ = stream.read(&mut discard);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{address}{path}")
}

fn sha256_hex(payload: &[u8]) -> String {
    Sha256::digest(payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn download_verifies_hash_and_signature_end_to_end() {
    let (keypair, pubkey) = test_keypair();
    let payload = b"fake installer bytes".to_vec();
    let update = AvailableUpdate {
        version: semver::Version::new(9, 9, 9),
        notes: None,
        artifact: Some(PlatformArtifact {
            url: serve_once(payload.clone(), "/OccluView-9.9.9.msi"),
            signature: sign(&keypair, &payload),
            sha256: sha256_hex(&payload),
        }),
    };
    let dir = std::env::temp_dir().join(format!("occluview-update-test-{}", std::process::id()));

    let mut last_progress = 0;
    let installer = download_with(&update, &pubkey, &dir, &mut |done, _| last_progress = done)
        .expect("verified download succeeds");
    assert!(installer.ends_with("OccluView-9.9.9.msi"));
    assert_eq!(last_progress, payload.len() as u64);
    assert_eq!(std::fs::read(&installer).expect("read artifact"), payload);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn download_rejects_hash_mismatch_and_removes_partial() {
    let (keypair, pubkey) = test_keypair();
    let payload = b"fake installer bytes".to_vec();
    let update = AvailableUpdate {
        version: semver::Version::new(9, 9, 9),
        notes: None,
        artifact: Some(PlatformArtifact {
            url: serve_once(payload.clone(), "/OccluView-9.9.9.msi"),
            signature: sign(&keypair, &payload),
            sha256: "0".repeat(64),
        }),
    };
    let dir = std::env::temp_dir().join(format!("occluview-update-badhash-{}", std::process::id()));

    let result = download_with(&update, &pubkey, &dir, &mut |_, _| {});
    assert!(matches!(result, Err(UpdateError::BadHash)));
    assert!(!dir.join("OccluView-9.9.9.msi.partial").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn download_rejects_bad_signature_and_removes_partial() {
    // Hash matches (so the stream is written in full) but the artifact is signed
    // by the wrong key: the signature step fails and the centralized cleanup
    // must still delete the fully written `.partial`.
    let (_keypair, pubkey) = test_keypair();
    let (wrong_keypair, _wrong_pubkey) = test_keypair();
    let payload = b"fake installer bytes".to_vec();
    let update = AvailableUpdate {
        version: semver::Version::new(9, 9, 9),
        notes: None,
        artifact: Some(PlatformArtifact {
            url: serve_once(payload.clone(), "/OccluView-9.9.9.msi"),
            signature: sign(&wrong_keypair, &payload),
            sha256: sha256_hex(&payload),
        }),
    };
    let dir = std::env::temp_dir().join(format!("occluview-update-badsig-{}", std::process::id()));

    let result = download_with(&update, &pubkey, &dir, &mut |_, _| {});
    assert!(matches!(result, Err(UpdateError::BadSignature)));
    assert!(!dir.join("OccluView-9.9.9.msi.partial").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

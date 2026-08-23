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

    assert!(verify_signature(&[pubkey.as_str()], message, signature.as_bytes()).is_ok());
    assert!(matches!(
        verify_signature(
            &[pubkey.as_str()],
            b"tampered payload",
            signature.as_bytes()
        ),
        Err(UpdateError::BadSignature)
    ));
    let (_, other_pubkey) = test_keypair();
    assert!(matches!(
        verify_signature(&[other_pubkey.as_str()], message, signature.as_bytes()),
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

    let update = check_with(&manifest_url, &sig_url, &[pubkey.as_str()], "0.1.0")
        .expect("check succeeds")
        .expect("newer version must be announced even without a platform asset");
    assert!(!update.downloadable());
    assert!(update.url().is_none());
    assert!(matches!(
        download_with(
            &update,
            &[pubkey.as_str()],
            &std::env::temp_dir(),
            &mut |_, _| {}
        ),
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
    let installer = download_with(&update, &[pubkey.as_str()], &dir, &mut |done, _| {
        last_progress = done;
    })
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

    let result = download_with(&update, &[pubkey.as_str()], &dir, &mut |_, _| {});
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

    let result = download_with(&update, &[pubkey.as_str()], &dir, &mut |_, _| {});
    assert!(matches!(result, Err(UpdateError::BadSignature)));
    assert!(!dir.join("OccluView-9.9.9.msi.partial").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_refuses_to_offer_the_running_version_or_an_older_one() {
    // This comparison is the entire rollback protection. Inverting it would make
    // the app offer, download, verify and hand msiexec an OLDER signed
    // installer -- and every signature and hash along the way would agree,
    // because that build really was signed by this key. Nothing else in the
    // module can catch that, so it is checked directly.
    let (keypair, pubkey) = test_keypair();
    for (manifest_version, current) in [("1.2.3", "1.2.3"), ("1.2.3", "1.2.4"), ("1.2.3", "2.0.0")]
    {
        let manifest =
            format!(r#"{{"version": "{manifest_version}", "platforms": {{}}}}"#).into_bytes();
        let signature = sign(&keypair, &manifest).into_bytes();
        let manifest_url = serve_once(manifest, "/latest.json");
        let sig_url = serve_once(signature, "/latest.json.minisig");

        let outcome = check_with(&manifest_url, &sig_url, &[pubkey.as_str()], current)
            .expect("check succeeds");
        assert!(
            outcome.is_none(),
            "manifest {manifest_version} must not be offered to a {current} install"
        );
    }
}

#[test]
fn check_still_offers_a_genuinely_newer_version() {
    // The other half of the gate: proof the comparison is not simply refusing
    // everything, which would make the test above pass for the wrong reason.
    let (keypair, pubkey) = test_keypair();
    let manifest = br#"{"version": "1.2.4", "platforms": {}}"#.to_vec();
    let signature = sign(&keypair, &manifest).into_bytes();
    let manifest_url = serve_once(manifest, "/latest.json");
    let sig_url = serve_once(signature, "/latest.json.minisig");

    let update = check_with(&manifest_url, &sig_url, &[pubkey.as_str()], "1.2.3")
        .expect("check succeeds")
        .expect("1.2.4 is newer than 1.2.3");
    assert_eq!(update.version, semver::Version::new(1, 2, 4));
}

#[test]
fn download_refuses_an_artifact_url_that_is_not_a_plain_file_name() {
    // The manifest chooses this string and it becomes a path under dest_dir.
    let (keypair, pubkey) = test_keypair();
    let payload = b"fake installer bytes".to_vec();
    let dir = std::env::temp_dir().join(format!("occluview-update-badname-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    for path in ["/", "/..", "/.", "/a:b.msi", "/a\\b.msi"] {
        let update = AvailableUpdate {
            version: semver::Version::new(9, 9, 9),
            notes: None,
            artifact: Some(PlatformArtifact {
                url: serve_once(payload.clone(), path),
                signature: sign(&keypair, &payload),
                sha256: sha256_hex(&payload),
            }),
        };
        let result = download_with(&update, &[pubkey.as_str()], &dir, &mut |_, _| {});
        assert!(
            matches!(result, Err(UpdateError::BadManifest(_))),
            "artifact path {path:?} should be rejected, got {result:?}"
        );
    }

    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path().display().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "a rejected artifact name must leave nothing behind: {leftovers:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn any_trusted_key_verifies_and_an_untrusted_one_does_not() {
    // Rotation depends on this: a build that accepts both the current and the
    // next key can be shipped and spread BEFORE signing moves, which turns a
    // key change from an outage into two ordinary releases. Without it, a lost
    // or leaked private key ends updates for every installed copy, because the
    // trust anchor is compiled into binaries already on clinic workstations.
    let (current, current_pubkey) = test_keypair();
    let (next, next_pubkey) = test_keypair();
    let (_stranger_key, stranger_pubkey) = test_keypair();
    let message = b"manifest bytes";
    let signature = sign(&current, message);

    let trusted = [current_pubkey.as_str(), next_pubkey.as_str()];
    assert!(verify_signature(&trusted, message, signature.as_bytes()).is_ok());

    // Signed by the incoming key, verified by the same two-key build.
    let signature_from_next = sign(&next, message);
    assert!(verify_signature(&trusted, message, signature_from_next.as_bytes()).is_ok());

    // A key nobody trusts stays rejected, and so does a tampered payload.
    assert!(matches!(
        verify_signature(&[stranger_pubkey.as_str()], message, signature.as_bytes()),
        Err(UpdateError::BadSignature)
    ));
    assert!(matches!(
        verify_signature(&trusted, b"tampered", signature.as_bytes()),
        Err(UpdateError::BadSignature)
    ));
    assert!(matches!(
        verify_signature(&[], message, signature.as_bytes()),
        Err(UpdateError::BadSignature)
    ));
}

#[test]
fn every_shipped_key_is_usable_and_the_release_key_is_among_them() {
    // The assertion here was `UPDATE_PUBKEYS.contains(&UPDATE_PUBKEY)` against
    // a list defined as `&[UPDATE_PUBKEY]`: it restated its own definition and
    // could not fail.
    //
    // What can fail is a key that is not a key. `verify_signature` skips an
    // entry it cannot parse and moves on, so a truncated constant or a stray
    // character from an edit does not raise anything here -- it makes every
    // installed copy refuse every update, silently, and the release that
    // shipped it looks green.
    for key in UPDATE_PUBKEYS {
        assert!(
            minisign::PublicKey::from_base64(key).is_ok(),
            "a shipped key does not parse, so nothing signed with it can ever \
             be accepted: {key}"
        );
    }
    assert!(
        UPDATE_PUBKEYS.contains(&UPDATE_PUBKEY),
        "releases are signed with UPDATE_PUBKEY; dropping it from the accepted \
         list leaves CI green and every installed copy refusing the update"
    );

    // And the list is consulted rather than assumed: a signature from a key
    // nobody trusts has to be refused by exactly this list.
    let (keypair, _) = test_keypair();
    let message = b"manifest payload";
    let stranger = sign(&keypair, message);
    assert!(
        matches!(
            verify_signature(UPDATE_PUBKEYS, message, stranger.as_bytes()),
            Err(UpdateError::BadSignature)
        ),
        "the shipped list must reject a signature it has no key for"
    );
}

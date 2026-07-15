//! Self-update checks for OccluView.
//!
//! Trust model (the Tauri-updater scheme, minus Tauri): every release attaches
//! a `latest.json` manifest plus detached minisign signatures. The client
//! fetches the manifest from a STABLE release-asset URL (never the GitHub API
//! — asset downloads are not API-rate-limited), verifies the manifest's
//! ed25519 signature against the public key baked into the binary, compares
//! versions, and only ever OFFERS the update. After consent it downloads the
//! installer, verifies its SHA-256 and its own minisign signature, and hands
//! off to `msiexec` on Windows or the desktop package installer for the
//! verified `.deb` on Linux. Nothing is installed silently.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::format_collect
    )
)]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

/// Stable asset URL of the update manifest on the latest GitHub release.
pub const MANIFEST_URL: &str =
    "https://github.com/occlutrace/OccluView/releases/latest/download/latest.json";
/// Detached minisign signature of [`MANIFEST_URL`].
pub const MANIFEST_SIG_URL: &str =
    "https://github.com/occlutrace/OccluView/releases/latest/download/latest.json.minisig";
/// Minisign public key every update is verified against. The matching private
/// key lives ONLY in the maintainer's offline backup + CI secret.
pub const UPDATE_PUBKEY: &str = "RWRoIIL40qxwrFOI5OeCx0Fcf1ClUksy36PrIZrdKkGhQq2kFOtITQnq";

/// Manifest platform key for the running build.
#[cfg(target_os = "windows")]
pub const PLATFORM: &str = "windows-x86_64";
/// Manifest platform key for the running build.
#[cfg(not(target_os = "windows"))]
pub const PLATFORM: &str = "linux-x86_64";

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// Hard ceiling for a downloaded installer (corrupt-manifest guard).
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// Errors surfaced by update checking, downloading, or verification.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Network or HTTP-status failure.
    #[error("update check failed: {0}")]
    Http(String),
    /// The manifest or artifact signature did not verify against the pinned key.
    #[error("update signature verification failed")]
    BadSignature,
    /// The downloaded artifact hash does not match the signed manifest.
    #[error("update artifact hash mismatch")]
    BadHash,
    /// The manifest could not be parsed or carries an invalid version.
    #[error("malformed update manifest: {0}")]
    BadManifest(String),
    /// The manifest has no entry for this platform.
    #[error("no update artifact published for {PLATFORM}")]
    NoPlatformAsset,
    /// Filesystem failure while storing the download.
    #[error("update download I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Installer handoff is only implemented for Windows MSI builds.
    #[error("in-app install is not supported on this platform")]
    Unsupported,
}

#[derive(serde::Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    platforms: BTreeMap<String, PlatformEntry>,
}

#[derive(serde::Deserialize, Clone)]
struct PlatformEntry {
    url: String,
    /// Detached minisign signature of the artifact (full `.minisig` contents).
    signature: String,
    /// Lowercase hex SHA-256 of the artifact.
    sha256: String,
}

/// A newer release the operator can choose to install.
#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    /// The new version.
    pub version: semver::Version,
    /// Release notes from the manifest, if any.
    pub notes: Option<String>,
    /// Signed installer artifact for this platform. `None` when the release
    /// carries no artifact for [`PLATFORM`] — the update is still announced
    /// (never silently swallowed), it just cannot be installed in-app.
    artifact: Option<PlatformArtifact>,
}

#[derive(Clone, Debug)]
struct PlatformArtifact {
    url: String,
    /// Detached minisign signature of the artifact (full `.minisig` contents).
    signature: String,
    /// Lowercase hex SHA-256 of the artifact.
    sha256: String,
}

impl AvailableUpdate {
    /// Whether the release publishes a signed installer for this platform.
    #[must_use]
    pub fn downloadable(&self) -> bool {
        self.artifact.is_some()
    }

    /// Direct download URL of the installer artifact, when one exists.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.artifact.as_ref().map(|artifact| artifact.url.as_str())
    }
}

/// Fetch and verify the manifest; report a newer version when one exists.
///
/// Returns `Ok(None)` when the running version is current (or newer).
///
/// # Errors
/// Any network, signature, or manifest-shape failure. Callers treat errors as
/// "no update today" and stay quiet — never block the app on this.
pub fn check_for_update(current_version: &str) -> Result<Option<AvailableUpdate>, UpdateError> {
    check_with(
        MANIFEST_URL,
        MANIFEST_SIG_URL,
        UPDATE_PUBKEY,
        current_version,
    )
}

/// [`check_for_update`] with injectable endpoints (tests).
///
/// # Errors
/// See [`check_for_update`].
pub fn check_with(
    manifest_url: &str,
    manifest_sig_url: &str,
    pubkey: &str,
    current_version: &str,
) -> Result<Option<AvailableUpdate>, UpdateError> {
    let agent = agent();
    let manifest_bytes = fetch_bytes(&agent, manifest_url, 1024 * 1024)?;
    let signature = fetch_bytes(&agent, manifest_sig_url, 64 * 1024)?;
    verify_signature(pubkey, &manifest_bytes, &signature)?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| UpdateError::BadManifest(error.to_string()))?;
    let latest = parse_version(&manifest.version)?;
    let current = parse_version(current_version)?;
    if latest <= current {
        return Ok(None);
    }
    // A missing platform entry is NOT an error: the newer release is still
    // announced so the operator can fetch it manually from the release page.
    let artifact = manifest
        .platforms
        .get(PLATFORM)
        .map(|entry| PlatformArtifact {
            url: entry.url.clone(),
            signature: entry.signature.clone(),
            sha256: entry.sha256.to_ascii_lowercase(),
        });
    Ok(Some(AvailableUpdate {
        version: latest,
        notes: manifest.notes,
        artifact,
    }))
}

/// Download the installer into `dest_dir`, verifying the SHA-256 from the
/// signed manifest AND the artifact's own minisign signature before returning
/// the final path. A failed verification removes the temp file.
///
/// `progress` receives `(bytes_downloaded, total_bytes_if_known)`.
///
/// # Errors
/// Network, I/O, hash, or signature failures.
pub fn download_update(
    update: &AvailableUpdate,
    dest_dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    download_with(update, UPDATE_PUBKEY, dest_dir, progress)
}

/// [`download_update`] with an injectable public key (tests).
///
/// # Errors
/// See [`download_update`].
pub fn download_with(
    update: &AvailableUpdate,
    pubkey: &str,
    dest_dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    let artifact = update
        .artifact
        .as_ref()
        .ok_or(UpdateError::NoPlatformAsset)?;
    let agent = agent();
    let response = agent
        .get(&artifact.url)
        .call()
        .map_err(|error| UpdateError::Http(error.to_string()))?;

    let file_name = artifact
        .url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !name.contains(['\\', ':']))
        .ok_or_else(|| UpdateError::BadManifest("artifact URL has no file name".to_string()))?;
    std::fs::create_dir_all(dest_dir)?;
    let final_path = dest_dir.join(file_name);
    let temp_path = dest_dir.join(format!("{file_name}.partial"));

    // Any failure past this point (a dropped connection mid-stream, a full disk,
    // a hash/signature mismatch, a failed rename) must not leave a half-written
    // `.partial` behind — a stale partial would masquerade as a resumable
    // download and never be retried cleanly. Clean up on every error path.
    match stream_and_verify(response, &temp_path, artifact, pubkey, progress)
        .and_then(|()| std::fs::rename(&temp_path, &final_path).map_err(UpdateError::Io))
    {
        Ok(()) => Ok(final_path),
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            Err(error)
        }
    }
}

/// Stream the response body into `temp_path`, verifying its SHA-256 and its
/// detached minisign signature against the signed manifest. Leaves the fully
/// written temp file in place on success; the caller renames it and is
/// responsible for removing it on any error.
fn stream_and_verify(
    response: ureq::Response,
    temp_path: &Path,
    artifact: &PlatformArtifact,
    pubkey: &str,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), UpdateError> {
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    let mut reader = response.into_reader().take(MAX_ARTIFACT_BYTES);
    let mut file = std::fs::File::create(temp_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..read])?;
        hasher.update(&buffer[..read]);
        downloaded += read as u64;
        progress(downloaded, total);
    }
    // Flush + sync so a "write returned Ok" that the OS buffered is actually on
    // disk before we trust the hash: a full disk surfaces here, not silently.
    std::io::Write::flush(&mut file)?;
    file.sync_all()?;
    drop(file);

    let digest = hasher.finalize();
    let mut actual = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(actual, "{byte:02x}");
    }
    if actual != artifact.sha256 {
        return Err(UpdateError::BadHash);
    }
    let payload = std::fs::read(temp_path)?;
    verify_signature(pubkey, &payload, artifact.signature.as_bytes())
}

/// Hand the verified installer to the OS and let the app exit.
///
/// Windows spawns `msiexec /i` (the MSI's major-upgrade logic replaces the
/// install; a per-machine install shows the expected UAC prompt). Linux opens
/// the verified `.deb` with the desktop's package installer via `xdg-open` —
/// the operator confirms the privileged install there; nothing runs as root
/// from inside OccluView.
///
/// # Errors
/// Spawn failure, or [`UpdateError::Unsupported`] on platforms without an
/// installer handoff.
pub fn launch_installer(installer: &Path) -> Result<(), UpdateError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("msiexec")
            .arg("/i")
            .arg(installer)
            .spawn()
            .map_err(UpdateError::Io)?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(installer)
            .spawn()
            .map_err(UpdateError::Io)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = installer;
        Err(UpdateError::Unsupported)
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("occluview-update/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn fetch_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let response = agent
        .get(url)
        .call()
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(UpdateError::Io)?;
    Ok(bytes)
}

fn verify_signature(pubkey: &str, message: &[u8], signature: &[u8]) -> Result<(), UpdateError> {
    let key = PublicKey::from_base64(pubkey).map_err(|_| UpdateError::BadSignature)?;
    let signature_text = std::str::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    let signature =
        Signature::decode(signature_text.trim()).map_err(|_| UpdateError::BadSignature)?;
    key.verify(message, &signature, false)
        .map_err(|_| UpdateError::BadSignature)
}

fn parse_version(raw: &str) -> Result<semver::Version, UpdateError> {
    semver::Version::parse(raw.trim().trim_start_matches('v'))
        .map_err(|error| UpdateError::BadManifest(format!("bad version {raw:?}: {error}")))
}

#[cfg(test)]
mod tests;

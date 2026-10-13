use super::protocol::{
    is_fallback_ack, is_native_request, parse_request, serialize_fallback_ack, serialize_request,
    CAPABILITY_ACK, CAPABILITY_PROBE, OPEN_REQUEST_ACK,
};
use super::{OpenRequest, FALLBACK_POLL_INTERVAL};
use crate::desktop::app_paths::app_state_dir;
use anyhow::{Context, Result};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const NATIVE_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const NATIVE_ACK_POLL_INTERVAL: Duration = Duration::from_millis(25);
const ORPHAN_FILE_MAX_AGE: Duration = Duration::from_secs(120);
const MAX_ORPHAN_FILES_SCANNED_PER_POLL: usize = 32;
static NEXT_REQUEST_NONCE: AtomicU64 = AtomicU64::new(0);
static NEXT_ORPHAN_SCAN_START: AtomicU64 = AtomicU64::new(0);

pub(super) enum NativeDiskOutcome {
    Accepted,
    Unsupported,
    Unconfirmed,
}

pub(super) fn write_disk_open_request(request: &OpenRequest) -> Result<()> {
    let dir = open_request_dir().context("locating open request directory")?;
    std::fs::create_dir_all(&dir).context("creating open request directory")?;
    let request_path = dir.join(unique_request_file_name());
    let payload = serialize_request(request)?;
    publish_disk_open_request(&request_path, &payload, |path, payload| {
        std::fs::write(path, payload)
    })
}

/// Negotiate with the disk-queue listener before publishing native path bytes.
/// The same nonce binds each response to this request and prevents an old ack
/// file from being mistaken for a new handoff.
pub(super) fn write_native_disk_open_request(payload: &[u8]) -> Result<NativeDiskOutcome> {
    let dir = open_request_dir().context("locating the single-instance request queue")?;
    std::fs::create_dir_all(&dir).context("creating the single-instance request queue")?;
    let nonce = unique_request_id();
    let probe_path = dir.join(format!("{nonce}.probe"));
    let ack_path = dir.join(format!("{nonce}.ack"));

    publish_native_file(&probe_path, &CAPABILITY_PROBE)
        .context("publishing a path-free single-instance capability probe")?;
    if !wait_for_native_ack(&ack_path, CAPABILITY_ACK, &nonce) {
        let _ = std::fs::remove_file(&probe_path);
        let _ = std::fs::remove_file(&ack_path);
        return Ok(NativeDiskOutcome::Unsupported);
    }
    let _ = std::fs::remove_file(&probe_path);

    let request_path = dir.join(format!("{nonce}.open"));
    if let Err(error) = publish_native_file(&request_path, payload) {
        let _ = std::fs::remove_file(&request_path);
        let _ = std::fs::remove_file(&ack_path);
        return Err(error).context("publishing the native single-instance request");
    }
    if wait_for_native_ack(&ack_path, OPEN_REQUEST_ACK, &nonce) {
        Ok(NativeDiskOutcome::Accepted)
    } else {
        // If it has not been claimed, prevent a late delivery after returning
        // an uncertain result. A claimed request remains in `.processing` and
        // is never retried through another transport.
        let _ = std::fs::remove_file(&request_path);
        let _ = std::fs::remove_file(&ack_path);
        Ok(NativeDiskOutcome::Unconfirmed)
    }
}

fn publish_native_file(path: &Path, payload: &[u8]) -> Result<()> {
    let partial_path = path.with_extension("partial");
    if let Err(error) = std::fs::write(&partial_path, payload) {
        let _ = std::fs::remove_file(&partial_path);
        return Err(error).context("writing a single-instance queue file");
    }
    if let Err(error) = std::fs::rename(&partial_path, path) {
        let _ = std::fs::remove_file(&partial_path);
        return Err(error).context("publishing a single-instance queue file");
    }
    Ok(())
}

fn wait_for_native_ack(path: &Path, kind: &[u8; 8], nonce: &str) -> bool {
    let deadline = Instant::now() + NATIVE_ACK_TIMEOUT;
    loop {
        if let Ok(bytes) = std::fs::read(path) {
            let accepted = is_fallback_ack(&bytes, kind, nonce);
            let _ = std::fs::remove_file(path);
            return accepted;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(NATIVE_ACK_POLL_INTERVAL);
    }
}

fn publish_disk_open_request(
    request_path: &Path,
    payload: &[u8],
    write_payload: impl FnOnce(&Path, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let partial_path = request_path.with_extension("partial");
    if let Err(error) = write_payload(&partial_path, payload) {
        let _ = std::fs::remove_file(&partial_path);
        return Err(error)
            .with_context(|| format!("writing open request {}", request_path.display()));
    }
    if let Err(error) = std::fs::rename(&partial_path, request_path) {
        let _ = std::fs::remove_file(&partial_path);
        return Err(error)
            .with_context(|| format!("publishing open request {}", request_path.display()));
    }
    Ok(())
}

pub(super) fn spawn_disk_fallback_listener(
    sender: mpsc::Sender<OpenRequest>,
    repaint_ctx: egui::Context,
    listener_alive: std::sync::Weak<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        let mut delivered = false;
        if let Some(dir) = open_request_dir() {
            process_native_probes_in(&dir);
            for queued in take_open_requests_in(&dir) {
                if sender.send(queued.request).is_err() {
                    return;
                }
                delivered = true;
                if let Some(native_delivery) = queued.native_delivery {
                    if let Err(error) = finish_native_delivery(&native_delivery) {
                        tracing::warn!(?error, "single-instance native request acknowledgement failed");
                    }
                }
            }
            cleanup_orphaned_native_files(&dir);
        }
        if delivered {
            super::request_open_handoff_repaint(&repaint_ctx);
        }
        // A failed `send` only reports a dead receiver when there is a
        // request to deliver. Without this check an idle listener drop
        // leaves the 20 Hz `read_dir` loop running for the rest of the
        // process, so exit when the listener is gone regardless of
        // whether anything was delivered.
        if listener_alive.upgrade().is_none() {
            return;
        }
        thread::sleep(FALLBACK_POLL_INTERVAL);
    })
}

struct NativeDiskDelivery {
    processing_path: PathBuf,
    ack_path: PathBuf,
    nonce: String,
}

struct QueuedOpenRequest {
    request: OpenRequest,
    native_delivery: Option<NativeDiskDelivery>,
}

fn take_open_requests_in(dir: &Path) -> Vec<QueuedOpenRequest> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("open"))
        .collect::<Vec<_>>();
    files.sort();

    let mut requests = Vec::new();
    for path in files {
        // The request file's own name. Its directory is under the operator's
        // profile, and everything logged here can end up in a crash report.
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        match std::fs::read(&path) {
            Ok(bytes) => match parse_request(&bytes) {
                Ok(request) if !request.paths.is_empty() => {
                    if is_native_request(&bytes) {
                        let Some(nonce) = path.file_stem().and_then(|stem| stem.to_str()) else {
                            let _ = std::fs::remove_file(&path);
                            continue;
                        };
                        if !valid_request_nonce(nonce) {
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                        let processing_path = path.with_extension("processing");
                        if let Err(error) = std::fs::rename(&path, &processing_path) {
                            tracing::warn!(?error, request = %name, "single-instance native request claim failed");
                            continue;
                        }
                        let ack_path = path.with_extension("ack");
                        requests.push(QueuedOpenRequest {
                            request,
                            native_delivery: Some(NativeDiskDelivery {
                                processing_path,
                                ack_path,
                                nonce: nonce.to_owned(),
                            }),
                        });
                        continue;
                    }
                    requests.push(QueuedOpenRequest {
                        request,
                        native_delivery: None,
                    });
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(?error, request = %name, "open request parse failed"),
            },
            Err(error) => {
                tracing::warn!(?error, request = %name, "open request read failed");
            }
        }
        if let Err(error) = std::fs::remove_file(&path) {
            tracing::warn!(?error, request = %name, "open request cleanup failed");
        }
    }
    requests
}

fn process_native_probes_in(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("probe") {
            continue;
        }
        let Some(nonce) = path.file_stem().and_then(|stem| stem.to_str()) else {
            let _ = std::fs::remove_file(path);
            continue;
        };
        if !valid_request_nonce(nonce) {
            let _ = std::fs::remove_file(path);
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) if bytes.as_slice() == CAPABILITY_PROBE.as_slice() => {
                let ack_path = path.with_extension("ack");
                let ack = serialize_fallback_ack(CAPABILITY_ACK, nonce);
                if publish_native_file(&ack_path, &ack).is_ok() {
                    // Publish the nonce-bound acceptance before deleting the
                    // probe, so the sender can distinguish a live new peer.
                    let _ = std::fs::remove_file(path);
                }
            }
            Ok(_) => {
                let _ = std::fs::remove_file(path);
            }
            Err(_) => {}
        }
    }
}

fn finish_native_delivery(delivery: &NativeDiskDelivery) -> Result<()> {
    let ack = serialize_fallback_ack(OPEN_REQUEST_ACK, &delivery.nonce);
    publish_native_file(&delivery.ack_path, &ack)
        .context("publishing a queued native request acknowledgement")?;
    // The request remains claimed until the acknowledgement is visible.
    std::fs::remove_file(&delivery.processing_path)
        .context("removing an acknowledged native request")
}

fn valid_request_nonce(nonce: &str) -> bool {
    !nonce.is_empty()
        && nonce.len() <= 64
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn cleanup_orphaned_native_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries = entries.filter_map(std::result::Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    if entries.is_empty() {
        return;
    }
    let start = NEXT_ORPHAN_SCAN_START.fetch_add(
        MAX_ORPHAN_FILES_SCANNED_PER_POLL as u64,
        Ordering::Relaxed,
    ) as usize
        % entries.len();
    let scan_count = entries.len().min(MAX_ORPHAN_FILES_SCANNED_PER_POLL);
    let now = SystemTime::now();
    for offset in 0..scan_count {
        let entry = &entries[(start + offset) % entries.len()];
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ack" | "probe" | "processing" | "partial")
        ) || !entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= ORPHAN_FILE_MAX_AGE);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn open_request_dir() -> Option<PathBuf> {
    app_state_dir().map(|base| base.join(super::REQUEST_DIR))
}

fn unique_request_file_name() -> String {
    format!("{}.open", unique_request_id())
}

fn unique_request_id() -> String {
    let pid = std::process::id();
    let counter = NEXT_REQUEST_NONCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{nanos}-{pid}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_disk_request_is_visible_only_after_its_payload_is_complete() -> Result<()> {
        use std::io::Write;

        let dir =
            std::env::temp_dir().join(format!("occluview-request-publish-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("request.open");
        let request = OpenRequest {
            paths: vec![PathBuf::from("/tmp/scan.stl")],
            activation_token: Some("launch-token".to_owned()),
        };
        let payload = serialize_request(&request)?;
        publish_disk_open_request(&path, &payload, |writing_path, bytes| {
            let mut file = std::fs::File::create(writing_path)?;
            file.write_all(&bytes[..4])?;
            assert!(
                take_open_requests_in(&dir).is_empty(),
                "unfinished requests stay hidden"
            );
            assert!(
                writing_path.exists(),
                "a poll must not delete a request still being written"
            );
            file.write_all(&bytes[4..])
        })?;
        let received = take_open_requests_in(&dir);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].request.paths, request.paths);
        assert_eq!(
            received[0].request.activation_token,
            request.activation_token
        );
        assert!(
            take_open_requests_in(&dir).is_empty(),
            "a request is delivered once"
        );
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn disk_probe_ack_is_nonce_bound_and_published_before_probe_removal() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "occluview-probe-ack-{}-{}",
            std::process::id(),
            unique_request_id()
        ));
        std::fs::create_dir_all(&dir)?;
        let nonce = "probe-123-45";
        let probe_path = dir.join(format!("{nonce}.probe"));
        let ack_path = dir.join(format!("{nonce}.ack"));
        std::fs::write(&probe_path, CAPABILITY_PROBE)?;

        process_native_probes_in(&dir);

        assert!(
            !probe_path.exists(),
            "probe is removed after its ack is published"
        );
        let ack = std::fs::read(&ack_path)?;
        assert!(is_fallback_ack(&ack, CAPABILITY_ACK, nonce));
        assert!(!is_fallback_ack(&ack, CAPABILITY_ACK, "probe-123-46"));
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_request_is_claimed_until_queue_ack_is_published() -> Result<()> {
        use std::os::unix::ffi::OsStringExt;

        let dir = std::env::temp_dir().join(format!(
            "occluview-native-queue-{}-{}",
            std::process::id(),
            unique_request_id()
        ));
        std::fs::create_dir_all(&dir)?;
        let nonce = "native-123-45";
        let open_path = dir.join(format!("{nonce}.open"));
        let request = OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_vec(
                b"/tmp/native-\xff.stl".to_vec(),
            ))],
            activation_token: None,
        };
        std::fs::write(&open_path, serialize_request(&request)?)?;

        let mut queued = take_open_requests_in(&dir);
        assert_eq!(queued.len(), 1);
        let delivery = queued[0]
            .native_delivery
            .take()
            .expect("native requests require a queue acknowledgement");
        assert!(!open_path.exists());
        assert!(delivery.processing_path.exists());
        assert!(!delivery.ack_path.exists());

        let (sender, receiver) = mpsc::channel();
        sender.send(queued.remove(0).request)?;
        finish_native_delivery(&delivery)?;
        assert_eq!(receiver.recv()?.paths, request.paths);
        assert!(!delivery.processing_path.exists());
        let ack = std::fs::read(&delivery.ack_path)?;
        assert!(is_fallback_ack(&ack, OPEN_REQUEST_ACK, nonce));
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    /// The disk poller must stop once its listener is gone, even when no
    /// request was ever delivered. `send` only reports a dead receiver when
    /// there is something to send, so an idle drop used to leak the 20 Hz
    /// `read_dir` loop for the rest of the process.
    #[test]
    fn disk_fallback_listener_exits_when_listener_is_dropped() {
        // Isolate the poller's request directory so the test exercises the
        // idle path deterministically: no request file may be present.
        let state_dir =
            std::env::temp_dir().join(format!("occluview-fallback-exit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state_dir);
        assert!(std::fs::create_dir_all(&state_dir).is_ok());
        std::env::set_var(crate::desktop::app_paths::TEST_STATE_DIR_ENV, &state_dir);

        let (sender, receiver) = mpsc::channel::<OpenRequest>();
        let listener_alive = std::sync::Arc::new(());
        let poller = spawn_disk_fallback_listener(
            sender,
            egui::Context::default(),
            std::sync::Arc::downgrade(&listener_alive),
        );
        drop(receiver);
        drop(listener_alive);

        let (exited_tx, exited_rx) = mpsc::channel();
        let _joiner = thread::spawn(move || {
            let _ = exited_tx.send(poller.join());
        });
        assert!(
            matches!(exited_rx.recv_timeout(Duration::from_secs(5)), Ok(Ok(()))),
            "disk fallback poller did not exit within 5 s of its listener being dropped"
        );
    }
}

use super::protocol::{parse_request, serialize_request};
use super::{OpenRequest, FALLBACK_POLL_INTERVAL};
use crate::desktop::app_paths::app_state_dir;
use anyhow::{Context, Result};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn write_disk_open_request(request: &OpenRequest) -> Result<()> {
    let dir = open_request_dir().context("locating open request directory")?;
    std::fs::create_dir_all(&dir).context("creating open request directory")?;
    let request_path = dir.join(unique_request_file_name());
    let payload = serialize_request(request)?;
    std::fs::write(&request_path, payload)
        .with_context(|| format!("writing open request {}", request_path.display()))?;
    Ok(())
}

pub(super) fn spawn_disk_fallback_listener(
    sender: mpsc::Sender<OpenRequest>,
    repaint_ctx: egui::Context,
    listener_alive: std::sync::Weak<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        let mut delivered = false;
        for request in take_open_requests() {
            delivered = true;
            if sender.send(request).is_err() {
                return;
            }
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

fn take_open_requests() -> Vec<OpenRequest> {
    let Some(dir) = open_request_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
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
                Ok(request) if !request.paths.is_empty() => requests.push(request),
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

fn open_request_dir() -> Option<PathBuf> {
    app_state_dir().map(|base| base.join(super::REQUEST_DIR))
}

fn unique_request_file_name() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{nanos}-{pid}.open")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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

        let deadline = Instant::now() + Duration::from_secs(5);
        while !poller.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            poller.is_finished(),
            "disk fallback poller did not exit within 5 s of its listener being dropped"
        );
        assert!(poller.join().is_ok(), "disk fallback poller panicked");
    }
}

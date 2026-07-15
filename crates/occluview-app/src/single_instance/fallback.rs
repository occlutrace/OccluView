use super::protocol::{parse_request, serialize_request};
use super::{OpenRequest, FALLBACK_POLL_INTERVAL};
use crate::app_paths::app_state_dir;
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
) {
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
        thread::sleep(FALLBACK_POLL_INTERVAL);
    });
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
        match std::fs::read(&path) {
            Ok(bytes) => match parse_request(&bytes) {
                Ok(request) if !request.paths.is_empty() => requests.push(request),
                Ok(_) => {}
                Err(error) => tracing::warn!(?error, path = ?path, "open request parse failed"),
            },
            Err(error) => {
                tracing::warn!(?error, path = ?path, "open request read failed");
            }
        }
        if let Err(error) = std::fs::remove_file(&path) {
            tracing::warn!(?error, path = ?path, "open request cleanup failed");
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

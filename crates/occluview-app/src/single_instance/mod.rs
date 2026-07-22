//! Single-window handoff for file-association launches.

use anyhow::Result;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

#[cfg(windows)]
use ::windows::Win32::Foundation::{CloseHandle, HANDLE};

mod activation;
mod fallback;
mod protocol;
#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) use activation::{capture_activation_token, complete_startup_notification, RaiseTarget};

/// One file-open handoff from a second instance: the files to open plus, when
/// available, the launcher's window-activation token (used to raise the running
/// window past focus-stealing prevention). See `activation.rs`.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpenRequest {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) activation_token: Option<String>,
}

const REQUEST_DIR: &str = "open-requests";
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(not(windows))]
const LINUX_OPEN_REQUEST_WAKE_BURST_INTERVAL: Duration = Duration::from_millis(25);
#[cfg(not(windows))]
const LINUX_OPEN_REQUEST_WAKE_BURST_STEPS: usize = 48;

pub(crate) struct SingleInstance {
    #[cfg(windows)]
    handle: Option<HANDLE>,
    #[cfg(not(windows))]
    lock_path: Option<PathBuf>,
    secondary: bool,
}

impl SingleInstance {
    pub(crate) fn acquire() -> Result<Self> {
        #[cfg(windows)]
        {
            windows::acquire()
        }

        #[cfg(not(windows))]
        {
            unix::acquire()
        }
    }

    pub(crate) const fn is_secondary(&self) -> bool {
        self.secondary
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(handle) = self.handle.take() {
            // SAFETY: handle was returned by CreateMutexW and is owned here.
            let _ = unsafe { CloseHandle(handle) };
        }

        #[cfg(not(windows))]
        if let Some(path) = self.lock_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(crate) fn write_open_request(request: &OpenRequest) -> Result<()> {
    #[cfg(windows)]
    if windows::send_pipe_open_request(request).is_ok() {
        return Ok(());
    }

    #[cfg(not(windows))]
    if unix::send_socket_open_request(request).is_ok() {
        return Ok(());
    }

    fallback::write_disk_open_request(request)
}

pub(crate) struct OpenRequestListener {
    receiver: Receiver<OpenRequest>,
}

impl OpenRequestListener {
    pub(crate) fn spawn(repaint_ctx: egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        #[cfg(windows)]
        windows::spawn_pipe_listener(sender.clone(), repaint_ctx.clone());
        #[cfg(not(windows))]
        unix::spawn_socket_listener(sender.clone(), repaint_ctx.clone());
        fallback::spawn_disk_fallback_listener(sender, repaint_ctx);
        Self { receiver }
    }

    pub(crate) fn take_requests(&self) -> Vec<OpenRequest> {
        self.receiver.try_iter().collect()
    }
}

fn request_open_handoff_repaint(repaint_ctx: &egui::Context) {
    repaint_ctx.request_repaint();

    #[cfg(not(windows))]
    {
        let repaint_ctx = repaint_ctx.clone();
        std::thread::spawn(move || {
            for _ in 0..LINUX_OPEN_REQUEST_WAKE_BURST_STEPS {
                std::thread::sleep(LINUX_OPEN_REQUEST_WAKE_BURST_INTERVAL);
                repaint_ctx.request_repaint();
            }
        });
    }
}

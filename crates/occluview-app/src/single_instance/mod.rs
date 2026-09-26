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
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_open_files;
mod protocol;
#[cfg(all(not(windows), not(target_os = "macos")))]
mod unix;
#[cfg(windows)]
mod windows;

pub(crate) use activation::{capture_activation_token, complete_startup_notification, RaiseTarget};

/// Add the Finder document handlers as the application finishes launching.
/// Call before the event loop runs. Nothing to do off macOS.
pub(crate) fn install_open_files_handler_at_launch() {
    #[cfg(target_os = "macos")]
    macos_open_files::install_when_launching();
}

/// Add the Finder document handlers now, if the launch observer has not.
/// Nothing to do off macOS.
pub(crate) fn install_open_files_handler() {
    #[cfg(target_os = "macos")]
    macos_open_files::install();
}

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
const OPEN_REQUEST_WAKE_BURST_INTERVAL: Duration = Duration::from_millis(25);
#[cfg(not(windows))]
const OPEN_REQUEST_WAKE_BURST_STEPS: usize = 48;

pub(crate) struct SingleInstance {
    #[cfg(windows)]
    handle: Option<HANDLE>,
    #[cfg(all(not(windows), not(target_os = "macos")))]
    lock_path: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    lock_file: Option<std::fs::File>,
    secondary: bool,
}

impl SingleInstance {
    pub(crate) fn acquire() -> Result<Self> {
        #[cfg(windows)]
        {
            windows::acquire()
        }

        #[cfg(target_os = "macos")]
        {
            macos::acquire()
        }

        #[cfg(all(not(windows), not(target_os = "macos")))]
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

        #[cfg(all(not(windows), not(target_os = "macos")))]
        if let Some(path) = self.lock_path.take() {
            let _ = std::fs::remove_file(path);
        }

        // On macOS the open file descriptor owns the kernel lock. Dropping the
        // field releases it; leave the stable lock file in place so a second
        // process can never race a pathname replacement against the lock.
        #[cfg(target_os = "macos")]
        let _ = self.lock_file.take();
    }
}

pub(crate) fn write_open_request(request: &OpenRequest) -> Result<()> {
    #[cfg(windows)]
    if windows::send_pipe_open_request(request).is_ok() {
        return Ok(());
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    if unix::send_socket_open_request(request).is_ok() {
        return Ok(());
    }

    fallback::write_disk_open_request(request)
}

pub(crate) struct OpenRequestListener {
    receiver: Receiver<OpenRequest>,
}

impl OpenRequestListener {
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::for_tests_with_sender().0
    }

    /// The test listener together with the sender that feeds it, so a test can
    /// hand it a request the way the socket thread does. Without the sender a
    /// test can only observe an empty listener, which proves nothing about the
    /// event-driven path.
    #[cfg(test)]
    pub(crate) fn for_tests_with_sender() -> (Self, mpsc::Sender<OpenRequest>) {
        let (sender, receiver) = mpsc::channel();
        (Self { receiver }, sender)
    }

    pub(crate) fn spawn(repaint_ctx: egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        #[cfg(windows)]
        windows::spawn_pipe_listener(sender.clone(), repaint_ctx.clone());
        #[cfg(all(not(windows), not(target_os = "macos")))]
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
            for _ in 0..OPEN_REQUEST_WAKE_BURST_STEPS {
                std::thread::sleep(OPEN_REQUEST_WAKE_BURST_INTERVAL);
                repaint_ctx.request_repaint();
            }
        });
    }
}

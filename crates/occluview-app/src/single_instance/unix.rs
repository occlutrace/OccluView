use super::protocol::{parse_request, serialize_request, MAX_REQUEST_BYTES};
use super::{OpenRequest, SingleInstance};
use anyhow::{Context, Result};
use eframe::egui;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

const SOCKET_NAME: &str = "open-requests.sock";
const LOCK_FILE_NAME: &str = "single-instance.lock";

pub(super) fn acquire() -> Result<SingleInstance> {
    let lock_path = lock_file_path().context("locating single-instance lock file")?;
    acquire_lock_file(lock_path)
}

pub(super) fn spawn_socket_listener(sender: mpsc::Sender<OpenRequest>, repaint_ctx: egui::Context) {
    thread::spawn(move || {
        let listener = match bind_socket_listener() {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(?error, "single-instance socket listener unavailable");
                return;
            }
        };
        for stream in listener.incoming() {
            match stream.and_then(read_socket_open_request) {
                Ok(Some(request)) => {
                    if sender.send(request).is_err() {
                        return;
                    }
                    super::request_open_handoff_repaint(&repaint_ctx);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(?error, "single-instance socket receive failed"),
            }
        }
    });
}

pub(super) fn send_socket_open_request(request: &OpenRequest) -> Result<()> {
    if request.paths.is_empty() {
        return Ok(());
    }
    let path = socket_path().context("locating single-instance socket")?;
    let payload = serialize_request(request)?;
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting single-instance socket {}", path.display()))?;
    stream
        .write_all(&payload)
        .context("writing single-instance socket request")?;
    Ok(())
}

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|base| base.join("occluview").join(SOCKET_NAME))
}

fn lock_file_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|base| base.join("occluview").join(LOCK_FILE_NAME))
        .or_else(|| crate::app_paths::app_state_dir().map(|base| base.join(LOCK_FILE_NAME)))
}

fn acquire_lock_file(lock_path: PathBuf) -> Result<SingleInstance> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("creating single-instance directory")?;
    }

    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut lock_file) => {
                writeln!(lock_file, "{}", std::process::id())
                    .context("writing single-instance lock file")?;
                return Ok(SingleInstance {
                    lock_path: Some(lock_path),
                    secondary: false,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_file_owner_is_running(&lock_path) {
                    return Ok(SingleInstance {
                        lock_path: None,
                        secondary: true,
                    });
                }
                std::fs::remove_file(&lock_path)
                    .context("removing stale single-instance lock file")?;
            }
            Err(error) => return Err(error).context("creating single-instance lock file"),
        }
    }
}

fn lock_file_owner_is_running(path: &Path) -> bool {
    read_lock_file_pid(path).is_some_and(process_is_running)
}

fn read_lock_file_pid(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    text.trim().parse::<u32>().ok().filter(|pid| *pid > 0)
}

#[cfg(target_os = "linux")]
fn process_is_running(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(not(target_os = "linux"))]
fn process_is_running(_pid: u32) -> bool {
    true
}

fn bind_socket_listener() -> std::io::Result<UnixListener> {
    let path = socket_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(path)
}

fn read_socket_open_request(stream: UnixStream) -> std::io::Result<Option<OpenRequest>> {
    let mut bytes = Vec::new();
    stream
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "single-instance socket request exceeds max size",
        ));
    }
    let request = parse_request(&bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if request.paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_pid_parser_rejects_empty_or_invalid_values() {
        let path = std::env::temp_dir().join(format!("occluview-lock-pid-{}", std::process::id()));

        assert!(std::fs::write(&path, "").is_ok());
        assert_eq!(read_lock_file_pid(&path), None);

        assert!(std::fs::write(&path, "not-a-pid").is_ok());
        assert_eq!(read_lock_file_pid(&path), None);

        assert!(std::fs::write(&path, "0").is_ok());
        assert_eq!(read_lock_file_pid(&path), None);

        assert!(std::fs::write(&path, "42\n").is_ok());
        assert_eq!(read_lock_file_pid(&path), Some(42));

        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_lock_file_owner_detects_current_process() {
        assert!(process_is_running(std::process::id()));
        assert!(!process_is_running(u32::MAX));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_lock_file_is_replaced_by_new_primary() {
        let path = std::env::temp_dir()
            .join(format!("occluview-stale-lock-{}", std::process::id()))
            .join(LOCK_FILE_NAME);
        let parent = path.parent().map(Path::to_path_buf);
        assert!(parent.is_some(), "lock path should have parent");
        let Some(parent) = parent else {
            return;
        };
        assert!(std::fs::create_dir_all(&parent).is_ok());
        assert!(std::fs::write(&path, u32::MAX.to_string()).is_ok());

        let instance = acquire_lock_file(path.clone());
        assert!(instance.is_ok(), "stale lock should be replaced");
        let Ok(instance) = instance else {
            return;
        };

        assert!(!instance.is_secondary());
        assert_eq!(read_lock_file_pid(&path), Some(std::process::id()));
        drop(instance);
        assert!(!path.exists());
        let _ = std::fs::remove_dir(parent);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_lock_file_is_treated_as_secondary() {
        let path = std::env::temp_dir()
            .join(format!("occluview-live-lock-{}", std::process::id()))
            .join(LOCK_FILE_NAME);
        let parent = path.parent().map(Path::to_path_buf);
        assert!(parent.is_some(), "lock path should have parent");
        let Some(parent) = parent else {
            return;
        };
        assert!(std::fs::create_dir_all(&parent).is_ok());
        assert!(std::fs::write(&path, std::process::id().to_string()).is_ok());

        let instance = acquire_lock_file(path.clone());
        assert!(instance.is_ok(), "live lock should be secondary");
        let Ok(instance) = instance else {
            return;
        };

        assert!(instance.is_secondary());
        assert!(path.exists());
        drop(instance);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(parent);
    }
}

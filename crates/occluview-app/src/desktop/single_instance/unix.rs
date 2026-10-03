use super::protocol::{parse_request, serialize_request, MAX_REQUEST_BYTES};
use super::{OpenRequest, SingleInstance};
use anyhow::{Context, Result};
use eframe::egui;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
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
        .or_else(|| {
            crate::desktop::app_paths::app_state_dir().map(|base| base.join(LOCK_FILE_NAME))
        })
}

fn acquire_lock_file(lock_path: PathBuf) -> Result<SingleInstance> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("creating single-instance directory")?;
    }

    let mut lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .context("opening single-instance lock file")?;
    match lock_file.try_lock() {
        Ok(()) => {
            // Ownership is established before publishing the diagnostic PID.
            lock_file
                .set_len(0)
                .context("clearing single-instance PID")?;
            writeln!(lock_file, "{}", std::process::id())
                .context("writing single-instance lock file")?;
            Ok(SingleInstance {
                lock_file: Some(lock_file),
                secondary: false,
            })
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(SingleInstance {
            lock_file: None,
            secondary: true,
        }),
        Err(std::fs::TryLockError::Error(error)) => {
            Err(error).context("acquiring single-instance file lock")
        }
    }
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

/// How long a peer may hold the read open without finishing its request.
///
/// Without a bound, a same-user peer that connects and then sends nothing
/// blocks `read_to_end` indefinitely and wedges the listener thread for the
/// rest of the session, so every later second-instance launch is refused even
/// though the primary is healthy. The bound is generous for a handful of paths
/// on a local socket.
const SOCKET_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn read_socket_open_request(stream: UnixStream) -> std::io::Result<Option<OpenRequest>> {
    stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
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
    fn an_unpublished_pid_cannot_create_a_second_primary() {
        let path =
            std::env::temp_dir().join(format!("occluview-unpublished-lock-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let owner = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path);
        assert!(owner.is_ok());
        let Ok(owner) = owner else {
            panic!("lock setup failed")
        };
        assert!(owner.try_lock().is_ok());
        let contender = acquire_lock_file(path.clone());
        assert!(contender.is_ok());
        let Ok(contender) = contender else {
            panic!("acquisition failed")
        };
        assert!(
            contender.is_secondary(),
            "the primary owns the file before publishing its PID"
        );
        drop(contender);
        drop(owner);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn lock_file_contents_cannot_override_kernel_ownership() {
        let path = std::env::temp_dir().join(format!("occluview-lock-pid-{}", std::process::id()));
        for text in ["", "not-a-pid", "0", "42\n"] {
            assert!(std::fs::write(&path, text).is_ok());
            let instance = acquire_lock_file(path.clone());
            assert!(instance.is_ok());
            let Ok(instance) = instance else {
                panic!("acquisition failed")
            };
            assert!(!instance.is_secondary());
            assert_eq!(
                std::fs::read_to_string(&path).ok(),
                Some(format!("{}\n", std::process::id()))
            );
            drop(instance);
        }
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_current_pid_without_kernel_ownership_does_not_block_startup() {
        let path =
            std::env::temp_dir().join(format!("occluview-current-pid-{}", std::process::id()));
        assert!(std::fs::write(&path, std::process::id().to_string()).is_ok());
        let instance = acquire_lock_file(path.clone());
        assert!(instance.is_ok());
        let Ok(instance) = instance else {
            panic!("acquisition failed")
        };
        assert!(!instance.is_secondary());
        drop(instance);
        let _ = std::fs::remove_file(path);
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
            panic!("required test setup or expected result was missing");
        };
        assert!(std::fs::create_dir_all(&parent).is_ok());
        assert!(std::fs::write(&path, u32::MAX.to_string()).is_ok());

        let instance = acquire_lock_file(path.clone());
        assert!(instance.is_ok(), "stale lock should be replaced");
        let Ok(instance) = instance else {
            panic!("required test setup or expected result was missing");
        };

        assert!(!instance.is_secondary());
        assert_eq!(
            std::fs::read_to_string(&path).ok(),
            Some(format!("{}\n", std::process::id()))
        );
        drop(instance);
        assert!(
            path.exists(),
            "keep the stable inode after releasing ownership"
        );
        let restarted = acquire_lock_file(path.clone());
        assert!(restarted.is_ok());
        let Ok(restarted) = restarted else {
            panic!("restart failed")
        };
        assert!(!restarted.is_secondary());
        drop(restarted);
        let _ = std::fs::remove_file(path);
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
            panic!("required test setup or expected result was missing");
        };
        assert!(std::fs::create_dir_all(&parent).is_ok());
        let primary = acquire_lock_file(path.clone());
        assert!(primary.is_ok());
        let Ok(primary) = primary else {
            panic!("primary acquisition failed")
        };
        assert!(!primary.is_secondary());

        let instance = acquire_lock_file(path.clone());
        assert!(instance.is_ok(), "live lock should be secondary");
        let Ok(instance) = instance else {
            panic!("required test setup or expected result was missing");
        };

        assert!(instance.is_secondary());
        assert!(path.exists());
        drop(instance);
        drop(primary);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(parent);
    }
}

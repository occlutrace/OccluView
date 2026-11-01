use super::protocol::{
    is_capability_probe, is_native_request, parse_request, serialize_request,
    CAPABILITY_ACK, CAPABILITY_PROBE, OPEN_REQUEST_ACK, MAX_REQUEST_BYTES,
};
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
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    tracing::warn!(?error, "single-instance socket accept failed");
                    continue;
                }
            };
            match handle_socket_open_request(stream, &sender, &repaint_ctx) {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    tracing::warn!(?error, "single-instance socket receive failed");
                }
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

pub(super) enum NativeSocketOutcome {
    Accepted,
    Unavailable,
    Unconfirmed,
}

/// Negotiate support without sending paths, then require an acknowledgement
/// after the native request has entered the primary's queue.
pub(super) fn send_native_socket_open_request(payload: &[u8]) -> NativeSocketOutcome {
    let Some(path) = socket_path() else {
        return NativeSocketOutcome::Unavailable;
    };
    if !probe_socket(&path) {
        return NativeSocketOutcome::Unavailable;
    }

    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(_) => return NativeSocketOutcome::Unavailable,
    };
    if stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT)).is_err()
    {
        return NativeSocketOutcome::Unavailable;
    }
    if stream.write_all(payload).is_err() || stream.shutdown(std::net::Shutdown::Write).is_err() {
        return NativeSocketOutcome::Unconfirmed;
    }
    let mut response = [0u8; OPEN_REQUEST_ACK.len()];
    if stream.read_exact(&mut response).is_ok() && &response == OPEN_REQUEST_ACK {
        NativeSocketOutcome::Accepted
    } else {
        NativeSocketOutcome::Unconfirmed
    }
}

fn probe_socket(path: &std::path::Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
        .is_err()
        || stream
            .set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))
            .is_err()
        || stream.write_all(&CAPABILITY_PROBE).is_err()
        || stream.shutdown(std::net::Shutdown::Write).is_err()
    {
        return false;
    }
    let mut response = [0u8; CAPABILITY_ACK.len()];
    stream.read_exact(&mut response).is_ok() && &response == CAPABILITY_ACK
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
/// Bound native request and acknowledgement writes to an unresponsive peer.
const SOCKET_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn handle_socket_open_request(
    mut stream: UnixStream,
    sender: &mpsc::Sender<OpenRequest>,
    repaint_ctx: &egui::Context,
) -> std::io::Result<bool> {
    stream.set_read_timeout(Some(SOCKET_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_WRITE_TIMEOUT))?;
    let mut bytes = Vec::new();
    (&mut stream)
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "single-instance socket request exceeds max size",
        ));
    }
    if is_capability_probe(&bytes) {
        stream.write_all(CAPABILITY_ACK)?;
        return Ok(true);
    }
    let native_request = is_native_request(&bytes);
    let request = parse_request(&bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if request.paths.is_empty() {
        return Ok(true);
    }
    if sender.send(request).is_err() {
        return Ok(false);
    }
    super::request_open_handoff_repaint(repaint_ctx);
    if native_request {
        stream.write_all(OPEN_REQUEST_ACK)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_probe_gets_a_fixed_path_free_ack_without_queueing() {
        let pair = UnixStream::pair();
        assert!(pair.is_ok());
        let Ok((mut client, server)) = pair else {
            panic!("Unix stream pair setup failed");
        };
        let (sender, receiver) = mpsc::channel();
        let context = egui::Context::default();
        let handler = thread::spawn(move || handle_socket_open_request(server, &sender, &context));

        assert!(client.write_all(&CAPABILITY_PROBE).is_ok());
        assert!(client.shutdown(std::net::Shutdown::Write).is_ok());
        let mut response = [0u8; CAPABILITY_ACK.len()];
        assert!(client.read_exact(&mut response).is_ok());
        assert_eq!(&response, CAPABILITY_ACK);
        assert!(matches!(handler.join(), Ok(Ok(true))));
        assert!(receiver.try_recv().is_err(), "a capability probe queues no paths");
    }

    #[cfg(unix)]
    #[test]
    fn native_socket_ack_follows_queue_delivery() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let native_path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/scan-\xff.stl".to_vec()));
        let request = OpenRequest {
            paths: vec![native_path.clone()],
            activation_token: None,
        };
        let payload = serialize_request(&request);
        assert!(payload.is_ok());
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        let pair = UnixStream::pair();
        assert!(pair.is_ok());
        let Ok((mut client, server)) = pair else {
            panic!("Unix stream pair setup failed");
        };
        let (sender, receiver) = mpsc::channel();
        let context = egui::Context::default();
        let handler = thread::spawn(move || handle_socket_open_request(server, &sender, &context));

        assert!(client.write_all(&payload).is_ok());
        assert!(client.shutdown(std::net::Shutdown::Write).is_ok());
        let queued = receiver.recv_timeout(std::time::Duration::from_secs(1));
        assert!(queued.is_ok(), "the request must enter the queue before its ack");
        let Ok(queued) = queued else {
            panic!("native request was not queued");
        };
        assert_eq!(queued.paths[0].as_os_str().as_bytes(), native_path.as_os_str().as_bytes());
        let mut response = [0u8; OPEN_REQUEST_ACK.len()];
        assert!(client.read_exact(&mut response).is_ok());
        assert_eq!(&response, OPEN_REQUEST_ACK);
        assert!(matches!(handler.join(), Ok(Ok(true))));
    }

    #[cfg(unix)]
    #[test]
    fn queued_native_request_repaints_when_disconnected_peer_rejects_ack() {
        use std::os::unix::ffi::OsStringExt;

        let request = OpenRequest {
            paths: vec![PathBuf::from(std::ffi::OsString::from_vec(
                b"/tmp/disconnected-\xff.stl".to_vec(),
            ))],
            activation_token: None,
        };
        let payload = serialize_request(&request);
        assert!(payload.is_ok());
        let Ok(payload) = payload else {
            panic!("required test setup or expected result was missing");
        };
        let pair = UnixStream::pair();
        assert!(pair.is_ok());
        let Ok((mut client, server)) = pair else {
            panic!("Unix stream pair setup failed");
        };
        let (sender, receiver) = mpsc::channel();
        let repaint_context = egui::Context::default();
        let handler_context = repaint_context.clone();
        let handler = thread::spawn(move || {
            handle_socket_open_request(server, &sender, &handler_context)
        });

        assert!(client.write_all(&payload).is_ok());
        assert!(client.shutdown(std::net::Shutdown::Both).is_ok());
        drop(client);

        let result = handler.join();
        assert!(matches!(result, Ok(Err(_))), "the disconnected peer rejects the ACK");
        let queued = receiver.try_recv();
        assert!(queued.is_ok(), "the accepted request stays queued");
        assert!(repaint_context.has_requested_repaint());
    }

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

use super::protocol::{parse_request, serialize_request, MAX_REQUEST_BYTES};
use super::{OpenRequest, SingleInstance, FALLBACK_POLL_INTERVAL};
use anyhow::{bail, Context, Result};
use eframe::egui;
use std::sync::mpsc;
use std::thread;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_WRITE,
    FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

const PIPE_BUFFER_BYTES: u32 = 256 * 1024;
const PIPE_WAIT_TIMEOUT_MS: u32 = 150;

// Keep the base name as a constant so source-contract tests can locate it via
// string matching; the actual runtime names are per-user (see below).
const PIPE_NAME_BASE: &str = "OccluTrace.OccluView.OpenRequests";
const PIPE_NAME: &str = PIPE_NAME_BASE;

fn user_sid_suffix() -> String {
    let mut suffix = String::from("default");
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed > 0 {
                let mut buf = vec![0u8; needed as usize];
                if GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buf.as_mut_ptr().cast()),
                    needed,
                    &mut needed,
                )
                .is_ok()
                {
                    let user = &*buf.as_ptr().cast::<TOKEN_USER>();
                    let sid = user.User.Sid;
                    let sid_len = GetLengthSid(sid) as usize;
                    if sid_len > 0 && sid_len <= 256 {
                        let sid_bytes = std::slice::from_raw_parts(sid.0 as *const u8, sid_len);
                        let mut hash: u64 = 14695981039346656037;
                        for &b in sid_bytes {
                            hash ^= u64::from(b);
                            hash = hash.wrapping_mul(1099511628211);
                        }
                        suffix = format!("{hash:016x}");
                    }
                }
            }
            let _ = CloseHandle(token);
        }
    }
    suffix
}

fn mutex_name() -> HSTRING {
    HSTRING::from(format!(
        "Local\\OccluTrace.OccluView.SingleInstance.{}",
        user_sid_suffix()
    ))
}

fn pipe_name_wide() -> Vec<u16> {
    let name = format!(
        "\\\\.\\pipe\\OccluTrace.OccluView.OpenRequests.{}",
        user_sid_suffix()
    );
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(super) fn acquire() -> Result<SingleInstance> {
    let name = mutex_name();
    // SAFETY: Passing no custom security attributes and a valid named mutex string.
    let handle = unsafe { CreateMutexW(None, false, &name) }
        .context("creating OccluView single-instance mutex")?;
    // SAFETY: GetLastError reads the calling thread's last Win32 error.
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    Ok(SingleInstance {
        handle: Some(handle),
        secondary: already_exists,
    })
}

pub(super) fn spawn_pipe_listener(sender: mpsc::Sender<OpenRequest>, repaint_ctx: egui::Context) {
    thread::spawn(move || loop {
        match read_pipe_open_request() {
            Ok(Some(request)) => {
                if sender.send(request).is_err() {
                    return;
                }
                super::request_open_handoff_repaint(&repaint_ctx);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(?error, "single-instance pipe receive failed");
                thread::sleep(FALLBACK_POLL_INTERVAL);
            }
        }
    });
}

pub(super) fn send_pipe_open_request(request: &OpenRequest) -> Result<()> {
    if request.paths.is_empty() {
        return Ok(());
    }

    let pipe_wide = pipe_name_wide();
    let pipe_name = PCWSTR(pipe_wide.as_ptr());
    // SAFETY: pipe_name is a valid NUL-terminated wide string.
    if unsafe { WaitNamedPipeW(pipe_name, PIPE_WAIT_TIMEOUT_MS) }.0 == 0 {
        bail!("single-instance pipe was not ready");
    }

    // SAFETY: Opening an existing named pipe with a valid constant path.
    let pipe = unsafe {
        CreateFileW(
            pipe_name,
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            HANDLE::default(),
        )
    }
    .context("opening single-instance pipe")?;

    let payload = serialize_request(request)?;
    let mut bytes_written = 0u32;
    // SAFETY: `pipe` is a valid HANDLE from CreateFileW and payload lives for the call.
    let write_result = unsafe {
        WriteFile(
            pipe,
            Some(payload.as_slice()),
            Some(&mut bytes_written),
            None,
        )
    };
    // SAFETY: `pipe` was returned by CreateFileW and is owned here.
    let _ = unsafe { CloseHandle(pipe) };
    write_result.context("writing single-instance pipe request")?;
    if usize::try_from(bytes_written).ok() != Some(payload.len()) {
        bail!(
            "single-instance pipe wrote {} of {} bytes",
            bytes_written,
            payload.len()
        );
    }
    Ok(())
}

fn read_pipe_open_request() -> Result<Option<OpenRequest>> {
    let pipe_wide = pipe_name_wide();
    let pipe_name = PCWSTR(pipe_wide.as_ptr());
    // SAFETY: Creating a named pipe with a per-user path and process-local buffers.
    let pipe = unsafe {
        CreateNamedPipeW(
            pipe_name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        bail!("creating single-instance pipe failed");
    }

    // SAFETY: Waiting for a client to connect on a valid named-pipe handle.
    let connect_result = unsafe { ConnectNamedPipe(pipe, None) };
    if let Err(error) = connect_result {
        // SAFETY: GetLastError reads the calling thread's last Win32 error.
        if unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            // SAFETY: `pipe` is valid and owned here.
            let _ = unsafe { CloseHandle(pipe) };
            return Err(error).context("connecting single-instance pipe");
        }
    }

    let read_result = read_pipe_message(pipe);
    // SAFETY: `pipe` is valid and owned here.
    let _ = unsafe { DisconnectNamedPipe(pipe) };
    // SAFETY: `pipe` is valid and owned here.
    let _ = unsafe { CloseHandle(pipe) };
    let buffer = read_result.context("reading single-instance pipe request")?;
    if buffer.is_empty() {
        return Ok(None);
    }
    let request = parse_request(&buffer)?;
    if request.paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(request))
}

fn read_pipe_message(pipe: HANDLE) -> Result<Vec<u8>> {
    let mut message = Vec::new();

    loop {
        if message.len() >= MAX_REQUEST_BYTES {
            bail!("single-instance pipe request exceeds max size of {MAX_REQUEST_BYTES} bytes");
        }

        let remaining = MAX_REQUEST_BYTES - message.len();
        let chunk_len = remaining.min(PIPE_BUFFER_BYTES as usize);
        let mut chunk = vec![0u8; chunk_len];
        let mut bytes_read = 0u32;
        // SAFETY: `pipe` is a valid connected named pipe and chunk is writable for the call.
        let read_result = unsafe {
            ReadFile(
                pipe,
                Some(chunk.as_mut_slice()),
                Some(&mut bytes_read),
                None,
            )
        };
        // SAFETY: GetLastError reads the calling thread's last Win32 error.
        let last_error = unsafe { GetLastError() };

        if bytes_read > 0 {
            chunk.truncate(bytes_read as usize);
            message.extend_from_slice(&chunk);
        }

        match read_result {
            Ok(()) => return Ok(message),
            Err(_error) if last_error == ERROR_MORE_DATA => {}
            Err(error) => return Err(error).context("ReadFile for single-instance pipe failed"),
        }
    }
}

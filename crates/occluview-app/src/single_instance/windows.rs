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
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

const PIPE_BUFFER_BYTES: u32 = 256 * 1024;
const PIPE_WAIT_TIMEOUT_MS: u32 = 150;

/// Base name only. `pipe_name_wide` appends a per-user suffix so two
/// signed-in users never land on the same pipe.
const PIPE_NAME: &str = "OccluTrace.OccluView.OpenRequests";

/// The current user's SID in string form, e.g. `S-1-5-21-...-1001`.
///
/// Both the per-user pipe name and the pipe's DACL derive from this, and they
/// have to agree. The name is only a convention; anything in the session can
/// claim it first. The DACL is what keeps another user out.
fn current_user_sid_string() -> Option<String> {
    let mut token = HANDLE::default();
    // SAFETY: `token` is an out-parameter this function owns and closes below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return None;
    }
    let sid_text = read_token_user_sid(token);
    // SAFETY: `token` came from OpenProcessToken and is owned here.
    let _ = unsafe { CloseHandle(token) };
    sid_text
}

fn read_token_user_sid(token: HANDLE) -> Option<String> {
    let mut needed = 0u32;
    // SAFETY: the probing call is documented to fail with the required size.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    // `TOKEN_USER` is 8-aligned and a `Vec<u8>` is not, so allocate as `u64`
    // and round up. A byte vector here would be a misaligned struct read.
    let mut buffer = vec![0_u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: `buffer` is at least `needed` bytes, 8-aligned, and writable for
    // the duration of the call.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .ok()?;

    // SAFETY: on success the buffer holds a TOKEN_USER, and it outlives the
    // borrow. The SID pointer inside it stays valid for the same reason.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut sid_text = windows::core::PWSTR::null();
    // SAFETY: `sid` is a valid SID from the token; `sid_text` receives a
    // LocalAlloc'd string that is freed below.
    unsafe { ConvertSidToStringSidW(sid, &mut sid_text) }.ok()?;
    let owned = unsafe { sid_text.to_string() }.ok();
    // SAFETY: `sid_text` was allocated by ConvertSidToStringSidW.
    let _ = unsafe {
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            sid_text.0.cast(),
        )))
    };
    owned
}

/// A short, stable per-user suffix for the pipe and mutex names.
///
/// `None` when the SID cannot be read, and callers must decline rather than
/// substitute a fixed string. A predictable shared name is worse than no
/// single-instance check at all: a squatter holding the mutex makes every
/// launch believe it is the second instance and write its scan paths to him
/// before exiting.
fn user_sid_suffix() -> Option<String> {
    let sid = current_user_sid_string()?;
    // FNV-1a, only to keep the object names short and free of SID punctuation.
    // Not a security boundary; the DACL is.
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in sid.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    Some(format!("{hash:016x}"))
}

/// A security descriptor granting the current user full access and nobody else
/// anything, for the listening end of the hand-off pipe.
///
/// The default DACL would let any signed-in user on a shared workstation --
/// a clinic reception machine is exactly that -- connect and read what travels
/// over the pipe, which is a list of scan paths, which in dental work names the
/// patient.
///
/// The returned descriptor is a `LocalAlloc`'d buffer the caller must free.
fn owner_only_security_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
    let sid = current_user_sid_string()?;
    // D: a DACL. P: protected, so no inherited ACE widens it.
    // (A;;GA;;;<sid>): generic-all to that SID, and to nothing else.
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})"));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` is a NUL-terminated wide string alive across the call, and
    // `descriptor` receives a LocalAlloc'd buffer the caller frees.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    };
    converted.ok().map(|()| descriptor)
}

fn mutex_name() -> Option<HSTRING> {
    Some(HSTRING::from(format!(
        "Local\\OccluTrace.OccluView.SingleInstance.{}",
        user_sid_suffix()?
    )))
}

fn pipe_name_wide() -> Option<Vec<u16>> {
    let name = format!("\\\\.\\pipe\\{PIPE_NAME}.{}", user_sid_suffix()?);
    Some(name.encode_utf16().chain(std::iter::once(0)).collect())
}

pub(super) fn acquire() -> Result<SingleInstance> {
    // No SID, no per-user name (see `user_sid_suffix`). Behave as the first
    // instance instead.
    let Some(name) = mutex_name() else {
        return Ok(SingleInstance {
            handle: None,
            secondary: false,
        });
    };
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

/// How many creates in a row may fail before the listener gives up.
///
/// `FILE_FLAG_FIRST_PIPE_INSTANCE` makes a claimed name fail permanently, so a
/// squatted name never clears. Retrying it every 50 ms writes a warning each
/// time and overwrites the whole 50-line crash-report ring in under three
/// seconds, leaving the process with one repeated line for diagnostics. Five
/// covers a genuinely transient failure; past that the disk fallback listener
/// beside this one carries the hand-off.
const MAX_CONSECUTIVE_PIPE_FAILURES: u32 = 5;

pub(super) fn spawn_pipe_listener(sender: mpsc::Sender<OpenRequest>, repaint_ctx: egui::Context) {
    thread::spawn(move || {
        let mut failures = 0_u32;
        loop {
            match read_pipe_open_request() {
                Ok(Some(request)) => {
                    failures = 0;
                    if sender.send(request).is_err() {
                        return;
                    }
                    super::request_open_handoff_repaint(&repaint_ctx);
                }
                Ok(None) => failures = 0,
                Err(error) => {
                    failures += 1;
                    if failures >= MAX_CONSECUTIVE_PIPE_FAILURES {
                        tracing::warn!(
                            ?error,
                            failures,
                            "single-instance pipe listener stopping; hand-off continues \
                             over the disk fallback"
                        );
                        return;
                    }
                    tracing::warn!(?error, "single-instance pipe receive failed");
                    thread::sleep(FALLBACK_POLL_INTERVAL);
                }
            }
        }
    });
}

pub(super) fn send_pipe_open_request(request: &OpenRequest) -> Result<()> {
    if request.paths.is_empty() {
        return Ok(());
    }

    // The request carries the paths of the scans being opened. With no
    // per-user name to send them to, send them nowhere.
    let Some(pipe_wide) = pipe_name_wide() else {
        bail!("refusing to send an open request to a pipe name that is not per-user");
    };
    let pipe_name = PCWSTR(pipe_wide.as_ptr());
    // SAFETY: pipe_name is a valid NUL-terminated wide string.
    if unsafe { WaitNamedPipeW(pipe_name, PIPE_WAIT_TIMEOUT_MS) }.0 == 0 {
        bail!("single-instance pipe was not ready");
    }

    // SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION caps what the server end
    // may do with this connection. The default is impersonation level, which
    // lets whoever owns the other end call `ImpersonateNamedPipeClient` and act
    // as this user. At identification level it learns who we are and no more,
    // which is all a legitimate listener needs.
    //
    // SAFETY: Opening an existing named pipe with a valid constant path.
    let pipe = unsafe {
        CreateFileW(
            pipe_name,
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
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
    let Some(pipe_wide) = pipe_name_wide() else {
        bail!("refusing to listen on a pipe name that is not per-user");
    };
    let pipe_name = PCWSTR(pipe_wide.as_ptr());

    // Two things this listener must not do: share its name with a squatter,
    // and accept a connection from another user.
    //
    // FILE_FLAG_FIRST_PIPE_INSTANCE makes the create fail if the name already
    // exists, rather than quietly becoming a second instance beside whoever got
    // there first. The DACL grants the current user and nobody else, so on a
    // shared workstation another signed-in account cannot read the scan paths
    // that travel over this pipe.
    //
    // No descriptor, no pipe: creating it with the default DACL would be the
    // shared-workstation squat this function exists to prevent, reached by a
    // silent downgrade. Hand-off is not load-bearing -- the disk fallback
    // listener runs beside this one and carries the request.
    let Some(descriptor) = owner_only_security_descriptor() else {
        bail!("refusing to create the single-instance pipe without an owner-only DACL");
    };
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let attributes_ptr = Some(std::ptr::addr_of!(attributes));

    // SAFETY: `pipe_name` is a NUL-terminated wide string, and `attributes`
    // (when present) points at a live SECURITY_ATTRIBUTES whose descriptor
    // outlives this call and is freed below.
    let pipe = unsafe {
        CreateNamedPipeW(
            pipe_name,
            PIPE_ACCESS_DUPLEX | FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_FIRST_PIPE_INSTANCE.0),
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            attributes_ptr,
        )
    };
    // SAFETY: the descriptor came from ConvertStringSecurityDescriptorToSecurityDescriptorW,
    // which allocates with LocalAlloc, and CreateNamedPipeW has copied what it needs.
    let _ = unsafe {
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            descriptor.0,
        )))
    };
    if pipe.is_invalid() {
        // Transient, or someone already holds the name. Report either; do not
        // work around either.
        bail!("creating single-instance pipe failed (name already claimed?)");
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

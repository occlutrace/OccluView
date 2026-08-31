#![cfg_attr(all(windows, feature = "diagnostic-logs"), allow(unsafe_code))]

use crate::ShellError;
use occluview_render::RenderError;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

#[cfg(all(windows, feature = "diagnostic-logs"))]
use std::sync::{mpsc, Mutex, OnceLock};
#[cfg(all(windows, feature = "diagnostic-logs"))]
use windows::core::{w, PCWSTR};
#[cfg(all(windows, feature = "diagnostic-logs"))]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(all(windows, feature = "diagnostic-logs"))]
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    REG_DWORD, REG_VALUE_TYPE,
};
#[cfg(all(windows, feature = "diagnostic-logs"))]
use windows::Win32::UI::Shell::{FOLDERID_LocalAppDataLow, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

const MAX_DIAGNOSTIC_LOG_BYTES: u64 = 64 * 1024;
#[cfg(all(windows, feature = "diagnostic-logs"))]
const DIAGNOSTIC_QUEUE_CAPACITY: usize = 16;
#[cfg(all(windows, feature = "diagnostic-logs"))]
const DIAGNOSTIC_REGISTRY_KEY: PCWSTR = w!("Software\\OccluTrace\\OccluView\\Diagnostics");
#[cfg(all(windows, feature = "diagnostic-logs"))]
const DIAGNOSTIC_ENABLED_VALUE: PCWSTR = w!("PreviewFailureLogEnabled");
#[cfg(all(windows, feature = "diagnostic-logs"))]
const ERROR_SUCCESS: u32 = 0;
#[cfg(all(windows, feature = "diagnostic-logs"))]
const ERROR_FILE_NOT_FOUND: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewFailureStage {
    Render,
    Bitmap,
}

impl PreviewFailureStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Bitmap => "bitmap",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewFailureCategory {
    ReadbackTimeout,
    Renderer,
    Format,
    Windows,
}

impl PreviewFailureCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReadbackTimeout => "readback_timeout",
            Self::Renderer => "renderer",
            Self::Format => "format",
            Self::Windows => "windows",
        }
    }
}

fn category_for_shell_error(error: &ShellError) -> PreviewFailureCategory {
    match error {
        ShellError::Render(RenderError::ReadbackTimeout { .. }) => {
            PreviewFailureCategory::ReadbackTimeout
        }
        ShellError::Render(_) => PreviewFailureCategory::Renderer,
        ShellError::Format(_) => PreviewFailureCategory::Format,
        ShellError::Win32(_) => PreviewFailureCategory::Windows,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviewFailureEvent {
    timestamp_unix_ms: u128,
    process_id: u32,
    stage: PreviewFailureStage,
    category: PreviewFailureCategory,
}

impl PreviewFailureEvent {
    fn for_shell_error(
        error: &ShellError,
        stage: PreviewFailureStage,
        timestamp_unix_ms: u128,
        process_id: u32,
    ) -> Self {
        Self {
            timestamp_unix_ms,
            process_id,
            stage,
            category: category_for_shell_error(error),
        }
    }

    const fn bitmap_failure(timestamp_unix_ms: u128, process_id: u32) -> Self {
        Self {
            timestamp_unix_ms,
            process_id,
            stage: PreviewFailureStage::Bitmap,
            category: PreviewFailureCategory::Windows,
        }
    }

    fn json_line(self) -> String {
        format!(
            "{{\"timestamp_unix_ms\":{},\"pid\":{},\"stage\":\"{}\",\"category\":\"{}\"}}\n",
            self.timestamp_unix_ms,
            self.process_id,
            self.stage.as_str(),
            self.category.as_str(),
        )
    }
}

#[cfg(test)]
fn preview_failure_event_json(
    error: &ShellError,
    stage: PreviewFailureStage,
    timestamp_unix_ms: u128,
    process_id: u32,
) -> String {
    PreviewFailureEvent::for_shell_error(error, stage, timestamp_unix_ms, process_id).json_line()
}

fn append_diagnostic_line(path: &Path, line: &str) -> std::io::Result<()> {
    let line_len = u64::try_from(line.len()).unwrap_or(MAX_DIAGNOSTIC_LOG_BYTES);
    if line_len > MAX_DIAGNOSTIC_LOG_BYTES {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut log = OpenOptions::new().append(true).create(true).open(path)?;
    let current_len = log.metadata()?.len();
    if current_len > MAX_DIAGNOSTIC_LOG_BYTES.saturating_sub(line_len) {
        return Ok(());
    }
    log.write_all(line.as_bytes())
}

const fn diagnostic_switch_enabled(value: Option<u32>) -> bool {
    matches!(value, Some(1))
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
static DIAGNOSTIC_SENDER: OnceLock<mpsc::SyncSender<PreviewFailureEvent>> = OnceLock::new();
#[cfg(all(windows, feature = "diagnostic-logs"))]
static DIAGNOSTIC_WRITER_INIT: OnceLock<Mutex<()>> = OnceLock::new();

/// Record a preview render failure without allowing diagnostics I/O to delay the
/// preview host. The normal customer package does not compile this function.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn record_preview_render_failure(error: &ShellError) {
    record_preview_failure(PreviewFailureEvent::for_shell_error(
        error,
        PreviewFailureStage::Render,
        current_unix_ms(),
        std::process::id(),
    ));
}

/// Perform the one-time opt-in check and worker startup off the COM callback
/// stack, before a deferred preview render may emit an error.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn prepare_preview_diagnostics() {
    let _ = diagnostic_sender();
}

/// Records a GDI conversion failure as a fixed category, never the Windows
/// error text returned by the operating system.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn record_preview_bitmap_failure() {
    record_preview_failure(PreviewFailureEvent::bitmap_failure(
        current_unix_ms(),
        std::process::id(),
    ));
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn record_preview_failure(event: PreviewFailureEvent) {
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        let _ = sender.try_send(event);
    }
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn diagnostic_sender() -> Option<&'static mpsc::SyncSender<PreviewFailureEvent>> {
    // Do not cache a disabled result: the operator may enable this per-user
    // switch while the private preview surrogate is already alive. A later
    // deferred render observes the new setting without touching the callback
    // path or requiring an Explorer restart.
    if !diagnostic_switch_enabled(preview_diagnostic_switch_value()) {
        return None;
    }
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        return Some(sender);
    }

    // Initialization only runs from deferred preview work, never a COM
    // callback. The short mutex makes construction single-writer on stable
    // Rust; the sender remains lock-free for the error callback below.
    let _guard = diagnostic_writer_init_mutex().lock().ok()?;
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        return Some(sender);
    }

    // The diagnostic worker executes Rust code after the render callback
    // returns. Pin this DLL before creating it so a COM unload cannot unmap
    // the worker's code. A failed initialization is retried by a later
    // deferred render and never caches a disabled or failed state.
    crate::com::own_pinned_dll_module().ok()?;
    let (sender, receiver) = mpsc::sync_channel(DIAGNOSTIC_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("occluview-preview-diagnostics".to_owned())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                write_preview_failure_event(event);
            }
        })
        .ok()?;
    let _ = DIAGNOSTIC_SENDER.set(sender);
    DIAGNOSTIC_SENDER.get()
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn diagnostic_writer_init_mutex() -> &'static Mutex<()> {
    DIAGNOSTIC_WRITER_INIT.get_or_init(|| Mutex::new(()))
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn write_preview_failure_event(event: PreviewFailureEvent) {
    let Some(path) = local_app_data_low_path() else {
        return;
    };
    let path = path
        .join("OccluView")
        .join("diagnostics")
        .join("preview-failures.jsonl");
    let _ = append_diagnostic_line(&path, &event.json_line());
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn current_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn preview_diagnostic_switch_value() -> Option<u32> {
    let mut key = HKEY::default();
    // SAFETY: `key` is a stack out-param and the literal is NUL-terminated.
    let open = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            DIAGNOSTIC_REGISTRY_KEY,
            None,
            KEY_QUERY_VALUE,
            &raw mut key,
        )
    };
    if open.0 != ERROR_SUCCESS {
        return None;
    }
    let value = query_registry_dword(key, DIAGNOSTIC_ENABLED_VALUE);
    // SAFETY: `key` was opened successfully above.
    let _ = unsafe { RegCloseKey(key) };
    value
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn query_registry_dword(key: HKEY, name: PCWSTR) -> Option<u32> {
    let mut value_type = REG_VALUE_TYPE::default();
    let mut bytes = [0_u8; 4];
    let mut byte_len = u32::try_from(bytes.len()).ok()?;
    // SAFETY: `bytes` is a four-byte output buffer and `byte_len` describes
    // its size. `name` is a live NUL-terminated literal.
    let query = unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            Some(&raw mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&raw mut byte_len),
        )
    };
    if query.0 == ERROR_FILE_NOT_FOUND {
        return None;
    }
    if query.0 == ERROR_SUCCESS && value_type == REG_DWORD && byte_len >= 4 {
        Some(u32::from_le_bytes(bytes))
    } else {
        None
    }
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn local_app_data_low_path() -> Option<std::path::PathBuf> {
    // SAFETY: Windows allocates a NUL-terminated result on success and the
    // current token is selected by passing None.
    let raw_path =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppDataLow, KF_FLAG_DEFAULT, None).ok()? };
    // SAFETY: `raw_path` is the NUL-terminated buffer returned above.
    let path = unsafe { raw_path.to_string().ok() };
    // SAFETY: SHGetKnownFolderPath allocates with the COM task allocator.
    unsafe { CoTaskMemFree(Some(raw_path.0.cast())) };
    path.map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_render::RenderError;
    use std::time::Duration;

    #[test]
    fn timeout_failure_event_uses_only_fixed_safe_fields() {
        let timeout = ShellError::Render(RenderError::ReadbackTimeout {
            timeout: Duration::from_secs(2),
        });

        assert_eq!(
            preview_failure_event_json(&timeout, PreviewFailureStage::Render, 1_725_000_001, 42),
            "{\"timestamp_unix_ms\":1725000001,\"pid\":42,\"stage\":\"render\",\"category\":\"readback_timeout\"}\n"
        );
    }

    #[test]
    fn bounded_diagnostic_log_preserves_existing_data_when_the_next_event_does_not_fit() {
        let root = std::env::temp_dir().join(format!(
            "occluview-preview-diagnostics-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&root).expect("temporary diagnostic directory");
        let path = root.join("preview-failures.jsonl");
        std::fs::write(&path, vec![b'x'; MAX_DIAGNOSTIC_LOG_BYTES as usize - 1])
            .expect("seed bounded diagnostic log");
        let line = preview_failure_event_json(
            &ShellError::Render(RenderError::ReadbackTimeout {
                timeout: Duration::from_secs(2),
            }),
            PreviewFailureStage::Render,
            1,
            2,
        );

        append_diagnostic_line(&path, &line).expect("full log is a no-op, not a failure");

        assert_eq!(
            std::fs::metadata(&path)
                .expect("bounded diagnostic log metadata")
                .len(),
            MAX_DIAGNOSTIC_LOG_BYTES - 1,
            "diagnostic logging must never grow an already-full file"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostic_event_appends_one_fixed_json_line() {
        let root = std::env::temp_dir().join(format!(
            "occluview-preview-diagnostics-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        ));
        let path = root.join("preview-failures.jsonl");
        let line = preview_failure_event_json(
            &ShellError::Render(RenderError::ReadbackTimeout {
                timeout: Duration::from_secs(2),
            }),
            PreviewFailureStage::Render,
            1,
            2,
        );

        append_diagnostic_line(&path, &line).expect("append safe diagnostic event");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read diagnostic event"),
            line,
            "diagnostic persistence must write the fixed event verbatim"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bitmap_failure_event_uses_a_fixed_windows_category() {
        assert_eq!(
            PreviewFailureEvent::bitmap_failure(1, 2).json_line(),
            "{\"timestamp_unix_ms\":1,\"pid\":2,\"stage\":\"bitmap\",\"category\":\"windows\"}\n"
        );
    }

    #[test]
    fn diagnostic_logging_is_disabled_when_the_registry_value_is_absent_or_zero() {
        assert!(!diagnostic_switch_enabled(None));
        assert!(!diagnostic_switch_enabled(Some(0)));
        assert!(diagnostic_switch_enabled(Some(1)));
    }

    #[test]
    fn preparation_rechecks_the_opt_in_switch_before_the_writer_is_cached() {
        // A diagnostic handler can be activated before the operator runs the
        // helper. Caching `None` then makes a later opt-in inert until the
        // surrogate happens to restart. The sender cache must only represent
        // a running writer; every deferred preparation checks the current
        // per-user setting first.
        let source = include_str!("preview_diagnostics.rs");
        let sender_declaration = source
            .lines()
            .find(|line| line.contains("static DIAGNOSTIC_SENDER"))
            .expect("diagnostic sender declaration");
        assert!(
            sender_declaration.contains("OnceLock<mpsc::SyncSender<PreviewFailureEvent>>"),
            "the writer cache must not persist the disabled state"
        );
        assert!(
            !sender_declaration.contains("Option<"),
            "the sender declaration must not cache a disabled result"
        );
        let start = source
            .find(
                "fn diagnostic_sender() -> Option<&'static mpsc::SyncSender<PreviewFailureEvent>>",
            )
            .expect("diagnostic sender");
        let end = source[start..]
            .find("fn write_preview_failure_event")
            .map(|offset| start + offset)
            .expect("writer after sender");
        let sender = &source[start..end];
        let switch_check = sender
            .find("if !diagnostic_switch_enabled(preview_diagnostic_switch_value())")
            .expect("per-user opt-in check");
        let writer_initialization = sender
            .find("diagnostic_writer_init_mutex().lock()")
            .expect("race-safe diagnostic writer initialization");
        assert!(
            switch_check < writer_initialization,
            "each deferred preparation must re-read opt-in before consulting a cached writer"
        );
        assert!(
            sender.contains("let _guard = diagnostic_writer_init_mutex().lock().ok()?;"),
            "concurrent preparations must serialize diagnostic writer initialization"
        );
        assert!(
            sender.contains("DIAGNOSTIC_SENDER.set(sender)"),
            "the one writer constructed under the deferred init lock must become the callback sender"
        );
    }

    #[test]
    fn failure_event_never_serializes_format_or_windows_error_display_text() {
        let format_error = ShellError::Format(occluview_formats::FormatError::UnsafePath {
            format: "3mf",
            path: "C:\\Patients\\Ada Lovelace\\private.3mf".to_owned(),
        });
        let windows_error =
            ShellError::Win32("could not open C:\\Patients\\Ada Lovelace\\private.stl".to_owned());

        for error in [&format_error, &windows_error] {
            let line = preview_failure_event_json(error, PreviewFailureStage::Render, 1, 2);
            assert!(line.contains("\"category\":"));
            assert!(
                !line.contains("Patients")
                    && !line.contains("Ada Lovelace")
                    && !line.contains("private"),
                "diagnostic event must not serialize an error's Display text: {line}"
            );
        }
    }

    #[test]
    fn error_callback_only_attempts_a_nonblocking_send() {
        let source = include_str!("preview_diagnostics.rs");
        let start = source
            .find("fn record_preview_failure(event: PreviewFailureEvent)")
            .expect("failure callback");
        let end = source[start..]
            .find("fn diagnostic_sender()")
            .map(|offset| start + offset)
            .expect("diagnostic initialization after callback");
        let callback = &source[start..end];

        assert!(callback.contains("sender.try_send(event)"));
        assert!(
            !callback.contains("preview_diagnostic_switch_value")
                && !callback.contains("diagnostic_sender()"),
            "the render-error callback must not open the registry or initialize a worker"
        );
    }
}

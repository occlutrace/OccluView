#![cfg_attr(all(windows, feature = "diagnostic-logs"), allow(unsafe_code))]
#![cfg_attr(test, allow(dead_code))]

//! Privacy-safe, opt-in lifecycle diagnostics for the Windows Shell hosts.
//!
//! These events intentionally contain only stable enums and elapsed time. A
//! support bundle must never collect scan names, paths, mesh data, raw driver
//! strings, or arbitrary error text from a dental workstation.

use crate::ShellError;
use occluview_render::RenderError;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

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
const DIAGNOSTIC_ENABLED_VALUE: PCWSTR = w!("ShellEventLogEnabled");
#[cfg(all(windows, feature = "diagnostic-logs"))]
const LEGACY_DIAGNOSTIC_ENABLED_VALUE: PCWSTR = w!("PreviewFailureLogEnabled");
#[cfg(all(windows, feature = "diagnostic-logs"))]
const ERROR_SUCCESS: u32 = 0;
#[cfg(all(windows, feature = "diagnostic-logs"))]
const ERROR_FILE_NOT_FOUND: u32 = 2;

/// Which independent Windows shell component emitted the event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellDiagnosticComponent {
    Preview,
    Thumbnail,
}

impl ShellDiagnosticComponent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Thumbnail => "thumbnail",
        }
    }

    const fn process_role(self) -> &'static str {
        match self {
            Self::Preview => "preview_host",
            Self::Thumbnail => "thumbnail_surrogate",
        }
    }
}

/// A bounded point in one Shell activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellDiagnosticStage {
    Activation,
    Source,
    SceneLoad,
    Adapter,
    Render,
    BitmapPublish,
    ComReturn,
}

impl ShellDiagnosticStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Activation => "activation",
            Self::Source => "source",
            Self::SceneLoad => "scene_load",
            Self::Adapter => "adapter",
            Self::Render => "render",
            Self::BitmapPublish => "bitmap_publish",
            Self::ComReturn => "com_return",
        }
    }
}

/// Whether a stage began, completed, or failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellDiagnosticOutcome {
    Started,
    Completed,
    Failed,
}

impl ShellDiagnosticOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Adapter class, deliberately excluding a GPU name, driver, or identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellDiagnosticAdapter {
    NotObserved,
    Hardware,
    Fallback,
}

impl ShellDiagnosticAdapter {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NotObserved => "not_observed",
            Self::Hardware => "hardware",
            Self::Fallback => "fallback",
        }
    }
}

/// Coarse failure category, never an operating-system or parser message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellDiagnosticErrorClass {
    None,
    Deadline,
    Renderer,
    Format,
    Windows,
    Transient,
}

impl ShellDiagnosticErrorClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Deadline => "deadline",
            Self::Renderer => "renderer",
            Self::Format => "format",
            Self::Windows => "windows",
            Self::Transient => "transient",
        }
    }
}

/// One fixed-field lifecycle event written by the optional diagnostic package.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShellDiagnosticEvent {
    timestamp_unix_ms: u128,
    process_id: u32,
    component: ShellDiagnosticComponent,
    stage: ShellDiagnosticStage,
    outcome: ShellDiagnosticOutcome,
    adapter: ShellDiagnosticAdapter,
    error_class: ShellDiagnosticErrorClass,
    elapsed_ms: u64,
}

/// Stable event fields supplied by the shell callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShellDiagnosticEventInput {
    pub(crate) component: ShellDiagnosticComponent,
    pub(crate) stage: ShellDiagnosticStage,
    pub(crate) adapter: ShellDiagnosticAdapter,
    pub(crate) elapsed_ms: u64,
}

/// Process identity captured once for one diagnostic event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShellDiagnosticProcess {
    pub(crate) timestamp_unix_ms: u128,
    pub(crate) process_id: u32,
}

impl ShellDiagnosticEvent {
    pub(crate) const fn normal(
        input: ShellDiagnosticEventInput,
        outcome: ShellDiagnosticOutcome,
        process: ShellDiagnosticProcess,
    ) -> Self {
        Self {
            timestamp_unix_ms: process.timestamp_unix_ms,
            process_id: process.process_id,
            component: input.component,
            stage: input.stage,
            outcome,
            adapter: input.adapter,
            error_class: ShellDiagnosticErrorClass::None,
            elapsed_ms: input.elapsed_ms,
        }
    }

    pub(crate) const fn failure(
        input: ShellDiagnosticEventInput,
        error_class: ShellDiagnosticErrorClass,
        process: ShellDiagnosticProcess,
    ) -> Self {
        Self {
            timestamp_unix_ms: process.timestamp_unix_ms,
            process_id: process.process_id,
            component: input.component,
            stage: input.stage,
            outcome: ShellDiagnosticOutcome::Failed,
            adapter: input.adapter,
            error_class,
            elapsed_ms: input.elapsed_ms,
        }
    }

    #[cfg(test)]
    pub(crate) fn json_line(self) -> String {
        self.json_line_inner()
    }

    fn json_line_inner(self) -> String {
        format!(
            concat!(
                "{{\"timestamp_unix_ms\":{},\"pid\":{},\"component\":\"{}\",",
                "\"process_role\":\"{}\",\"stage\":\"{}\",\"outcome\":\"{}\",",
                "\"adapter\":\"{}\",\"error_class\":\"{}\",\"elapsed_ms\":{}}}\n"
            ),
            self.timestamp_unix_ms,
            self.process_id,
            self.component.as_str(),
            self.component.process_role(),
            self.stage.as_str(),
            self.outcome.as_str(),
            self.adapter.as_str(),
            self.error_class.as_str(),
            self.elapsed_ms,
        )
    }
}

const fn error_class_for_shell_error(error: &ShellError) -> ShellDiagnosticErrorClass {
    match error {
        ShellError::Render(RenderError::ReadbackTimeout { .. }) => {
            ShellDiagnosticErrorClass::Deadline
        }
        ShellError::Render(_) => ShellDiagnosticErrorClass::Renderer,
        ShellError::Format(_) => ShellDiagnosticErrorClass::Format,
        ShellError::Win32(_) => ShellDiagnosticErrorClass::Windows,
    }
}

pub(crate) fn elapsed_ms_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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
static DIAGNOSTIC_SENDER: OnceLock<mpsc::SyncSender<ShellDiagnosticEvent>> = OnceLock::new();
#[cfg(all(windows, feature = "diagnostic-logs"))]
static DIAGNOSTIC_WRITER_INIT: OnceLock<Mutex<()>> = OnceLock::new();

/// Initialize the opt-in writer before an activation can report events.
///
/// This runs only in the diagnostic DLL. The production customer package
/// omits both its registry check and its file-I/O code.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn prepare_shell_diagnostics() {
    let _ = diagnostic_sender();
}

/// Queue a fixed normal lifecycle event without I/O on the COM callback.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn record_shell_event(
    component: ShellDiagnosticComponent,
    stage: ShellDiagnosticStage,
    outcome: ShellDiagnosticOutcome,
    adapter: ShellDiagnosticAdapter,
    elapsed_ms: u64,
) {
    record_event(ShellDiagnosticEvent::normal(
        ShellDiagnosticEventInput {
            component,
            stage,
            adapter,
            elapsed_ms,
        },
        outcome,
        current_diagnostic_process(),
    ));
}

/// Queue a classified `ShellError` without serializing the error itself.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn record_shell_error(
    error: &ShellError,
    component: ShellDiagnosticComponent,
    stage: ShellDiagnosticStage,
    adapter: ShellDiagnosticAdapter,
    elapsed_ms: u64,
) {
    record_shell_failure(
        component,
        stage,
        adapter,
        error_class_for_shell_error(error),
        elapsed_ms,
    );
}

/// Queue a fixed failure category when no Rust `ShellError` exists.
#[cfg(all(windows, feature = "diagnostic-logs"))]
pub(crate) fn record_shell_failure(
    component: ShellDiagnosticComponent,
    stage: ShellDiagnosticStage,
    adapter: ShellDiagnosticAdapter,
    error_class: ShellDiagnosticErrorClass,
    elapsed_ms: u64,
) {
    record_event(ShellDiagnosticEvent::failure(
        ShellDiagnosticEventInput {
            component,
            stage,
            adapter,
            elapsed_ms,
        },
        error_class,
        current_diagnostic_process(),
    ));
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn record_event(event: ShellDiagnosticEvent) {
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        let _ = sender.try_send(event);
    }
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn diagnostic_sender() -> Option<&'static mpsc::SyncSender<ShellDiagnosticEvent>> {
    let opt_in = diagnostic_switch_enabled(shell_diagnostic_switch_value())
        || diagnostic_switch_enabled(legacy_preview_diagnostic_switch_value());
    if !opt_in {
        return None;
    }
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        return Some(sender);
    }

    let _guard = diagnostic_writer_init_mutex().lock().ok()?;
    if let Some(sender) = DIAGNOSTIC_SENDER.get() {
        return Some(sender);
    }

    // The worker holds code in this COM DLL after the activation returns, so
    // pin the module before creating it. Failed initialization stays retryable.
    crate::com::own_pinned_dll_module().ok()?;
    let (sender, receiver) = mpsc::sync_channel(DIAGNOSTIC_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("occluview-shell-diagnostics".to_owned())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                write_shell_event(event);
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
fn write_shell_event(event: ShellDiagnosticEvent) {
    let Some(root) = local_app_data_low_directory() else {
        return;
    };
    let log = root
        .join("OccluView")
        .join("diagnostics")
        .join("shell-events.jsonl");
    let _ = append_diagnostic_line(&log, &event.json_line_inner());
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn current_diagnostic_process() -> ShellDiagnosticProcess {
    ShellDiagnosticProcess {
        timestamp_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        process_id: std::process::id(),
    }
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn shell_diagnostic_switch_value() -> Option<u32> {
    diagnostic_switch_value(DIAGNOSTIC_ENABLED_VALUE)
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn legacy_preview_diagnostic_switch_value() -> Option<u32> {
    diagnostic_switch_value(LEGACY_DIAGNOSTIC_ENABLED_VALUE)
}

#[cfg(all(windows, feature = "diagnostic-logs"))]
fn diagnostic_switch_value(name: PCWSTR) -> Option<u32> {
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
    let value = query_registry_dword(key, name);
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
fn local_app_data_low_directory() -> Option<std::path::PathBuf> {
    // SAFETY: Windows allocates a NUL-terminated result on success and the
    // current token is selected by passing None.
    let raw_directory =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppDataLow, KF_FLAG_DEFAULT, None).ok()? };
    // SAFETY: `raw_directory` is the NUL-terminated buffer returned above.
    let directory = unsafe { raw_directory.to_string().ok() };
    // SAFETY: SHGetKnownFolderPath allocates with the COM task allocator.
    unsafe { CoTaskMemFree(Some(raw_directory.0.cast())) };
    directory.map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timeout_event_is_fixed_field_and_has_no_error_text() {
        let error = ShellError::Render(RenderError::ReadbackTimeout {
            timeout: Duration::from_secs(2),
        });
        let event = ShellDiagnosticEvent::failure(
            ShellDiagnosticEventInput {
                component: ShellDiagnosticComponent::Preview,
                stage: ShellDiagnosticStage::Render,
                adapter: ShellDiagnosticAdapter::Fallback,
                elapsed_ms: 2000,
            },
            error_class_for_shell_error(&error),
            ShellDiagnosticProcess {
                timestamp_unix_ms: 1,
                process_id: 2,
            },
        );
        assert_eq!(
            event.json_line(),
            "{\"timestamp_unix_ms\":1,\"pid\":2,\"component\":\"preview\",\"process_role\":\"preview_host\",\"stage\":\"render\",\"outcome\":\"failed\",\"adapter\":\"fallback\",\"error_class\":\"deadline\",\"elapsed_ms\":2000}\n"
        );
    }

    #[test]
    fn bounded_diagnostic_log_preserves_existing_data_when_next_event_does_not_fit() {
        let root = std::env::temp_dir().join(format!(
            "occluview-shell-diagnostics-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&root).expect("temporary diagnostic directory");
        let log = root.join("shell-events.jsonl");
        let seed_len = usize::try_from(MAX_DIAGNOSTIC_LOG_BYTES)
            .expect("diagnostic log ceiling fits in usize")
            - 1;
        std::fs::write(&log, vec![b'x'; seed_len]).expect("seed bounded diagnostic log");
        let event = ShellDiagnosticEvent::normal(
            ShellDiagnosticEventInput {
                component: ShellDiagnosticComponent::Thumbnail,
                stage: ShellDiagnosticStage::ComReturn,
                adapter: ShellDiagnosticAdapter::NotObserved,
                elapsed_ms: 1,
            },
            ShellDiagnosticOutcome::Completed,
            ShellDiagnosticProcess {
                timestamp_unix_ms: 1,
                process_id: 2,
            },
        );

        append_diagnostic_line(&log, &event.json_line()).expect("full log is a no-op");

        assert_eq!(
            std::fs::metadata(&log)
                .expect("bounded diagnostic log metadata")
                .len(),
            MAX_DIAGNOSTIC_LOG_BYTES - 1
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostics_stay_disabled_without_an_explicit_dword_one() {
        assert!(!diagnostic_switch_enabled(None));
        assert!(!diagnostic_switch_enabled(Some(0)));
        assert!(diagnostic_switch_enabled(Some(1)));
    }
}

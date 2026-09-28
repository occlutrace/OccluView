use crate::{app, app_paths, live_viewport, single_instance};
use anyhow::Result;
use eframe::egui;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

mod graphics;
#[cfg(test)]
use eframe::egui_wgpu::wgpu;
#[cfg(all(test, target_os = "linux"))]
use graphics::root_viewport_builder;
#[cfg(test)]
use graphics::{
    adapter_device_score, device_limits_for_backend, format_supports_live_render,
    live_msaa_override, live_sample_count_for, select_live_sample_count,
    validate_graphics_environment_values, AdapterIdentity, GraphicsPreflight, LIVE_DEPTH_FORMAT,
    LIVE_SAFE_SAMPLE_COUNT,
};
use graphics::{
    graphics_diagnostics_details, native_options, preflight_graphics_devices,
    validate_graphics_environment,
};

/// How many recent log lines to keep for the crash report. A short window is
/// enough to see what led to a crash without bloating the report.
const CRASH_LOG_CAPACITY: usize = 50;
/// Keep the native startup breadcrumb file useful without allowing it to grow
/// forever across many launches. Entries contain no case paths or payloads.
const STARTUP_JOURNAL_CAPACITY: usize = 64;
const STARTUP_JOURNAL_MAX_BYTES: u64 = 64 * 1024;
const STARTUP_JOURNAL_FILE: &str = "startup-journal.log";
const SCENE_LOAD_LOG_ENV: &str = "OCCLUVIEW_SCENE_LOAD_LOG";
pub(crate) const MAX_RENDER_TEXTURE_DIMENSION: u32 = 8192;
/// Binary entry behind the library boundary: install the panic hook, then run
/// fallible startup and report failures instead of unwinding through `main`.
///
/// Never returns a `Result`: startup failures are written under `crashes/`,
/// offered to the operator through the platform's own dialog or notification
/// channel, and terminate with a failure status. Blocks running the event loop;
/// `--version`, `--diagnostics`, and `--shell-refresh` exit first.
pub fn main_entry() {
    append_startup_stage("entry");
    install_panic_hook();
    append_startup_stage("panic-hook-installed");
    if let Err(error) = real_main() {
        append_startup_stage("startup-failure");
        let details = format!("Startup failure\n\n{error:#}");
        let report_path = write_crash_report("startup-failure", &details);
        show_startup_fatal_message(report_path.as_deref(), &details);
        std::process::exit(1);
    }
    append_startup_stage("clean-exit");
}

fn initialize_logging() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Console output plus an in-memory ring buffer of the last few log lines,
    // so a crash report can show what the app was doing right before it died.
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(CrashLogLayer)
        .with(SceneLoadLogLayer::from_environment())
        .init();
}

fn real_main() -> Result<()> {
    initialize_logging();
    append_startup_stage("logging-ready");

    set_process_app_user_model_id();

    let args = crate::parse_args();
    if args.version {
        print_version_line();
        return Ok(());
    }
    if args.help {
        print_help();
        return Ok(());
    }
    if args.shell_refresh {
        #[cfg(windows)]
        {
            crate::shell_refresh::notify_shell_associations_changed();
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            return Err(anyhow::anyhow!(
                "--shell-refresh is only available on Windows"
            ));
        }
    }
    if args.diagnostics {
        append_startup_stage("diagnostics");
        // Diagnostics must remain useful even when the operator is debugging
        // the very environment override that normal startup rejects. Keep the
        // validation error in the report instead of failing before a report
        // can be written.
        let details = graphics_diagnostics_details(validate_graphics_environment());
        let report_path = require_report_path(write_report("graphics-diagnostics", &details))?;
        show_diagnostics_message(Some(&report_path));
        return Ok(());
    }
    validate_graphics_environment()?;

    // Shape, not identity. This line goes into the ring buffer that
    // `write_crash_report` dumps to disk, and a dental scan's path is the case
    // it belongs to. A crash report attached to a public issue must not carry
    // patient identifiers.
    tracing::info!(
        file_count = args.files.len(),
        formats = ?crate::file_extensions(&args.files),
        "OccluView starting"
    );
    let single_instance = single_instance::SingleInstance::acquire()?;
    if single_instance.is_secondary() {
        if !args.files.is_empty() {
            // As the short-lived second instance we inherit the launcher's
            // window-activation token (user-interaction provenance). Forward it
            // with the paths so the running instance can raise itself past the
            // desktop's focus-stealing prevention. See single_instance/activation.rs.
            let request = single_instance::OpenRequest {
                paths: args.files.clone(),
                activation_token: single_instance::capture_activation_token(),
            };
            single_instance::write_open_request(&request)?;
        }
        return Ok(());
    }

    // The primary instance also inherits the launcher's activation token; the
    // first startup load uses it to claim focus on X11 (see activation.rs).
    // Capture it before eframe/winit runs so nothing consumes the env first.
    let startup_activation_token = single_instance::capture_activation_token();

    append_startup_stage("graphics-preflight");
    let graphics_preflight = preflight_graphics_devices()?;
    append_startup_stage("graphics-preflight-ok");
    append_startup_stage("graphics-init");
    let live_sample_count = graphics_preflight.live_sample_count;
    let native_options = native_options(&graphics_preflight);

    // Finder hands a cold launch its documents before eframe calls the app
    // creator below, so the handlers go in as the application finishes
    // launching; the call in the creator only covers a failed observer.
    single_instance::install_open_files_handler_at_launch();

    eframe::run_native(
        "OccluView 3D Viewer",
        native_options,
        Box::new(move |cc| {
            single_instance::install_open_files_handler();
            append_startup_stage("window-ready");
            // Capture both raw handles now so the open-file handoff can use
            // the compositor's native activation protocol on Linux.
            let raise_target = single_instance::RaiseTarget::from_handles(cc, cc);
            // What actually reaches the offscreen fallback, traced rather than
            // assumed: this closure runs only after eframe has built a device,
            // because `WgpuWinitApp::init_run_state` calls `set_window(..)?`
            // before `painter.render_state()`. An adapter or device failure
            // aborts `run_native` and never gets here. That leaves one trigger,
            // `with_shared_device_sample_count` failing on a device that
            // already works -- a pipeline, shader or bind-group failure -- for
            // which the fallback's cure is a second wgpu device building the
            // same pipelines from the same shaders.
            //
            // The branch is not "no GPU" coverage; that case never arrives
            // here. It runs the same overlay body as the live path (see
            // `show_viewport_overlays`), so it stays current with the live
            // viewport.
            let live_viewport = cc.wgpu_render_state.as_ref().and_then(|state| {
                match live_viewport::LiveViewport::from_render_state(state, live_sample_count) {
                    Ok(viewport) => Some(viewport),
                    Err(e) => {
                        tracing::warn!(
                            error = ?e,
                            "live viewport unavailable; using offscreen fallback"
                        );
                        None
                    }
                }
            });
            Ok(Box::new(app::OccluViewApp::new(
                cc.egui_ctx.clone(),
                args.files.clone(),
                live_viewport,
                app::StartupHandles {
                    single_instance,
                    raise_target,
                    activation_token: startup_activation_token,
                },
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e:?}"))?;

    append_startup_stage("event-loop-exited");

    Ok(())
}

/// `--version` for scripts and packaging checks, printed before the
/// single-instance handshake so it never focuses a running viewer. On
/// Windows this is a GUI-subsystem binary: with no console attached the line
/// is discarded; it prints whenever stdout is piped or redirected, and always
/// on Linux. Attaching a parent console would drag in Win32 console plumbing
/// for one line.
#[allow(clippy::print_stdout)]
fn print_version_line() {
    println!("occluview {}", env!("CARGO_PKG_VERSION"));
}

#[allow(clippy::print_stdout)]
fn print_help() {
    println!(
        "OccluView {}\n\nUsage: occluview [OPTIONS] [FILE ...]\n\nOptions:\n  -h, --help       Show this help\n  -V, --version    Show the installed version\n      --diagnostics  Check graphics adapters without opening a window",
        env!("CARGO_PKG_VERSION")
    );
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        append_startup_stage("panic");
        let details = format_panic_details(panic_info);
        let report_path = write_crash_report("panic", &details);
        show_startup_fatal_message(report_path.as_deref(), &details);
    }));
}

fn format_panic_details(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = panic_info.payload().downcast_ref::<&str>().map_or_else(
        || {
            panic_info
                .payload()
                .downcast_ref::<String>()
                .map_or("non-string panic payload".to_string(), Clone::clone)
        },
        |message| (*message).to_string(),
    );
    let location = panic_info.location().map_or_else(
        || "unknown location".to_string(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    format!(
        "OccluView crash report\nversion: {}\nthread: {thread_name}\nlocation: {location}\n\n{payload}",
        env!("CARGO_PKG_VERSION")
    )
}

fn write_crash_report(kind: &str, details: &str) -> Option<PathBuf> {
    write_report(kind, details)
}

fn require_report_path(report_path: Option<PathBuf>) -> Result<PathBuf> {
    report_path.ok_or_else(|| anyhow::anyhow!("could not write the graphics diagnostics report"))
}

/// Write a diagnostic or crash report without overwriting a report created by
/// another failure in the same clock tick. `create_new` also protects a report
/// when two processes fail during the same nanosecond on a fast filesystem.
fn write_report(kind: &str, details: &str) -> Option<PathBuf> {
    let stamp_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let pid = std::process::id();
    let report = format!(
        "{details}\n{}\n{}\nBuild: {}\n",
        recent_startup_stages(),
        recent_log_lines(),
        env!("CARGO_PKG_VERSION"),
    );
    for dir in crash_report_dirs() {
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        for attempt in 0..16 {
            let path = dir.join(report_file_name(kind, stamp_nanos, pid, attempt));
            let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => break,
            };
            if file.write_all(report.as_bytes()).is_ok() {
                let _ = file.sync_all();
                return Some(path);
            }
            let _ = std::fs::remove_file(path);
            break;
        }
    }
    None
}

fn report_file_name(kind: &str, stamp_nanos: u128, pid: u32, attempt: u32) -> String {
    let safe_kind: String = kind
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect();
    let safe_kind = if safe_kind.is_empty() {
        "report"
    } else {
        safe_kind.as_str()
    };
    format!("occluview-{safe_kind}-{stamp_nanos}-{pid}-{attempt}.txt")
}

fn startup_journal_path() -> Option<PathBuf> {
    app_paths::app_state_dir().map(|base| base.join(STARTUP_JOURNAL_FILE))
}

fn startup_journal_paths() -> Vec<PathBuf> {
    startup_journal_paths_from(
        startup_journal_path(),
        std::env::temp_dir()
            .join("OccluView")
            .join(STARTUP_JOURNAL_FILE),
    )
}

fn startup_journal_paths_from(primary: Option<PathBuf>, fallback: PathBuf) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(2);
    if let Some(primary) = primary {
        paths.push(primary);
    }
    if !paths.contains(&fallback) {
        paths.push(fallback);
    }
    paths
}

fn startup_stage_line(stage: &str, stamp_nanos: u128, pid: u32) -> String {
    let safe_stage: String = stage
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect();
    let safe_stage = if safe_stage.is_empty() {
        "unknown"
    } else {
        safe_stage.as_str()
    };
    format!("{stamp_nanos} pid={pid} stage={safe_stage}")
}

/// Leave a tiny persistent breadcrumb at the last startup boundary. It is
/// metadata only: native driver crashes can happen before Rust reaches the
/// panic hook, but the next report can still say whether the process reached
/// logging, graphics initialization, or the window callback.
fn append_startup_stage(stage: &str) {
    let line = startup_stage_line(stage, unix_timestamp_nanos(), std::process::id());
    for path in startup_journal_paths() {
        let Some(parent) = path.parent() else {
            continue;
        };
        if std::fs::create_dir_all(parent).is_err() {
            continue;
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            if writeln!(file, "{line}").is_ok() {
                trim_startup_journal(&path);
                return;
            }
        }
    }
}

fn trim_startup_journal(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() <= STARTUP_JOURNAL_MAX_BYTES {
        return;
    }
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = contents.lines().collect();
    let first = lines.len().saturating_sub(STARTUP_JOURNAL_CAPACITY);
    let kept = lines[first..].join("\n");
    let kept = if kept.is_empty() {
        kept
    } else {
        format!("{kept}\n")
    };
    let _ = std::fs::write(path, kept);
}

fn recent_startup_stages() -> String {
    for path in startup_journal_paths() {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut lines: Vec<&str> = contents
            .lines()
            .rev()
            .take(STARTUP_JOURNAL_CAPACITY)
            .collect();
        if lines.is_empty() {
            continue;
        }
        lines.reverse();
        return format!(
            "\nRecent startup stages (oldest first):\n{}\n",
            lines.join("\n")
        );
    }
    String::new()
}

fn unix_timestamp_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

/// Shared ring buffer of the most recent formatted log lines.
fn crash_log() -> &'static Mutex<VecDeque<String>> {
    static LOG: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(CRASH_LOG_CAPACITY)))
}

/// Seconds since process start, for relative timing in the crash log.
fn process_uptime_secs() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// Append `line`, evicting the oldest entry once the capacity is reached.
fn evict_and_push(ring: &mut VecDeque<String>, line: String) {
    if ring.len() >= CRASH_LOG_CAPACITY {
        ring.pop_front();
    }
    ring.push_back(line);
}

fn push_crash_log_line(line: String) {
    if let Ok(mut ring) = crash_log().lock() {
        evict_and_push(&mut ring, line);
    }
}

/// Render the ring buffer for inclusion in a crash report. Uses `try_lock` so a
/// panic that fired while the ring lock was held (same thread) cannot deadlock
/// the crash-report writer — a missing tail is acceptable; a hang is not.
fn recent_log_lines() -> String {
    match crash_log().try_lock() {
        Ok(ring) if !ring.is_empty() => {
            let mut out = String::from("\nRecent log (oldest first):\n");
            for line in ring.iter() {
                out.push_str(line);
                out.push('\n');
            }
            out
        }
        _ => String::new(),
    }
}

/// A tracing layer that captures a compact line per event into [`crash_log`].
struct CrashLogLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CrashLogLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut visitor = CrashLogVisitor::default();
        event.record(&mut visitor);
        push_crash_log_line(format!(
            "[{:9.3}s] {:>5} {}:{}",
            process_uptime_secs(),
            meta.level(),
            meta.target(),
            visitor.text
        ));
    }
}

/// Collects an event's message + fields into a single flat string.
#[derive(Default)]
struct CrashLogVisitor {
    text: String,
    message: Option<String>,
    path_count: Option<String>,
}

impl tracing::field::Visit for CrashLogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let rendered = format!("{value:?}");
            self.message = Some(rendered.trim_matches('"').to_owned());
            let _ = write!(self.text, " {rendered}");
        } else {
            if field.name() == "path_count" {
                self.path_count = Some(format!("{value:?}"));
            }
            let _ = write!(self.text, " {}={value:?}", field.name());
        }
    }
}

/// An opt-in file sink records the path-free completion marker and input count.
struct SceneLoadLogLayer {
    file: Option<Mutex<std::fs::File>>,
}

impl SceneLoadLogLayer {
    fn from_environment() -> Self {
        let file = std::env::var_os(SCENE_LOAD_LOG_ENV).and_then(|path| {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .ok()
        });
        Self {
            file: file.map(Mutex::new),
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SceneLoadLogLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        let mut visitor = CrashLogVisitor::default();
        event.record(&mut visitor);
        if visitor.message.as_deref() != Some("scene load completed") {
            return;
        }
        let Some(path_count) = visitor.path_count else {
            return;
        };
        let Ok(mut guard) = file.lock() else {
            return;
        };
        let meta = event.metadata();
        let line = format!(
            "[{:9.3}s] {:>5} {}: scene load completed path_count={path_count}",
            process_uptime_secs(),
            meta.level(),
            meta.target()
        );
        let _ = writeln!(&mut *guard, "{line}");
    }
}

fn crash_report_dir() -> Option<PathBuf> {
    app_paths::app_state_dir().map(|base| base.join("crashes"))
}

fn crash_report_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Some(primary) = crash_report_dir() {
        dirs.push(primary);
    }
    let fallback = std::env::temp_dir().join("OccluView").join("crashes");
    if !dirs.contains(&fallback) {
        dirs.push(fallback);
    }
    dirs
}

fn show_startup_fatal_message(report_path: Option<&Path>, details: &str) {
    #[cfg(windows)]
    {
        show_startup_fatal_message_box(report_path, details);
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        // A desktop launch has no console. Give the operator the full path,
        // not just a filename they cannot locate, and put the same actionable
        // value in the next startup breadcrumb if every dialog channel fails.
        // The no-file state stays the machine-readable `none` sentinel, as in
        // `show_diagnostics_message`.
        let report =
            report_path.map_or_else(|| "none".to_owned(), |path| path.display().to_string());
        tracing::error!(report, details, "OccluView could not continue");
        notify_desktop("OccluView could not start", &report, "critical");
    }

    #[cfg(target_os = "macos")]
    {
        let report =
            report_path.map_or_else(|| "none".to_owned(), |path| path.display().to_string());
        tracing::error!(report, details, "OccluView could not continue");
        notify_macos_dialog("OccluView could not start", &report);
    }
}

fn show_diagnostics_message(report_path: Option<&Path>) {
    // Keep the no-file state machine-readable. This sentinel is also handled
    // by the Windows diagnostics dialog below; a prose fallback here would
    // bypass the presentation-sink contract before the locale manager exists.
    let report = report_path.map_or_else(|| "none".to_owned(), |path| path.display().to_string());

    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

        let message = if report == "none" {
            "No diagnostic report could be written.".to_string()
        } else {
            format!("Graphics diagnostics were saved as:\n{report}")
        };
        let title = HSTRING::from("OccluView graphics diagnostics");
        let message = HSTRING::from(message);
        unsafe {
            MessageBoxW(None, &message, &title, MB_OK | MB_ICONINFORMATION);
        }
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        tracing::info!(report, "graphics diagnostics written");
        notify_desktop("OccluView graphics diagnostics", &report, "normal");
    }

    #[cfg(target_os = "macos")]
    {
        tracing::info!(report, "graphics diagnostics written");
        notify_macos_dialog("OccluView graphics diagnostics", &report);
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn notify_desktop(title: &str, report: &str, urgency: &str) -> &'static str {
    let body = format!("Diagnostic report: {report}");
    for channel in NOTIFICATION_CHANNELS {
        let Some((program, args)) = notification_command(channel, title, &body, urgency) else {
            continue;
        };
        match run_notification(&program, &args) {
            Ok(true) => {
                tracing::info!(channel, "startup notice shown to the operator");
                return channel;
            }
            Ok(false) => {
                // The program ran and refused the message: `notify-send`
                // exits non-zero when no notification daemon answers, which
                // is the common case on a bare session. The list exists so the
                // next channel is tried.
                tracing::warn!(channel, "the desktop did not accept the notice");
            }
            Err(error) => {
                tracing::warn!(channel, %error, "the notice program could not be run");
            }
        }
    }
    // Best effort only: the report and the non-zero exit status remain the
    // authoritative support signals when no desktop notification service is
    // installed or the process is launched outside a graphical session. Saying
    // so in the journal is what stops a silent exit from reading as a crash.
    tracing::error!(
        "no desktop notification channel delivered the notice; the report path is the only operator-visible signal"
    );
    "none"
}

#[cfg(target_os = "macos")]
const MACOS_DIALOG_SCRIPT: &str = r#"on run argv
    display dialog (item 2 of argv) with title (item 1 of argv) buttons {"OK"} default button 1
end run"#;

/// Build a native `AppleScript` dialog command without interpolating operator or
/// filesystem content into executable script source.
#[cfg(target_os = "macos")]
fn macos_dialog_command(title: &str, body: &str) -> Vec<String> {
    vec![
        "-e".to_string(),
        MACOS_DIALOG_SCRIPT.to_string(),
        "--".to_string(),
        title.to_string(),
        body.to_string(),
    ]
}

#[cfg(target_os = "macos")]
fn notify_macos_dialog(title: &str, report: &str) {
    let body = format!("Diagnostic report: {report}");
    let args = macos_dialog_command(title, &body);
    match run_notification("osascript", &args) {
        Ok(true) => tracing::info!(
            channel = "osascript",
            "startup notice shown to the operator"
        ),
        Ok(false) => tracing::warn!(channel = "osascript", "macOS did not accept the notice"),
        Err(error) => {
            tracing::warn!(channel = "osascript", %error, "native macOS dialog could not be run");
        }
    }
}

/// Run one notification program and report whether it delivered the message.
///
/// Waiting for the exit status is what separates "the message was shown" from
/// "a binary with that name exists": `notify-send` succeeds only when a
/// notification daemon answered it.
///
/// The wait is bounded. `zenity`, `kdialog`, and `xmessage` are modal dialogs
/// that only return when dismissed, which is right for the message but wrong for
/// the process: this runs on the fatal-startup path, where `main_entry` still
/// owes the operator a non-zero exit status, and an unattended `xmessage` (a CI
/// host, a kiosk, an unwatched `.desktop` launch) would otherwise keep a dead
/// startup alive forever with no window.
#[cfg(not(windows))]
fn run_notification(program: &str, args: &[String]) -> std::io::Result<bool> {
    use std::time::Instant;

    let mut child = std::process::Command::new(program).args(args).spawn()?;
    let deadline = Instant::now() + NOTIFICATION_DISMISS_WAIT;
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status.success()),
            None if Instant::now() >= deadline => {
                // The dialog is on screen and the operator can still read it;
                // the process has said everything it can and must not block the
                // exit status on a click that may never come.
                tracing::warn!(
                    program,
                    "the notice is still open; leaving it on screen and continuing to exit"
                );
                return Ok(true);
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

/// How long a fatal-startup notice may hold the process open.
///
/// Long enough for a `notify-send` round trip or a dialog that appears at once;
/// short enough that a hung startup is not mistaken for a slow one.
#[cfg(not(windows))]
const NOTIFICATION_DISMISS_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The notification channels tried, in the order they are attempted.
///
/// `notify-send` is the freedesktop one; `zenity` and `kdialog` are what a
/// minimal desktop image tends to carry when no notification daemon answers;
/// `xmessage` is the last X11-only modal fallback.
#[cfg(all(not(windows), not(target_os = "macos")))]
const NOTIFICATION_CHANNELS: [&str; 4] = ["notify-send", "zenity", "kdialog", "xmessage"];

/// The command line for one notification channel.
///
/// The body carries the full report path and the title names the product, so a
/// desktop dialog is actionable without the console a `.desktop` launch never
/// has. Each channel's exit status is read by [`run_notification`], so a
/// channel with no service behind it does not consume the message.
#[cfg(all(not(windows), not(target_os = "macos")))]
fn notification_command(
    channel: &str,
    title: &str,
    body: &str,
    urgency: &str,
) -> Option<(String, Vec<String>)> {
    match channel {
        "notify-send" => Some((
            "notify-send".to_string(),
            vec![
                format!("--urgency={urgency}"),
                title.to_string(),
                body.to_string(),
            ],
        )),
        "zenity" => Some((
            "zenity".to_string(),
            vec![
                "--error".to_string(),
                "--title".to_string(),
                title.to_string(),
                "--text".to_string(),
                body.to_string(),
            ],
        )),
        "kdialog" => Some((
            "kdialog".to_string(),
            vec![
                "--error".to_string(),
                body.to_string(),
                "--title".to_string(),
                title.to_string(),
            ],
        )),
        "xmessage" => Some((
            "xmessage".to_string(),
            vec![
                "-center".to_string(),
                "-title".to_string(),
                title.to_string(),
                body.to_string(),
            ],
        )),
        _ => None,
    }
}

#[cfg(windows)]
fn show_startup_fatal_message_box(report_path: Option<&Path>, details: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let message = if let Some(path) = report_path {
        format!(
            "OccluView could not continue.\n\nA crash report was saved to:\n{}\n\n{}",
            path.display(),
            details
        )
    } else {
        format!("OccluView could not continue.\n\n{details}")
    };
    let title = HSTRING::from("OccluView 3D Viewer");
    let message = HSTRING::from(message);
    unsafe {
        MessageBoxW(None, &message, &title, MB_OK | MB_ICONERROR);
    }
}

fn load_window_icon() -> Arc<egui::IconData> {
    let bytes = include_bytes!("../assets/windows/occluview.png");
    let image = match image::load_from_memory(bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            tracing::warn!(?error, "embedded OccluView PNG icon failed to decode");
            return Arc::new(egui::IconData {
                rgba: vec![0, 0, 0, 0],
                width: 1,
                height: 1,
            });
        }
    };
    let (width, height) = image.dimensions();
    Arc::new(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

#[cfg(windows)]
fn set_process_app_user_model_id() {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    let app_id = HSTRING::from(crate::APP_USER_MODEL_ID);
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(&app_id) } {
        tracing::warn!(?error, "failed to set process AppUserModelID");
    }
}

#[cfg(not(windows))]
fn set_process_app_user_model_id() {}

#[cfg(test)]
#[path = "app_bootstrap_tests.rs"]
mod tests;

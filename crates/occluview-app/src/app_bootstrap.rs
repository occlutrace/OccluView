use crate::{app, app_paths, live_viewport, single_instance, LIVE_VIEWPORT_SAMPLE_COUNT};
use anyhow::Result;
use eframe::egui;
use eframe::egui_wgpu::wgpu;
use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// How many recent log lines to keep for the crash report. A short window is
/// enough to see what led to a crash without bloating the report.
const CRASH_LOG_CAPACITY: usize = 50;
/// Keep the native startup breadcrumb file useful without allowing it to grow
/// forever across many launches. Entries contain no case paths or payloads.
const STARTUP_JOURNAL_CAPACITY: usize = 64;
const STARTUP_JOURNAL_MAX_BYTES: u64 = 64 * 1024;
const STARTUP_JOURNAL_FILE: &str = "startup-journal.log";
const MAX_RENDER_TEXTURE_DIMENSION: u32 = 8192;

/// Binary entry behind the library boundary: install the panic hook, then run
/// fallible startup and report failures instead of unwinding through `main`.
///
/// Never returns a `Result`: startup failures are written under `crashes/`,
/// shown to the operator, and terminate with a failure status. Blocks running
/// the event loop; `--version`, `--diagnostics`, and `--shell-refresh` exit
/// first.
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

fn real_main() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Console output PLUS an in-memory ring buffer of the last few log lines,
    // so a crash report can show what the app was doing right before it died.
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(CrashLogLayer)
        .init();
    append_startup_stage("logging-ready");

    set_process_app_user_model_id();

    let args = crate::parse_args();
    if args.version {
        print_version_line();
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
        let details = graphics_diagnostics_report();
        let report_path = write_report("graphics-diagnostics", &details);
        show_diagnostics_message(report_path.as_deref());
        return Ok(());
    }

    // Shape, not identity. This line goes into the ring buffer that
    // `write_crash_report` dumps to disk, and a dental scan's path is the case
    // it belongs to. Someone asked to attach a crash report to a public issue
    // should not be attaching patient identifiers with it.
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

    // The PRIMARY instance also inherits the launcher's activation token; the
    // first startup load uses it to claim focus on X11 (see activation.rs).
    // Capture it before eframe/winit runs so nothing consumes the env first.
    let startup_activation_token = single_instance::capture_activation_token();

    append_startup_stage("graphics-init");
    let native_options = native_options();

    eframe::run_native(
        "OccluView 3D Viewer",
        native_options,
        Box::new(move |cc| {
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
            // The branch stays, but it is not "this machine has no GPU"
            // coverage; that case never arrives here. The same overlay body as
            // the live path runs it (see `show_viewport_overlays`), which is
            // what stops it rotting for the operators it does serve.
            let live_viewport = cc.wgpu_render_state.as_ref().and_then(|state| {
                match live_viewport::LiveViewport::from_render_state(state) {
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

fn native_options() -> eframe::NativeOptions {
    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetup::without_display_handle();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut wgpu_setup {
        // eframe creates the adapter/device before it calls our app creator.
        // Give it a descriptor derived from the selected adapter so a legacy
        // GL implementation is not rejected for a texture limit it cannot
        // support.
        create_new.device_descriptor = Arc::new(device_descriptor_for_adapter);
    }

    eframe::NativeOptions {
        viewport: root_viewport_builder(),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 24,
        stencil_buffer: 8,
        multisampling: LIVE_VIEWPORT_SAMPLE_COUNT,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // Keep vsync, but do not queue stale camera frames ahead of what
            // the operator is currently doing with the mouse.
            surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            wgpu_setup,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build the smallest valid device request for the selected adapter while
/// keeping the normal egui/wgpu defaults on modern hardware. The old eframe
/// default hard-coded an 8192 2D texture limit even for a GL adapter whose
/// advertised limit could be lower; wgpu rejects such a request before the
/// application creator runs.
fn device_limits_for_backend(backend: wgpu::Backend, supported: &wgpu::Limits) -> wgpu::Limits {
    let base_limits = if backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    wgpu::Limits {
        max_texture_dimension_2d: MAX_RENDER_TEXTURE_DIMENSION,
        ..base_limits
    }
    .or_worse_values_from(supported)
}

fn device_descriptor_for_adapter(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let info = adapter.get_info();
    let supported = adapter.limits();
    let required_limits = device_limits_for_backend(info.backend, &supported);
    tracing::info!(
        backend = ?info.backend,
        device_type = ?info.device_type,
        max_texture_dimension_2d = supported.max_texture_dimension_2d,
        requested_max_texture_dimension_2d = required_limits.max_texture_dimension_2d,
        "wgpu adapter selected"
    );
    wgpu::DeviceDescriptor {
        label: Some("occluview wgpu device"),
        required_limits,
        ..Default::default()
    }
}

fn root_viewport_builder() -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        .with_inner_size([1024.0, 768.0])
        .with_title("OccluView 3D Viewer")
        .with_icon(load_window_icon());

    #[cfg(target_os = "linux")]
    {
        builder.with_app_id(crate::LINUX_DESKTOP_APP_ID)
    }

    #[cfg(not(target_os = "linux"))]
    {
        builder
    }
}

/// `--version` for scripts and packaging checks, printed before the
/// single-instance handshake so it never focuses a running viewer. On
/// Windows this is a GUI-subsystem binary: with no console attached the
/// line goes to a null stdout and the process simply exits cleanly; it
/// prints whenever stdout is piped or redirected, and always on Linux.
/// Attaching a parent console would drag in Win32 console plumbing for one
/// line.
#[allow(clippy::print_stdout)]
fn print_version_line() {
    println!("occluview {}", env!("CARGO_PKG_VERSION"));
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

/// Write a diagnostic or crash report without overwriting a report created by
/// another failure in the same clock tick. `create_new` also protects a report
/// when two processes fail during the same nanosecond on a fast filesystem.
fn write_report(kind: &str, details: &str) -> Option<PathBuf> {
    let dir = crash_report_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
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
    for attempt in 0..16 {
        let path = dir.join(report_file_name(kind, stamp_nanos, pid, attempt));
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        };
        if file.write_all(report.as_bytes()).is_ok() {
            return Some(path);
        }
        let _ = std::fs::remove_file(path);
        return None;
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
/// deliberately metadata-only: native driver crashes can happen before Rust
/// reaches the panic hook, but the next report can still say whether the
/// process reached logging, graphics initialization, or the window callback.
fn append_startup_stage(stage: &str) {
    let Some(path) = startup_journal_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let line = startup_stage_line(stage, unix_timestamp_nanos(), std::process::id());
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
    trim_startup_journal(&path);
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
    let Some(path) = startup_journal_path() else {
        return String::new();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut lines: Vec<&str> = contents
        .lines()
        .rev()
        .take(STARTUP_JOURNAL_CAPACITY)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    lines.reverse();
    format!(
        "\nRecent startup stages (oldest first):\n{}\n",
        lines.join("\n")
    )
}

fn unix_timestamp_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn graphics_diagnostics_report() -> String {
    use std::fmt::Write as _;

    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    let backends = descriptor.backends;
    let instance = wgpu::Instance::new(descriptor);
    let adapters = pollster::block_on(instance.enumerate_adapters(backends));
    let mut report = String::new();
    let _ = writeln!(report, "OccluView graphics diagnostics");
    let _ = writeln!(report, "version: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(report, "backends: {backends:?}");
    let _ = writeln!(
        report,
        "WGPU_BACKEND: {}",
        std::env::var("WGPU_BACKEND").unwrap_or_else(|_| "<unset>".to_string())
    );
    let _ = writeln!(
        report,
        "WGPU_POWER_PREF: {}",
        std::env::var("WGPU_POWER_PREF").unwrap_or_else(|_| "<unset>".to_string())
    );
    let _ = writeln!(report, "DISPLAY: {}", environment_state("DISPLAY"));
    let _ = writeln!(
        report,
        "WAYLAND_DISPLAY: {}",
        environment_state("WAYLAND_DISPLAY")
    );
    let _ = writeln!(report, "adapters: {}", adapters.len());

    if adapters.is_empty() {
        let _ = writeln!(report, "adapter_result: none");
        return report;
    }

    for (index, adapter) in adapters.iter().enumerate() {
        let info = adapter.get_info();
        let supported = adapter.limits();
        let requested = device_limits_for_backend(info.backend, &supported);
        let _ = writeln!(report, "adapter[{index}]:");
        let _ = writeln!(report, "  name: {}", info.name);
        let _ = writeln!(report, "  backend: {}", info.backend);
        let _ = writeln!(report, "  device_type: {:?}", info.device_type);
        let _ = writeln!(report, "  driver: {}", info.driver);
        let _ = writeln!(report, "  driver_info: {}", info.driver_info);
        let _ = writeln!(
            report,
            "  max_texture_dimension_2d: supported={} requested={}",
            supported.max_texture_dimension_2d, requested.max_texture_dimension_2d
        );
        let device_status = match pollster::block_on(
            adapter.request_device(&device_descriptor_for_adapter(adapter)),
        ) {
            Ok((_device, _queue)) => "ok".to_string(),
            Err(error) => format!("error: {error}"),
        };
        let _ = writeln!(report, "  device_request: {device_status}");
    }
    report
}

fn environment_state(name: &str) -> &'static str {
    if std::env::var_os(name).is_some() {
        "set"
    } else {
        "unset"
    }
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
}

impl tracing::field::Visit for CrashLogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if field.name() == "message" {
            let _ = write!(self.text, " {value:?}");
        } else {
            let _ = write!(self.text, " {}={value:?}", field.name());
        }
    }
}

fn crash_report_dir() -> Option<PathBuf> {
    app_paths::app_state_dir()
        .map(|base| base.join("crashes"))
        .or_else(|| std::env::temp_dir().canonicalize().ok())
}

fn show_startup_fatal_message(report_path: Option<&Path>, details: &str) {
    #[cfg(windows)]
    {
        show_startup_fatal_message_box(report_path, details);
    }

    #[cfg(not(windows))]
    {
        // Name, not path, as above -- and this line goes into the ring the
        // NEXT report carries.
        let report = report_path
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("none");
        tracing::error!(report, details, "OccluView could not continue");
        notify_desktop("OccluView could not start", report, "critical");
    }
}

fn show_diagnostics_message(report_path: Option<&Path>) {
    let report = report_path
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("none");

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

    #[cfg(not(windows))]
    {
        tracing::info!(report, "graphics diagnostics written");
        notify_desktop("OccluView graphics diagnostics", report, "normal");
    }
}

#[cfg(not(windows))]
fn notify_desktop(title: &str, report: &str, urgency: &str) {
    // Best effort only: the report and non-zero exit status remain the
    // authoritative support signals when no desktop notification service is
    // installed or the process is launched outside a graphical session.
    let body = format!("Diagnostic report: {report}");
    let _ = std::process::Command::new("notify-send")
        .arg(format!("--urgency={urgency}"))
        .arg(title)
        .arg(body)
        .spawn();
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
mod tests {
    use super::*;

    #[test]
    fn native_options_use_the_low_latency_surface_contract() {
        let options = native_options();

        assert_eq!(
            options.wgpu_options.surface,
            eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
        );
    }

    #[test]
    fn graphics_limits_never_exceed_a_weak_adapter() {
        let supported = wgpu::Limits {
            max_texture_dimension_2d: 4096,
            ..wgpu::Limits::default()
        };

        let requested = device_limits_for_backend(wgpu::Backend::Gl, &supported);

        assert_eq!(requested.max_texture_dimension_2d, 4096);
    }

    #[test]
    fn graphics_limits_keep_the_requested_budget_when_hardware_supports_it() {
        let supported = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            ..wgpu::Limits::default()
        };

        let requested = device_limits_for_backend(wgpu::Backend::Vulkan, &supported);

        assert_eq!(requested.max_texture_dimension_2d, 8192);
    }

    #[test]
    fn compatible_startup_profile_uses_single_sample_rendering() {
        assert_eq!(LIVE_VIEWPORT_SAMPLE_COUNT, 1);
    }

    #[test]
    fn report_names_are_unique_even_when_failures_share_a_clock_tick() {
        let first = report_file_name("startup-failure", 42, 7, 0);
        let second = report_file_name("startup-failure", 42, 7, 1);
        assert_ne!(first, second);
        assert!(first.ends_with("-0.txt"));
        assert!(second.ends_with("-1.txt"));
    }

    #[test]
    fn startup_stage_lines_contain_only_diagnostic_metadata() {
        let line = startup_stage_line("graphics-init", 42, 7);
        assert_eq!(line, "42 pid=7 stage=graphics-init");
        assert!(!line.contains('/'));
    }

    #[test]
    fn crash_log_ring_keeps_only_the_most_recent_lines() {
        let mut ring = VecDeque::new();
        for i in 0..(CRASH_LOG_CAPACITY + 5) {
            evict_and_push(&mut ring, format!("line {i}"));
        }
        assert_eq!(
            ring.len(),
            CRASH_LOG_CAPACITY,
            "ring is bounded to its capacity"
        );
        assert_eq!(
            ring.front().map(String::as_str),
            Some("line 5"),
            "the five oldest lines are evicted"
        );
        assert_eq!(
            ring.back().map(String::as_str),
            Some(&format!("line {}", CRASH_LOG_CAPACITY + 4)[..]),
            "the newest line is retained"
        );
    }

    #[test]
    fn crash_report_includes_recent_log_lines() {
        push_crash_log_line("[    0.001s]  INFO occluview: booting".to_string());
        let report_tail = recent_log_lines();
        assert!(
            report_tail.contains("Recent log"),
            "crash report embeds the recent-log section"
        );
        assert!(
            report_tail.contains("booting"),
            "captured log lines reach the crash report"
        );
    }
}

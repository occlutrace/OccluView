use crate::{app, app_paths, live_viewport, single_instance, LIVE_VIEWPORT_SAMPLE_COUNT};
use anyhow::Result;
use eframe::egui;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// How many recent log lines to keep for the crash report. A short window is
/// enough to see what led to a crash without bloating the report.
const CRASH_LOG_CAPACITY: usize = 50;

pub(crate) fn main_entry() {
    install_panic_hook();
    if let Err(error) = real_main() {
        let details = format!("Startup failure\n\n{error:#}");
        let report_path = write_crash_report("startup-failure", &details);
        show_startup_fatal_message(report_path.as_deref(), &details);
    }
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

    set_process_app_user_model_id();

    let args = app::parse_args();
    if args.shell_refresh {
        #[cfg(windows)]
        {
            occluview_shell::notify_shell_associations_changed();
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            return Err(anyhow::anyhow!(
                "--shell-refresh is only available on Windows"
            ));
        }
    }

    tracing::info!(files = ?args.files, "OccluView starting");
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

    let native_options = eframe::NativeOptions {
        viewport: root_viewport_builder(),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 24,
        stencil_buffer: 8,
        multisampling: LIVE_VIEWPORT_SAMPLE_COUNT,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // Keep vsync, but do not queue stale camera frames ahead of what
            // the operator is currently doing with the mouse.
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "OccluView 3D Viewer",
        native_options,
        Box::new(move |cc| {
            // Capture both raw handles now so the open-file handoff can use
            // the compositor's native activation protocol on Linux.
            let raise_target = single_instance::RaiseTarget::from_handles(cc, cc);
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

    Ok(())
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

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
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
    let dir = crash_report_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let path = dir.join(format!("occluview-{kind}-{stamp}.txt"));
    let report = format!(
        "{details}\n{}\nBuild: {}\nReport path: {}\n",
        recent_log_lines(),
        env!("CARGO_PKG_VERSION"),
        path.display()
    );
    std::fs::write(&path, report).ok()?;
    Some(path)
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
        tracing::error!(?report_path, details, "OccluView could not continue");
    }
}

#[cfg(windows)]
fn show_startup_fatal_message_box(report_path: Option<&Path>, details: &str) {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
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
        MessageBoxW(HWND::default(), &message, &title, MB_OK | MB_ICONERROR);
    }
}

fn load_window_icon() -> std::sync::Arc<egui::IconData> {
    let bytes = include_bytes!("../assets/windows/occluview.png");
    let image = match image::load_from_memory(bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            tracing::warn!(?error, "embedded OccluView PNG icon failed to decode");
            return std::sync::Arc::new(egui::IconData {
                rgba: vec![0, 0, 0, 0],
                width: 1,
                height: 1,
            });
        }
    };
    let (width, height) = image.dimensions();
    std::sync::Arc::new(egui::IconData {
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

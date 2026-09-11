use crate::{app, app_paths, live_viewport, single_instance};
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
const LIVE_MSAA_SAMPLE_COUNT: u16 = 4;
const LIVE_SAFE_SAMPLE_COUNT: u16 = 1;
/// The depth/stencil format every live-pass pipeline declares.
///
/// Taken from the render crate that builds those pipelines, so the window's
/// request and the pipelines cannot drift apart: a mismatch is a validation
/// error on every draw, not a graceful degradation.
const LIVE_DEPTH_FORMAT: wgpu::TextureFormat = occluview_render::live_depth_format();
const LIVE_SURFACE_FORMATS: [wgpu::TextureFormat; 4] = [
    wgpu::TextureFormat::Rgba8Unorm,
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8UnormSrgb,
    wgpu::TextureFormat::Bgra8UnormSrgb,
];

/// Graphics facts established before eframe creates the window.
///
/// `live_sample_count` is part of this value instead of a process-wide
/// constant: eframe and the custom viewport must agree with the adapter that
/// will actually render this startup.
#[derive(Clone, Debug)]
struct GraphicsPreflight {
    adapters: Vec<AdapterIdentity>,
    live_sample_count: u16,
}

impl Default for GraphicsPreflight {
    fn default() -> Self {
        Self {
            adapters: Vec::new(),
            live_sample_count: LIVE_SAFE_SAMPLE_COUNT,
        }
    }
}

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

    append_startup_stage("graphics-preflight");
    let graphics_preflight = preflight_graphics_devices()?;
    append_startup_stage("graphics-preflight-ok");
    append_startup_stage("graphics-init");
    let live_sample_count = graphics_preflight.live_sample_count;
    let native_options = native_options(&graphics_preflight);

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

fn native_options(preflight: &GraphicsPreflight) -> eframe::NativeOptions {
    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetup::without_display_handle();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut wgpu_setup {
        // eframe creates the adapter/device before it calls our app creator.
        // Give it a descriptor derived from the selected adapter so a legacy
        // GL implementation is not rejected for a texture limit it cannot
        // support.
        create_new.device_descriptor = Arc::new(device_descriptor_for_adapter);
        let power_preference = create_new.power_preference;
        let preflight = preflight.clone();
        create_new.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            select_native_adapter(adapters, surface, power_preference, &preflight)
        }));
    }

    eframe::NativeOptions {
        viewport: root_viewport_builder(),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 24,
        stencil_buffer: 8,
        multisampling: preflight.live_sample_count,
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

fn native_graphics_profile() -> (wgpu::InstanceDescriptor, wgpu::PowerPreference) {
    let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) =
        eframe::egui_wgpu::WgpuSetup::without_display_handle()
    else {
        unreachable!("native graphics setup must create its own wgpu instance");
    };
    (create_new.instance_descriptor, create_new.power_preference)
}

/// Select the best adapter that can actually present to the desktop surface.
/// The stock eframe selector stops at its first request-adapter result; a
/// hybrid system can enumerate a software or headless adapter before the
/// usable integrated GPU. Filtering surface capabilities and ranking the
/// remaining adapters keeps that choice deterministic while preserving the
/// configured power preference.
fn select_native_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
    power_preference: wgpu::PowerPreference,
    preflight: &GraphicsPreflight,
) -> Result<wgpu::Adapter, String> {
    let mut best: Option<(i32, usize)> = None;
    for (index, adapter) in adapters.iter().enumerate() {
        let info = adapter.get_info();
        if surface.is_some_and(|surface| surface.get_capabilities(adapter).formats.is_empty()) {
            tracing::debug!(adapter = %info.name, backend = ?info.backend, "skipping adapter without a surface format");
            continue;
        }
        if !preflight.adapters.is_empty()
            && !preflight
                .adapters
                .iter()
                .any(|candidate| candidate.matches(&info))
        {
            tracing::debug!(adapter = %info.name, backend = ?info.backend, "skipping adapter that failed graphics preflight");
            continue;
        }
        if !adapter_supports_live_sample_count(adapter, surface, preflight.live_sample_count) {
            // Warned, not debugged: this skip is the one that can leave the
            // selector with no adapter at all, and the default log filter would
            // hide the reason from a support report otherwise.
            tracing::warn!(
                adapter = %info.name,
                backend = ?info.backend,
                sample_count = preflight.live_sample_count,
                "skipping adapter that cannot create the configured live targets; \
                 set {LIVE_MSAA_ENV}=1 to start without multisampling"
            );
            continue;
        }

        let score = adapter_device_score(info.device_type, power_preference);
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, index));
        }
    }

    let Some((_, index)) = best else {
        return Err(format!(
            "no graphics adapter can present to the desktop surface; run `occluview --diagnostics` and check the GPU driver, or set {LIVE_MSAA_ENV}=1 to start without multisampling",
        ));
    };
    let adapter = adapters[index].clone();
    let info = adapter.get_info();
    tracing::info!(
        adapter = %info.name,
        backend = ?info.backend,
        device_type = ?info.device_type,
        "surface-compatible wgpu adapter selected"
    );
    Ok(adapter)
}

/// Check the exact formats eframe will use for this surface before allowing a
/// multisampled startup. The preflight runs before a window exists, while this
/// selector is the first point where the adapter's real surface format is
/// available; keeping both checks closes that timing gap.
fn adapter_supports_live_sample_count(
    adapter: &wgpu::Adapter,
    surface: Option<&wgpu::Surface<'_>>,
    sample_count: u16,
) -> bool {
    let color_format = surface
        .map(|surface| surface.get_capabilities(adapter).formats)
        .map_or_else(
            || Some(wgpu::TextureFormat::Rgba8Unorm),
            |formats| eframe::egui_wgpu::preferred_framebuffer_format(&formats).ok(),
        );
    color_format.is_some_and(|format| {
        format_supports_live_render(
            adapter.get_texture_format_features(format),
            sample_count,
            true,
        ) && format_supports_live_render(
            adapter.get_texture_format_features(LIVE_DEPTH_FORMAT),
            sample_count,
            false,
        )
    })
}

/// Check the complete format contract used by the live egui render pass.
///
/// `sample_count_supported(1)` is deliberately true even for formats that
/// cannot be render attachments, so checking the sample count alone lets an
/// adapter pass preflight and fail later while eframe creates the swapchain or
/// a custom pipeline. The live color target is also used by translucent
/// pipelines, which requires the format's `BLENDABLE` feature.
fn format_supports_live_render(
    features: wgpu::TextureFormatFeatures,
    sample_count: u16,
    requires_blending: bool,
) -> bool {
    features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        && (!requires_blending
            || features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE))
        && features
            .flags
            .sample_count_supported(u32::from(sample_count))
}

fn adapter_device_score(
    device_type: wgpu::DeviceType,
    power_preference: wgpu::PowerPreference,
) -> i32 {
    let base = match device_type {
        wgpu::DeviceType::DiscreteGpu => 40,
        wgpu::DeviceType::IntegratedGpu => 30,
        wgpu::DeviceType::VirtualGpu => 20,
        wgpu::DeviceType::Cpu => 10,
        wgpu::DeviceType::Other => 0,
    };
    match power_preference {
        wgpu::PowerPreference::LowPower if device_type == wgpu::DeviceType::IntegratedGpu => {
            base + 5
        }
        wgpu::PowerPreference::HighPerformance if device_type == wgpu::DeviceType::DiscreteGpu => {
            base + 5
        }
        _ => base,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdapterIdentity {
    name: String,
    vendor: u32,
    device: u32,
    device_type: wgpu::DeviceType,
    device_pci_bus_id: String,
    backend: wgpu::Backend,
    supports_live_msaa_4: bool,
}

impl AdapterIdentity {
    fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        let info = adapter.get_info();
        Self {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            device_type: info.device_type,
            device_pci_bus_id: info.device_pci_bus_id.clone(),
            backend: info.backend,
            supports_live_msaa_4: adapter_supports_preflight_msaa_4(adapter),
        }
    }

    fn matches(&self, info: &wgpu::AdapterInfo) -> bool {
        self.name == info.name
            && self.vendor == info.vendor
            && self.device == info.device
            && self.device_type == info.device_type
            && self.device_pci_bus_id == info.device_pci_bus_id
            && self.backend == info.backend
    }
}

/// Conservative, window-free capability check used to choose the startup
/// profile. The selector repeats the check against the exact surface format
/// once the window exists.
fn adapter_supports_preflight_msaa_4(adapter: &wgpu::Adapter) -> bool {
    format_supports_live_render(
        adapter.get_texture_format_features(LIVE_DEPTH_FORMAT),
        LIVE_MSAA_SAMPLE_COUNT,
        false,
    ) && LIVE_SURFACE_FORMATS.iter().all(|&format| {
        format_supports_live_render(
            adapter.get_texture_format_features(format),
            LIVE_MSAA_SAMPLE_COUNT,
            true,
        )
    })
}

fn select_live_sample_count(selected_adapter_supports_msaa_4: bool) -> u16 {
    if selected_adapter_supports_msaa_4 {
        LIVE_MSAA_SAMPLE_COUNT
    } else {
        LIVE_SAFE_SAMPLE_COUNT
    }
}

/// Environment switch for the live multisampling profile.
///
/// `OCCLUVIEW_LIVE_MSAA=1` forces the single-sample path. The selector cannot
/// retry after eframe has built its render pass, so a driver that cannot
/// present the multisampled configuration would otherwise leave the operator no
/// way in without a new build. `=4` forces the multisampled profile back on.
const LIVE_MSAA_ENV: &str = "OCCLUVIEW_LIVE_MSAA";

/// Parse [`LIVE_MSAA_ENV`]. Anything unrecognized leaves the decision to the
/// adapter capability, which is the safe default for a typo.
fn live_msaa_override(value: Option<&str>) -> Option<u16> {
    match value.map(str::trim) {
        Some("1" | "off" | "false") => Some(LIVE_SAFE_SAMPLE_COUNT),
        Some("4") => Some(LIVE_MSAA_SAMPLE_COUNT),
        _ => None,
    }
}

/// The live viewport sample count for the adapters the preflight proved usable.
///
/// The preflight runs before a window exists, so it cannot see which adapter
/// can present the eventual desktop surface. A 4x choice based on one
/// high-scoring adapter can therefore make a hybrid machine fail when the
/// surface selector later lands on another adapter that only supports 1x.
/// Keep 4x as the normal hardware path when every working candidate proved the
/// same profile; otherwise choose the universally safe single-sample pass.
/// This one value configures eframe's render pass and the custom viewport's
/// pipelines together. An explicit operator override still wins outright.
fn live_sample_count_for(
    adapters: &[AdapterIdentity],
    _power_preference: wgpu::PowerPreference,
    override_count: Option<u16>,
) -> u16 {
    if let Some(count) = override_count {
        return count;
    }
    // The selector may reject a higher-scoring adapter after it sees the
    // surface. Since this function cannot inspect that surface yet, requiring
    // the profile from every working candidate is the only default that never
    // asks eframe for a pass the eventual adapter cannot satisfy.
    let every_candidate_supports_msaa_4 =
        !adapters.is_empty() && adapters.iter().all(|adapter| adapter.supports_live_msaa_4);
    select_live_sample_count(every_candidate_supports_msaa_4)
}

fn validate_graphics_environment() -> Result<()> {
    let raw_backends = std::env::var_os("WGPU_BACKEND");
    let raw_power_preference = std::env::var_os("WGPU_POWER_PREF");
    validate_graphics_environment_values(raw_backends.as_deref(), raw_power_preference.as_deref())
}

fn validate_graphics_environment_values(
    raw_backends: Option<&std::ffi::OsStr>,
    raw_power_preference: Option<&std::ffi::OsStr>,
) -> Result<()> {
    if let Some(raw_backends) = raw_backends {
        let raw_backends = raw_backends.to_string_lossy();
        if wgpu::Backends::from_comma_list(&raw_backends).is_empty() {
            return Err(anyhow::anyhow!(
                "WGPU_BACKEND={raw_backends:?} selects no known graphics backend; unset it or use vulkan, dx12, metal, or gl"
            ));
        }
    }
    if let Some(raw_power_preference) = raw_power_preference {
        let raw_power_preference = raw_power_preference.to_string_lossy();
        if !matches!(
            raw_power_preference.to_ascii_lowercase().as_str(),
            "low" | "high" | "none"
        ) {
            return Err(anyhow::anyhow!(
                "WGPU_POWER_PREF={raw_power_preference:?} is invalid; unset it or use low, high, or none"
            ));
        }
    }
    Ok(())
}

/// Request a device before creating the desktop window so an unsupported
/// driver becomes a visible startup error instead of an eframe callback that
/// never reaches the application creator. Try every adapter in preference
/// order: a broken discrete driver must not hide a usable integrated or CPU
/// adapter on a machine with both integrated and discrete graphics.
fn preflight_graphics_devices() -> Result<GraphicsPreflight> {
    let (descriptor, power_preference) = native_graphics_profile();
    let backends = descriptor.backends;
    let instance = wgpu::Instance::new(descriptor);
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
    adapters.sort_by(|left, right| {
        let left_info = left.get_info();
        let right_info = right.get_info();
        adapter_device_score(right_info.device_type, power_preference).cmp(&adapter_device_score(
            left_info.device_type,
            power_preference,
        ))
    });
    if adapters.is_empty() {
        return Err(anyhow::anyhow!(
            "no graphics adapter was found for the selected backend; run `occluview --diagnostics` and install or update the GPU driver"
        ));
    }

    let mut failures = Vec::new();
    let mut working_adapters = Vec::new();
    for adapter in adapters {
        let info = adapter.get_info();
        match pollster::block_on(adapter.request_device(&device_descriptor_for_adapter(&adapter))) {
            Ok((_device, _queue)) => {
                working_adapters.push(AdapterIdentity::from_adapter(&adapter));
                tracing::info!(
                    adapter = %info.name,
                    backend = ?info.backend,
                    device_type = ?info.device_type,
                    power_preference = ?power_preference,
                    "graphics device preflight passed"
                );
            }
            Err(error) => failures.push(format!("{} ({:?}): {error}", info.name, info.backend)),
        }
    }
    if !working_adapters.is_empty() {
        if !failures.is_empty() {
            tracing::warn!(
                failed_adapters = ?failures,
                "some graphics adapters failed preflight; restricting surface selection to working adapters"
            );
        }
        let live_sample_count = live_sample_count_for(
            &working_adapters,
            power_preference,
            live_msaa_override(std::env::var(LIVE_MSAA_ENV).ok().as_deref()),
        );
        tracing::info!(
            sample_count = live_sample_count,
            override_env = %LIVE_MSAA_ENV,
            "live viewport sample count selected"
        );
        return Ok(GraphicsPreflight {
            adapters: working_adapters,
            live_sample_count,
        });
    }
    Err(anyhow::anyhow!(
        "no graphics adapter could create a device; run `occluview --diagnostics`; attempts: {}",
        failures.join("; ")
    ))
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
/// line is discarded when no console is attached; it prints whenever stdout is
/// piped or redirected, and always on Linux.
/// Attaching a parent console would drag in Win32 console plumbing for one
/// line.
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
/// Metadata only: native driver crashes can happen before Rust
/// reaches the panic hook, but the next report can still say whether the
/// process reached logging, graphics initialization, or the window callback.
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

fn graphics_diagnostics_report() -> String {
    use std::fmt::Write as _;

    let (descriptor, power_preference) = native_graphics_profile();
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
    let _ = writeln!(
        report,
        "{LIVE_MSAA_ENV}: {}",
        std::env::var(LIVE_MSAA_ENV).unwrap_or_else(|_| "<unset>".to_string())
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

    let mut working_identities: Vec<AdapterIdentity> = Vec::new();
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
        let _ = writeln!(
            report,
            "  msaa{count}_preflight: {}",
            if adapter_supports_preflight_msaa_4(adapter) {
                "supported"
            } else {
                "unsupported"
            },
            count = LIVE_MSAA_SAMPLE_COUNT,
        );
        let device_status = match pollster::block_on(
            adapter.request_device(&device_descriptor_for_adapter(adapter)),
        ) {
            Ok((_device, _queue)) => {
                working_identities.push(AdapterIdentity::from_adapter(adapter));
                "ok".to_string()
            }
            Err(error) => format!("error: {error}"),
        };
        let _ = writeln!(report, "  device_request: {device_status}");
    }
    // Startup decides the count over the adapters whose device request
    // succeeded, so the report has to use that same set. Printing it over every
    // enumerated adapter would describe a startup that never happened on a
    // machine where a broken driver enumerates and fails to create a device.
    let live_sample_count = live_sample_count_for(
        &working_identities,
        power_preference,
        live_msaa_override(std::env::var(LIVE_MSAA_ENV).ok().as_deref()),
    );
    let _ = writeln!(
        report,
        "live_sample_count: {live_sample_count} (over {} adapter(s) that created a device)",
        working_identities.len()
    );
    report
}

fn graphics_diagnostics_details(validation: Result<()>) -> String {
    match validation {
        Ok(()) => graphics_diagnostics_report(),
        Err(error) => format!(
            "OccluView graphics diagnostics\nversion: {}\nstatus: invalid_environment\nerror: {error:#}\n",
            env!("CARGO_PKG_VERSION")
        ),
    }
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

    #[cfg(not(windows))]
    {
        // A .desktop launch has no console. Give the operator the full path,
        // not just a filename they cannot locate, and put the same actionable
        // value in the next startup breadcrumb if every dialog channel fails.
        // Keep the no-file state machine-readable. This sentinel is also
        // handled by the Windows diagnostics dialog below; a prose fallback
        // here would bypass the presentation-sink contract before the locale
        // manager exists.
        let report =
            report_path.map_or_else(|| "none".to_owned(), |path| path.display().to_string());
        tracing::error!(report, details, "OccluView could not continue");
        notify_desktop("OccluView could not start", &report, "critical");
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

    #[cfg(not(windows))]
    {
        tracing::info!(report, "graphics diagnostics written");
        notify_desktop("OccluView graphics diagnostics", &report, "normal");
    }
}

#[cfg(not(windows))]
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
                // is the common case on a bare session. Trying the next
                // channel is the whole point of the list.
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

/// Run one notification program and report whether it accepted the message.
///
/// Waiting for the exit status is what separates "the message was shown" from
/// "a binary with that name exists": `notify-send` succeeds only when a
/// notification daemon answered it. `zenity` and `kdialog` are modal dialogs
/// that return when dismissed, which is intended here - on this path the
/// message has to outlive the process that raised it.
#[cfg(not(windows))]
fn run_notification(program: &str, args: &[String]) -> std::io::Result<bool> {
    let status = std::process::Command::new(program).args(args).status()?;
    Ok(status.success())
}

/// The notification channels tried, in the order they are attempted.
///
/// `notify-send` is the freedesktop one; `zenity` and `kdialog` are what a
/// minimal desktop image tends to carry when no notification daemon answers;
/// `xmessage` is the last X11-only modal fallback.
#[cfg(not(windows))]
const NOTIFICATION_CHANNELS: [&str; 4] = ["notify-send", "zenity", "kdialog", "xmessage"];

/// The command line for one notification channel.
///
/// The body carries the full report path and the title names the product, so a
/// desktop dialog is actionable without the console a `.desktop` launch never
/// has. Each channel's exit status is read by [`run_notification`], so a
/// channel with no service behind it does not consume the message.
#[cfg(not(windows))]
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

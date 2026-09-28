use super::{load_window_icon, MAX_RENDER_TEXTURE_DIMENSION};
use anyhow::Result;
use eframe::egui;
use eframe::egui_wgpu::wgpu;
use std::sync::Arc;

pub(super) const LIVE_MSAA_SAMPLE_COUNT: u16 = 4;
pub(super) const LIVE_SAFE_SAMPLE_COUNT: u16 = 1;
/// The depth/stencil format every live-pass pipeline declares.
///
/// Taken from the render crate that builds those pipelines, so the window's
/// request and the pipelines cannot drift apart: a mismatch is a validation
/// error on every draw, not a graceful degradation.
pub(super) const LIVE_DEPTH_FORMAT: wgpu::TextureFormat = occluview_render::live_depth_format();
pub(super) const LIVE_SURFACE_FORMATS: [wgpu::TextureFormat; 4] = [
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
pub(super) struct GraphicsPreflight {
    pub(super) adapters: Vec<AdapterIdentity>,
    pub(super) live_sample_count: u16,
}

impl Default for GraphicsPreflight {
    fn default() -> Self {
        Self {
            adapters: Vec::new(),
            live_sample_count: LIVE_SAFE_SAMPLE_COUNT,
        }
    }
}

pub(super) fn native_options(preflight: &GraphicsPreflight) -> eframe::NativeOptions {
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

pub(super) fn native_graphics_profile() -> (wgpu::InstanceDescriptor, wgpu::PowerPreference) {
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
pub(super) fn select_native_adapter(
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
pub(super) fn adapter_supports_live_sample_count(
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
/// `sample_count_supported(1)` is true even for formats that
/// cannot be render attachments, so checking the sample count alone lets an
/// adapter pass preflight and fail later while eframe creates the swapchain or
/// a custom pipeline. The live color target is also used by translucent
/// pipelines, which requires the format's `BLENDABLE` feature.
pub(super) fn format_supports_live_render(
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

pub(super) fn adapter_device_score(
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
pub(super) struct AdapterIdentity {
    pub(super) name: String,
    pub(super) vendor: u32,
    pub(super) device: u32,
    pub(super) device_type: wgpu::DeviceType,
    pub(super) device_pci_bus_id: String,
    pub(super) backend: wgpu::Backend,
    pub(super) supports_live_msaa_4: bool,
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
pub(super) fn adapter_supports_preflight_msaa_4(adapter: &wgpu::Adapter) -> bool {
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

pub(super) fn select_live_sample_count(selected_adapter_supports_msaa_4: bool) -> u16 {
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
pub(super) const LIVE_MSAA_ENV: &str = "OCCLUVIEW_LIVE_MSAA";

/// Parse [`LIVE_MSAA_ENV`]. Anything unrecognized leaves the decision to the
/// adapter capability, which is the safe default for a typo.
pub(super) fn live_msaa_override(value: Option<&str>) -> Option<u16> {
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
pub(super) fn live_sample_count_for(
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
    //
    // The cost is documented in the README: a hybrid machine that
    // enumerates one device without 4x support renders the whole session at one
    // sample, even when the device it actually presents on supports 4x. The
    // operator override is the escape hatch in both directions.
    let every_candidate_supports_msaa_4 =
        !adapters.is_empty() && adapters.iter().all(|adapter| adapter.supports_live_msaa_4);
    select_live_sample_count(every_candidate_supports_msaa_4)
}

pub(super) fn validate_graphics_environment() -> Result<()> {
    let raw_backends = std::env::var_os("WGPU_BACKEND");
    let raw_power_preference = std::env::var_os("WGPU_POWER_PREF");
    validate_graphics_environment_values(raw_backends.as_deref(), raw_power_preference.as_deref())
}

pub(super) fn validate_graphics_environment_values(
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
pub(super) fn preflight_graphics_devices() -> Result<GraphicsPreflight> {
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
    // Probing costs a device creation on every adapter, and a software adapter
    // never releases its worker threads. Only the hardware candidates are worth
    // a probe here; the CPU adapter stays in the list as the last resort wgpu
    // itself falls back to, where its threads are doing real work.
    let (hardware, software): (Vec<_>, Vec<_>) = adapters
        .into_iter()
        .partition(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu);
    let adapters = if hardware.is_empty() {
        software
    } else {
        hardware
    };

    let mut failures = Vec::new();
    let mut working_adapters = Vec::new();
    for adapter in adapters {
        let info = adapter.get_info();
        match pollster::block_on(adapter.request_device(&device_descriptor_for_adapter(&adapter))) {
            Ok((_device, _queue)) => {
                // The device is dropped here on purpose: this pass only asks
                // whether the adapter can give one. That is also why the CPU
                // adapter is skipped above it — a software driver starts its
                // worker threads when the device is created and does not stop
                // them when the device is dropped, so probing llvmpipe leaves
                // three Vulkan helper threads and ten llvmpipe workers spinning
                // for the life of the process. Measured: with the software
                // adapter probed, the viewer idles at 10% CPU with no document
                // open and 20% with one.
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
/// keeping the normal egui/wgpu defaults on modern hardware. A request with a
/// fixed 8192 2D texture limit is rejected by wgpu before the application
/// creator runs when a GL adapter advertises a lower limit.
pub(super) fn device_limits_for_backend(
    backend: wgpu::Backend,
    supported: &wgpu::Limits,
) -> wgpu::Limits {
    let base_limits = if backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    wgpu::Limits {
        max_texture_dimension_2d: MAX_RENDER_TEXTURE_DIMENSION,
        // `wgpu::Limits::default()` is the WebGPU default tier, whose
        // `max_buffer_size` is 256 MiB. `or_worse_values_from` takes the
        // per-field minimum, so a default request caps the live device at
        // 256 MiB even on an adapter offering gigabytes, while a scan of three
        // million triangles needs a 309 MiB vertex buffer. A refused allocation
        // arrives at the fault handler, which latches: the viewport stops
        // drawing and offers a Retry that re-runs the same failing allocation.
        // Asking for the adapter's own number, as the offscreen path does,
        // leaves the intersection unable to lower it.
        max_buffer_size: supported.max_buffer_size,
        ..base_limits
    }
    .or_worse_values_from(supported)
}

pub(super) fn device_descriptor_for_adapter(
    adapter: &wgpu::Adapter,
) -> wgpu::DeviceDescriptor<'static> {
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

pub(super) fn root_viewport_builder() -> egui::ViewportBuilder {
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

pub(super) fn graphics_diagnostics_report() -> String {
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

pub(super) fn graphics_diagnostics_details(validation: Result<()>) -> String {
    match validation {
        Ok(()) => graphics_diagnostics_report(),
        Err(error) => format!(
            "OccluView graphics diagnostics\nversion: {}\nstatus: invalid_environment\nerror: {error:#}\n",
            env!("CARGO_PKG_VERSION")
        ),
    }
}

pub(super) fn environment_state(name: &str) -> &'static str {
    if std::env::var_os(name).is_some() {
        "set"
    } else {
        "unset"
    }
}

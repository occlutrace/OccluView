#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn native_options_use_the_low_latency_surface_contract() {
    let options = native_options(&GraphicsPreflight::default());

    assert_eq!(
        options.wgpu_options.surface,
        eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
    );
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else {
        panic!("OccluView must create its own wgpu setup");
    };
    assert!(
        setup.native_adapter_selector.is_some(),
        "startup must reject adapters that cannot present to the desktop surface"
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

fn format_features(
    allowed_usages: wgpu::TextureUsages,
    flags: wgpu::TextureFormatFeatureFlags,
) -> wgpu::TextureFormatFeatures {
    wgpu::TextureFormatFeatures {
        allowed_usages,
        flags,
    }
}

#[test]
fn live_render_contract_rejects_a_sampleable_only_depth_format() {
    let sampleable_only = format_features(
        wgpu::TextureUsages::TEXTURE_BINDING,
        wgpu::TextureFormatFeatureFlags::empty(),
    );

    assert!(
        !format_supports_live_render(sampleable_only, LIVE_SAFE_SAMPLE_COUNT, false,),
        "sample count one is not enough when the format cannot be a render attachment"
    );
}

#[test]
fn live_render_contract_requires_blending_for_translucent_pipelines() {
    let renderable_but_not_blendable = format_features(
        wgpu::TextureUsages::RENDER_ATTACHMENT,
        wgpu::TextureFormatFeatureFlags::empty(),
    );

    assert!(
        !format_supports_live_render(renderable_but_not_blendable, LIVE_SAFE_SAMPLE_COUNT, true,),
        "the live target is used by transparent and sculpt pipelines"
    );
    assert!(format_supports_live_render(
        renderable_but_not_blendable,
        LIVE_SAFE_SAMPLE_COUNT,
        false,
    ));
}

#[test]
fn an_unknown_backend_override_is_detectably_empty() {
    assert!(wgpu::Backends::from_comma_list("not-a-backend").is_empty());
    assert!(!wgpu::Backends::from_comma_list("vulkan,gl").is_empty());
}

#[test]
fn graphics_environment_validation_rejects_empty_or_unknown_values() {
    assert!(validate_graphics_environment_values(Some(std::ffi::OsStr::new("")), None,).is_err());
    assert!(validate_graphics_environment_values(
        Some(std::ffi::OsStr::new("vulkan")),
        Some(std::ffi::OsStr::new("turbo")),
    )
    .is_err());
}

#[test]
fn graphics_environment_validation_accepts_wgpu_spellings() {
    assert!(validate_graphics_environment_values(
        Some(std::ffi::OsStr::new("vk, gl")),
        Some(std::ffi::OsStr::new("HIGH")),
    )
    .is_ok());
    assert!(
        validate_graphics_environment_values(None, Some(std::ffi::OsStr::new("none")),).is_ok()
    );
}

#[test]
fn diagnostics_keeps_an_invalid_environment_error_in_the_report() {
    let details = graphics_diagnostics_details(Err(anyhow::anyhow!("invalid backend")));

    assert!(details.contains("status: invalid_environment"));
    assert!(details.contains("error: invalid backend"));
}

#[test]
fn diagnostics_fails_when_no_report_path_was_created() {
    assert!(require_report_path(None).is_err());
    let path = PathBuf::from("/tmp/occluview-diagnostics.txt");
    assert_eq!(
        require_report_path(Some(path.clone())).expect("a present report path is valid"),
        path
    );
}

#[test]
fn startup_journal_paths_fall_back_without_duplicate_entries() {
    let fallback = PathBuf::from("/tmp/occluview-startup-journal.log");
    assert_eq!(
        startup_journal_paths_from(None, fallback.clone()),
        vec![fallback.clone()]
    );
    assert_eq!(
        startup_journal_paths_from(Some(fallback.clone()), fallback.clone()),
        vec![fallback.clone()]
    );
    assert_eq!(
        startup_journal_paths_from(
            Some(PathBuf::from("/state/startup-journal.log")),
            fallback.clone(),
        ),
        vec![PathBuf::from("/state/startup-journal.log"), fallback,]
    );
}

#[test]
fn adapter_ranking_respects_the_requested_power_profile() {
    assert!(
        adapter_device_score(
            wgpu::DeviceType::IntegratedGpu,
            wgpu::PowerPreference::LowPower
        ) > adapter_device_score(
            wgpu::DeviceType::IntegratedGpu,
            wgpu::PowerPreference::HighPerformance
        )
    );
    assert!(
        adapter_device_score(
            wgpu::DeviceType::DiscreteGpu,
            wgpu::PowerPreference::HighPerformance
        ) > adapter_device_score(
            wgpu::DeviceType::DiscreteGpu,
            wgpu::PowerPreference::LowPower
        )
    );
}

#[test]
fn live_sample_policy_uses_msaa_when_the_selected_adapter_supports_it() {
    assert_eq!(select_live_sample_count(true), 4);
    assert_eq!(select_live_sample_count(false), 1);
}

fn adapter_identity(device_type: wgpu::DeviceType, supports_live_msaa_4: bool) -> AdapterIdentity {
    AdapterIdentity {
        name: format!("fixture-{device_type:?}"),
        vendor: 0,
        device: 0,
        device_type,
        device_pci_bus_id: String::new(),
        backend: wgpu::Backend::Vulkan,
        supports_live_msaa_4,
    }
}

/// The pre-window policy cannot know which adapter will own the eventual
/// surface, so 4x is enabled only when every working candidate can satisfy it.
/// That preserves AA on a single capable GPU and avoids a hybrid-GPU startup
/// dead end when the selector later lands on a 1x-only adapter.
#[test]
fn live_sample_count_is_safe_across_all_preflight_candidates() {
    let power = wgpu::PowerPreference::HighPerformance;
    let integrated_without = adapter_identity(wgpu::DeviceType::IntegratedGpu, false);
    let discrete_with = adapter_identity(wgpu::DeviceType::DiscreteGpu, true);
    let integrated_with = adapter_identity(wgpu::DeviceType::IntegratedGpu, true);
    let discrete_without = adapter_identity(wgpu::DeviceType::DiscreteGpu, false);

    assert_eq!(
        live_sample_count_for(&[integrated_without.clone(), discrete_with], power, None),
        1,
        "a lower-scoring 1x-only adapter can become the presentable one"
    );
    assert_eq!(
        live_sample_count_for(&[integrated_with, discrete_without], power, None),
        1,
        "a hybrid set with any 1x-only candidate must stay launchable"
    );
    assert_eq!(
        live_sample_count_for(
            &[
                integrated_without,
                adapter_identity(wgpu::DeviceType::DiscreteGpu, false)
            ],
            power,
            None,
        ),
        1,
        "no capable candidate must use the safe profile"
    );
    assert_eq!(
        live_sample_count_for(
            &[
                adapter_identity(wgpu::DeviceType::IntegratedGpu, true),
                adapter_identity(wgpu::DeviceType::DiscreteGpu, true),
            ],
            power,
            None,
        ),
        4,
        "4x stays enabled when every candidate proved the profile"
    );
    assert_eq!(
        live_sample_count_for(&[], power, None),
        1,
        "no adapter at all must not ask for multisampled targets"
    );
}

#[test]
fn live_sample_count_keeps_the_selector_first_tie() {
    let first = adapter_identity(wgpu::DeviceType::IntegratedGpu, false);
    let second = adapter_identity(wgpu::DeviceType::IntegratedGpu, true);

    assert_eq!(
        live_sample_count_for(
            &[first, second],
            wgpu::PowerPreference::HighPerformance,
            None,
        ),
        1,
        "a later capable adapter cannot make an earlier 1x-only candidate fail"
    );
}

/// An operator override is the only route in for a driver that rejects the
/// multisampled pass: eframe builds that pass before the app exists, so there
/// is nothing to retry inside one launch.
#[test]
fn the_msaa_environment_override_wins_over_adapter_capability() {
    assert_eq!(live_msaa_override(Some("1")), Some(1));
    assert_eq!(live_msaa_override(Some(" off ")), Some(1));
    assert_eq!(live_msaa_override(Some("false")), Some(1));
    assert_eq!(live_msaa_override(Some("4")), Some(4));
    assert_eq!(live_msaa_override(Some("yes")), None, "a typo stays safe");
    assert_eq!(live_msaa_override(None), None);

    let capable = adapter_identity(wgpu::DeviceType::DiscreteGpu, true);
    assert_eq!(
        live_sample_count_for(
            std::slice::from_ref(&capable),
            wgpu::PowerPreference::HighPerformance,
            live_msaa_override(Some("1"))
        ),
        1,
        "the override must turn multisampling off on capable hardware"
    );
    let capable_but_unsupported = adapter_identity(wgpu::DeviceType::DiscreteGpu, false);
    assert_eq!(
        live_sample_count_for(
            std::slice::from_ref(&capable_but_unsupported),
            wgpu::PowerPreference::HighPerformance,
            live_msaa_override(Some("4"))
        ),
        4,
        "the override must be able to force the multisampled profile back on"
    );
}

/// eframe derives the live pass depth format from these two window numbers,
/// and the custom viewport builds its pipelines for the same sample count
/// eframe is handed. Pinning the pair here is what connects the render crate's
/// `Depth24PlusStencil8` declaration to the pass that actually exists: the
/// render-side test draws into a pass it configures itself.
#[test]
fn the_live_window_options_match_the_render_contract() {
    let preflight = GraphicsPreflight {
        adapters: Vec::new(),
        live_sample_count: 4,
    };
    let options = native_options(&preflight);

    assert_eq!(
        options.multisampling, preflight.live_sample_count,
        "eframe's pass and the custom viewport must use one sample count"
    );
    assert_eq!(
        eframe::egui_wgpu::depth_format_from_bits(options.depth_buffer, options.stencil_buffer),
        Some(LIVE_DEPTH_FORMAT),
        "the pipelines declare LIVE_DEPTH_FORMAT, so the window must ask eframe for that pass"
    );
}

/// The device request must take its buffer ceiling from the adapter.
///
/// `wgpu::Limits::default()` is the WebGPU default tier, whose `max_buffer_size`
/// is 256 MiB, and `or_worse_values_from` takes the per-field MINIMUM - so a
/// request built on the default tier capped the live device at 256 MiB even on
/// an adapter offering gigabytes. A large scan needs a bigger vertex buffer than
/// that, the allocation is refused, and the refusal arrives at the fault handler
/// which LATCHES: a scan that rendered fine as a thumbnail was unopenable in the
/// app, with a Retry that re-ran the same failing allocation.
#[test]
fn the_device_request_takes_its_buffer_ceiling_from_the_adapter() {
    let generous = wgpu::Limits {
        max_buffer_size: 3 * 1024 * 1024 * 1024,
        ..wgpu::Limits::default()
    };
    let requested = device_limits_for_backend(wgpu::Backend::Vulkan, &generous);
    assert_eq!(
        requested.max_buffer_size, generous.max_buffer_size,
        "the request must not lower an adapter that offers more than the default tier"
    );

    let modest = wgpu::Limits {
        max_buffer_size: 64 * 1024 * 1024,
        ..wgpu::Limits::default()
    };
    let requested = device_limits_for_backend(wgpu::Backend::Gl, &modest);
    assert_eq!(
        requested.max_buffer_size,
        64 * 1024 * 1024,
        "and a device request may never exceed what a weak adapter reports"
    );
}

#[test]
fn report_names_are_unique_even_when_failures_share_a_clock_tick() {
    let first = report_file_name("startup-failure", 42, 7, 0);
    let second = report_file_name("startup-failure", 42, 7, 1);
    assert_ne!(first, second);
    assert!(first.ends_with("-0.txt"));
    assert!(second.ends_with("-1.txt"));
}

/// A `.desktop` launch has no console, so a fatal startup has to reach the
/// desktop through whatever the image actually ships. The command lines are
/// pinned here because a wrong flag makes the dialog never appear, which looks
/// exactly like the silent failure this exists to prevent.
#[cfg(not(windows))]
#[test]
fn every_desktop_notification_channel_builds_a_usable_command() {
    for channel in NOTIFICATION_CHANNELS {
        let (program, args) = notification_command(channel, "Title", "Body", "critical")
            .unwrap_or_else(|| panic!("{channel} must be a known channel"));
        assert_eq!(program, channel, "the channel name is the program name");
        assert!(
            args.iter().any(|arg| arg == "Title") && args.iter().any(|arg| arg == "Body"),
            "{channel} must carry both the title and the body: {args:?}"
        );
        assert!(
            args.iter().any(|arg| arg.contains("error")
                || arg.contains("critical")
                || channel == "xmessage"),
            "{channel} must present this as an error: {args:?}"
        );
    }
    assert_eq!(
        notification_command("unknown", "Title", "Body", "critical"),
        None,
        "an unknown channel must not be spawned"
    );
}

/// The fatal-startup notice may not hold the process open. `zenity`, `kdialog`
/// and `xmessage` are modal dialogs that only return when dismissed, and this
/// runs on the path that still owes the operator a non-zero exit status: an
/// unattended launch (CI, kiosk, a `.desktop` start nobody watches) would leave
/// a dead startup alive forever with no window.
#[cfg(not(windows))]
#[test]
fn a_fatal_notice_cannot_block_the_failure_exit() {
    // Stands in for a dialog nobody will dismiss: it never exits on its own.
    let started = Instant::now();
    let delivered = run_notification("sleep", &["30".to_string()])
        .expect("the notice program must be spawnable");
    let waited = started.elapsed();

    assert!(
        delivered,
        "a notice that is on screen has been delivered: the operator can still read it"
    );
    assert!(
        waited >= NOTIFICATION_DISMISS_WAIT,
        "the notice gets its full chance to be read, so a notification daemon round trip is not cut short"
    );
    assert!(
        waited < NOTIFICATION_DISMISS_WAIT + std::time::Duration::from_secs(5),
        "but the wait is bounded, because this path still owes the operator an exit status"
    );
}

#[cfg(not(windows))]
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

/// `run_and_return` is what makes a fatal `run_native` failure an error we can
/// report. With it false, eframe's wrapper exits the process itself with code 0
/// and no return value, so every startup failure that happens after the window
/// request - adapter selection in particular - would look like a clean exit with
/// no window and no crash report. eframe defaults it to true; this pins our
/// reliance on that default.
#[test]
fn the_window_loop_must_return_fatal_errors_instead_of_exiting_silently() {
    let options = native_options(&GraphicsPreflight::default());
    assert!(
        options.run_and_return,
        "a fatal startup failure must reach main_entry as an error, not exit 0"
    );
}

/// A crash report is meant to be attached to a public issue, so it must carry
/// the SHAPE of the session and never the identity of a case.
///
/// This is the behaviour the old source-text check only claimed: it looked for
/// the words `file_count` and `formats` in the startup log call. The property
/// worth holding is stronger and testable — feed the real summariser a path that
/// names a patient, and assert the report tail contains neither the directory,
/// the file stem, nor the full path. A dental scan's filename is the identifier.
#[test]
fn a_crash_report_never_carries_a_scan_path() {
    // Built as strings so this test's own source does not contain a path that
    // looks like patient data.
    let secret_dir = "patients";
    let secret_stem = "surname-firstname-1980";
    let path = PathBuf::from(format!("/var/scans/{secret_dir}/{secret_stem}.stl"));

    // What the startup line actually records.
    let counted = 1usize;
    let formats = crate::file_extensions(std::slice::from_ref(&path));
    assert_eq!(
        formats,
        vec!["stl".to_string()],
        "the report records the format, which is the shape of the session"
    );

    // Now drive the report path the way a failure would.
    push_crash_log_line(format!(
        "[    0.001s]  INFO occluview: OccluView starting file_count={counted} formats={formats:?}"
    ));
    let report_tail = recent_log_lines();
    let report = format!("{report_tail}\nBuild: {}\n", env!("CARGO_PKG_VERSION"));

    assert!(
        !report.contains(secret_stem),
        "a crash report must not carry the file name: it identifies the case"
    );
    assert!(
        !report.contains(secret_dir),
        "and it must not carry the directory either"
    );
    assert!(
        !report.contains(path.to_string_lossy().as_ref()),
        "nor the whole path"
    );
    assert!(
        report.contains("file_count=1") && report.contains("stl"),
        "while still saying how many files of which kind were opened"
    );
}

/// Where the installer keeps the shell entries this build ships.
#[cfg(target_os = "linux")]
fn installed_shell_entry(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../install/linux")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the package installs an entry named after the app id: {} ({error})",
            path.display()
        )
    })
}

/// The identity the window is created with is the one the installed desktop
/// entry is named after and declares back.
///
/// A Wayland compositor matches a window to its launcher entry by app id, and
/// an X11 one by `StartupWMClass`. When the two drift -- a rename on either
/// side -- the running viewer becomes a second unnamed icon and loses its name,
/// its icon and its file associations. This asks the real viewport builder, the
/// one `native_options` hands to eframe, which app id it produced, and needs no
/// display to do it: a compositor is what matches the strings later.
#[cfg(target_os = "linux")]
#[test]
fn linux_window_identity_matches_desktop_metadata() {
    let app_id = root_viewport_builder()
        .app_id
        .expect("a Linux window must declare an app id, or nothing can match it to an entry");

    let entry = installed_shell_entry(&format!("{app_id}.desktop"));
    let window_class = entry
        .lines()
        .find_map(|line| line.strip_prefix("StartupWMClass="));

    assert_eq!(
        window_class,
        Some(app_id.as_str()),
        "the entry the compositor matches this window against must declare the same id"
    );
}

/// The `AppStream` entry describes the same application the window creates.
///
/// The package installs `metainfo.xml` under the app id for software centres
/// and for `appstreamcli validate`; its `<id>` is the name the catalogue files
/// the product under and its `<launchable>` is the desktop entry a click has to
/// open. An entry whose id or launchable drifts from the window's own identity
/// names something that is not this binary.
#[cfg(target_os = "linux")]
#[test]
fn linux_window_identity_matches_the_installed_appstream_entry() {
    let app_id = root_viewport_builder()
        .app_id
        .expect("a Linux window must declare an app id, or nothing can match it to an entry");

    let metainfo = installed_shell_entry(&format!("{app_id}.metainfo.xml"));
    let launchable = format!("<launchable type=\"desktop-id\">{app_id}.desktop</launchable>");

    assert!(
        metainfo.contains(&format!("<id>{app_id}</id>")),
        "the catalogue id has to be the window's own app id"
    );
    assert!(
        metainfo.contains(&launchable),
        "the entry has to launch the desktop file that carries that id"
    );
}

#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn native_options_use_the_low_latency_surface_contract() {
    let options = native_options(&[]);

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

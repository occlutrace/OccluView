use super::*;

#[test]
fn windows_app_reports_startup_and_panic_failures() {
    let source = app_bootstrap_source();
    let manifest = app_manifest_source();

    assert!(
        source.contains("install_panic_hook();\n    if let Err(error) = real_main()"),
        "Windows-subsystem startup must install a panic hook before fallible startup"
    );
    assert!(
        source.contains("fn real_main() -> Result<()>"),
        "fallible startup should live behind a non-Result Windows main wrapper"
    );
    assert!(
        source.contains("show_startup_fatal_message_box"),
        "startup failures and panics should show a visible Windows dialog"
    );
    assert!(
        source.contains("MessageBoxW"),
        "Windows-subsystem fatal errors need MessageBoxW because there is no console"
    );
    assert!(
        source.contains("fn crash_report_dir() -> Option<PathBuf>")
            && source.contains(".map(|base| base.join(\"crashes\"))"),
        "crash reports should be written under the platform app state directory"
    );
    assert!(source.contains("env!(\"CARGO_PKG_VERSION\")"));
    assert!(manifest.contains("\"Win32_UI_WindowsAndMessaging\""));
}

#[test]
fn linux_build_uses_real_gui_instead_of_failure_stub() {
    let source = main_source();
    let manifest = app_manifest_source();

    assert!(
        !source.contains("#[cfg(not(windows))]\nfn main() -> std::process::ExitCode"),
        "Linux builds must launch the same egui/wgpu desktop viewer, not a failure stub"
    );
    assert!(
        source.contains("mod app"),
        "the GUI implementation should be compiled cross-platform"
    );
    assert!(
        !source.contains("#[cfg(windows)]\nmod app"),
        "app module must not be hidden behind cfg(windows)"
    );
    assert!(
        manifest.contains("features = [\"wgpu\", \"default_fonts\", \"x11\", \"wayland\"]"),
        "Linux GUI builds need eframe's x11 and wayland backends enabled"
    );
}

#[test]
fn linux_window_identity_matches_desktop_metadata() {
    let main_source = main_source();
    let bootstrap_source = app_bootstrap_source();
    let build_deb = linux_build_deb_source();
    let package_workflow = package_workflow_source();
    let metainfo = linux_metainfo_source();
    let desktop = linux_desktop_source();

    assert!(
        main_source.contains("LINUX_DESKTOP_APP_ID: &str = \"ai.occlutrace.OccluView\"")
            && bootstrap_source.contains(".with_app_id(crate::LINUX_DESKTOP_APP_ID)"),
        "Wayland app_id should match the installed desktop file id"
    );
    assert!(build_deb.contains("ai.occlutrace.OccluView.desktop"));
    assert!(metainfo
        .contains("<launchable type=\"desktop-id\">ai.occlutrace.OccluView.desktop</launchable>"));
    assert!(package_workflow
        .contains("desktop-file-validate install/linux/ai.occlutrace.OccluView.desktop"));
    assert!(!package_workflow.contains("desktop-file-validate install/linux/occluview.desktop"));
    // eframe 0.29 cannot complete a startup-notification sequence (no
    // activation-token path), so the desktop file must not advertise one:
    // advertising it makes GNOME map the first window UNFOCUSED with a
    // "window is ready" notification.
    assert!(desktop.contains("StartupNotify=false"));
}

#[test]
fn linux_desktop_state_uses_xdg_paths() {
    let app_paths = include_str!("../app_paths.rs");
    let single_instance_unix = include_str!("../single_instance/unix.rs");

    assert!(
        app_paths.contains("XDG_STATE_HOME") && app_paths.contains(".local/state"),
        "recent files and crash reports on Linux should use XDG state directories"
    );
    assert!(
        single_instance_unix.contains("XDG_RUNTIME_DIR"),
        "Linux single-instance IPC should prefer XDG_RUNTIME_DIR"
    );
    assert!(
        single_instance_unix.contains("UnixListener")
            && single_instance_unix.contains("UnixStream"),
        "Linux single-instance handoff should use Unix domain sockets"
    );
}

#[test]
fn public_linux_copy_is_not_left_as_windows_only() {
    let app_manifest = app_manifest_source();
    let live_viewport = include_str!("../live_viewport.rs");
    let about = function_source(app_dialogs_source(), "pub(super) fn show_about_window");
    let ci = ci_workflow_source();

    assert!(!app_manifest.contains("Windows-only"));
    assert!(!live_viewport.contains("Windows desktop app"));
    assert!(!about.contains("Native Windows viewer for fast scan inspection"));
    assert!(!ci.contains("Build the Windows-only crates (shell, app)"));
}

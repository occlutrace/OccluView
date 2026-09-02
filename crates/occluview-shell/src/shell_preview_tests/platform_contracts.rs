use std::path::Path;

#[test]
fn linux_host_has_windows_msvc_build_script() {
    let script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-windows-msvc.sh");
    assert!(script_path.exists());

    let script = include_str!("../../../../scripts/build-windows-msvc.sh");
    assert!(script.contains("cargo xwin build"));
    assert!(script.contains("x86_64-pc-windows-msvc"));
    assert!(script.contains("-p occluview-app"));
    assert!(script.contains("-p occluview-shell"));
    assert!(script.contains("occluview.exe"));
    assert!(script.contains("occluview_shell.dll"));
    assert!(script.contains("CARGO_ENCODED_RUSTFLAGS"));
    assert!(script.contains("cargo xwin env --target \"$target\""));
    assert!(script.contains("export CMAKE_TOOLCHAIN_FILE="));
    assert!(script.contains("manifold-csg-sys-*/out/build/CMakeCache.txt"));
    assert!(script.contains("--profile release-unwind"));
    assert!(!script.contains("-p occluview-cli"));
    assert!(!script.contains("occluview-cli.exe"));
}

#[test]
fn linux_install_assets_cover_freedesktop_and_deb_packaging() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let linux = repo.join("install/linux");

    for asset in [
        "ai.occlutrace.OccluView.desktop",
        "ai.occlutrace.OccluView.metainfo.xml",
        "ai.occlutrace.OccluView.thumbnailer",
        "occluview-mime.xml",
        "build-deb.sh",
        "check-deb.sh",
        "copyright",
    ] {
        assert!(linux.join(asset).exists(), "missing Linux asset: {asset}");
    }

    let desktop = std::fs::read_to_string(linux.join("ai.occlutrace.OccluView.desktop"))
        .expect("desktop file should be readable");
    assert!(desktop.contains("Exec=occluview %F"));
    assert!(desktop.contains("MimeType=model/stl;model/obj;model/gltf-binary;"));
    assert!(desktop.contains("Keywords=") && desktop.contains("STL;"));

    let thumbnailer = std::fs::read_to_string(linux.join("ai.occlutrace.OccluView.thumbnailer"))
        .expect("thumbnailer file should be readable");
    assert!(thumbnailer.contains("Exec=occluview-cli thumbnail %i -o %o --size %s"));
    assert!(thumbnailer.contains("MimeType=model/stl;model/obj;model/gltf-binary;"));

    let deb_script =
        std::fs::read_to_string(linux.join("build-deb.sh")).expect("deb script should be readable");
    let check_script = std::fs::read_to_string(linux.join("check-deb.sh"))
        .expect("deb check script should be readable");
    for package in [
        "libc6",
        "libgcc-s1",
        "libx11-6",
        "libxcb1",
        "libxcursor1",
        "libxi6",
        "libxrandr2",
        "libxkbcommon0",
        "libwayland-client0",
        "libwayland-cursor0",
        "libwayland-egl1",
        "libvulkan1",
        "desktop-file-utils",
        "shared-mime-info",
        "hicolor-icon-theme",
        "xdg-desktop-portal",
    ] {
        assert!(
            deb_script.contains(package),
            "Debian package should declare runtime dependency {package}"
        );
    }

    for required_path in [
        "usr/bin/occluview",
        "usr/bin/occluview-cli",
        "usr/share/applications/ai.occlutrace.OccluView.desktop",
        "usr/share/metainfo/ai.occlutrace.OccluView.metainfo.xml",
        "usr/share/mime/packages/occluview-mime.xml",
        "usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer",
        "usr/share/icons/hicolor/512x512/apps/occluview.png",
        "usr/share/icons/hicolor/scalable/mimetypes/model-stl.svg",
        "usr/share/icons/hicolor/scalable/mimetypes/application-x-occluview-hps.svg",
        "usr/share/doc/occluview/README.md",
        "usr/share/doc/occluview/copyright",
        "usr/share/doc/occluview/NEWS.gz",
        "usr/share/doc/occluview/changelog.gz",
        "usr/share/man/man1/occluview.1.gz",
        "usr/share/man/man1/occluview-cli.1.gz",
    ] {
        assert!(
            check_script.contains(required_path),
            "Debian package check should assert {required_path}"
        );
    }

    let copyright = std::fs::read_to_string(linux.join("copyright"))
        .expect("Debian copyright file should be readable");
    assert!(copyright.contains("License: Apache-2.0"));
    assert!(copyright.contains("/usr/share/common-licenses/Apache-2.0"));
    assert!(!copyright.contains("TERMS AND CONDITIONS"));
}

#[test]
fn gui_windows_resource_is_embedded_during_cross_builds() {
    let build_rs = include_str!("../../../occluview-app/build.rs");

    assert!(build_rs.contains("CARGO_CFG_WINDOWS"));
    assert!(build_rs.contains("llvm-rc"));
    assert!(build_rs.contains("cargo:rustc-link-arg-bin=occluview="));
    assert!(!build_rs.contains("env::consts::OS != \"windows\""));
}

#[test]
fn the_preview_window_and_the_com_object_die_together() {
    let preview = include_str!("../com/preview.rs");
    let window = include_str!("../com/preview/window.rs");

    let drop_impl = preview
        .split_once("impl Drop for PreviewHandler {")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let drop_body = drop_impl
        .split_once("\n}")
        .map(|(body, _)| body)
        .unwrap_or_default();
    assert!(drop_body.contains("self.destroy_preview_window();"));
    let destroys_before_count = drop_body
        .find("destroy_preview_window")
        .zip(drop_body.find("ACTIVE_COM_OBJECTS"))
        .is_some_and(|(window, count)| window < count);
    assert!(
        destroys_before_count,
        "the window must be torn down before the object count drops"
    );

    let destroy = preview
        .split_once("fn destroy_preview_window(&self)")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let clears = destroy.find("SetWindowLongPtrW");
    let destroys = destroy.find("DestroyWindow(hwnd)");
    assert!(clears
        .zip(destroys)
        .is_some_and(|(clear, destroy)| clear < destroy));
    assert!(destroy.contains("DeleteObject"));
    assert!(window.contains("WM_NCDESTROY"));

    let menu = include_str!("../com/preview/context_menu.rs");
    let after_tracking = menu
        .split_once("TrackPopupMenuEx(menu,")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let confirms = after_tracking.find("window_owns_handler(hwnd, std::ptr::from_ref(self))");
    let runs = after_tracking.find("self.run_menu_command(hwnd, command)");
    assert!(confirms
        .zip(runs)
        .is_some_and(|(confirm, run)| confirm < run));

    let smoke = include_str!("../../../../install/test-preview-handler.ps1");
    assert!(smoke.contains("Release without Unload left the child preview window alive"));
}

#[test]
fn diagnostic_events_are_fixed_field_and_cover_both_shell_components() {
    use crate::shell_diagnostics::{
        ShellDiagnosticAdapter, ShellDiagnosticComponent, ShellDiagnosticEvent,
        ShellDiagnosticEventInput, ShellDiagnosticOutcome, ShellDiagnosticProcess,
        ShellDiagnosticStage,
    };

    let preview = ShellDiagnosticEvent::normal(
        ShellDiagnosticEventInput {
            component: ShellDiagnosticComponent::Preview,
            stage: ShellDiagnosticStage::BitmapPublish,
            adapter: ShellDiagnosticAdapter::Hardware,
            elapsed_ms: 18,
        },
        ShellDiagnosticOutcome::Completed,
        ShellDiagnosticProcess {
            timestamp_unix_ms: 1_725_000_001,
            process_id: 42,
        },
    )
    .json_line();
    for expected in [
        "\"component\":\"preview\"",
        "\"stage\":\"bitmap_publish\"",
        "\"outcome\":\"completed\"",
        "\"adapter\":\"hardware\"",
        "\"elapsed_ms\":18",
    ] {
        assert!(preview.contains(expected));
    }
    for forbidden in ["path", "filename", "driver", "error.to_string"] {
        assert!(
            !preview.contains(forbidden),
            "diagnostic payload must not retain private or unstable data: {forbidden}"
        );
    }
}

#[cfg(windows)]
#[test]
fn com_entry_returns_the_fallback_when_the_body_panics() {
    let value = crate::com::com_entry("test::body_returns", || 0_u32, || 7);
    assert_eq!(value, 7, "a body that returns must pass its value through");

    let caught = crate::com::com_entry("test::body_panics", || 0_u32, || panic!("boom"));
    assert_eq!(
        caught, 0,
        "a panicking body must come back as the fallback, not unwind into the COM caller"
    );
}

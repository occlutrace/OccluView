use std::path::{Path, PathBuf};

fn registration_source() -> String {
    [
        include_str!("registration/mod.rs"),
        include_str!("registration/associations.rs"),
        include_str!("registration/clsid.rs"),
        include_str!("registration/paths.rs"),
        include_str!("registration/registry.rs"),
    ]
    .join("\n")
}

fn combined_com_source() -> String {
    [
        include_str!("com.rs"),
        include_str!("com/preview.rs"),
        include_str!("com/preview/theme.rs"),
        include_str!("com/preview/window.rs"),
        include_str!("com/preview/context_menu.rs"),
    ]
    .join("\n")
}

fn source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn preview_scene_is_split_by_responsibility_not_single_file() {
    let facade_source = source_file("src/preview_scene/mod.rs");
    let facade = facade_source
        .split_once("\n#[cfg(test)]\nmod tests")
        .map_or(facade_source.as_str(), |(source, _)| source);
    let loading = source_file("src/preview_scene/load.rs");
    let rendering = source_file("src/preview_scene/render.rs");
    let interaction = source_file("src/preview_scene/interaction.rs");
    let test_support = source_file("src/preview_scene/test_support.rs");

    assert!(
        facade.contains("mod interaction;")
            && facade.contains("mod load;")
            && facade.contains("mod render;"),
        "preview scene should be a private module directory split by loading, rendering, and interaction"
    );
    assert!(
        facade.contains("pub(crate) struct PreviewSceneState")
            && facade.contains("pub(crate) use interaction::win32_preview_orbit_delta;"),
        "preview scene facade should keep the COM-facing API stable"
    );
    assert!(
        !facade.contains("fn load_preview_mesh_from_file(")
            && !facade.contains("fn render_rgba_with_background(")
            && !facade.contains("fn viewport_ray("),
        "preview scene facade should not absorb loading, rendering, or interaction implementation"
    );
    assert!(
        loading.contains("fn load_preview_mesh_from_file(")
            && rendering.contains("fn render_rgba_with_background(")
            && interaction.contains("fn viewport_ray(")
            && test_support.contains("fn binary_stl_triangle("),
        "preview scene responsibilities should live in focused modules"
    );
}

#[test]
fn shell_extension_registers_preview_handler_for_explorer_preview_pane() {
    let shell_contract = include_str!("shell_contract.rs");
    let com = combined_com_source();
    let registration = registration_source();
    let wxs = include_str!("../../../install/occluview.wxs");
    let reg = include_str!("../../../install/occluview-shell-registration.reg");
    let smoke = include_str!("../../../install/test-msi-lifecycle.ps1");

    assert!(shell_contract.contains("PREVIEW_HANDLER_CATEGORY"));
    assert!(shell_contract.contains("{8895B1C6-B41F-4C1C-A562-0D564250836F}"));

    assert!(com.contains("OCCLUVIEW_PREVIEW_CLSID"));
    assert!(com.contains("pub struct PreviewHandler"));
    assert!(com.contains("IPreviewHandler"));
    assert!(com.contains("impl IPreviewHandler_Impl for PreviewHandler_Impl"));
    assert!(com.contains("impl IInitializeWithStream_Impl for PreviewHandler_Impl"));
    assert!(com.contains("impl IInitializeWithFile_Impl for PreviewHandler_Impl"));
    assert!(com.contains("impl IInitializeWithItem_Impl for PreviewHandler_Impl"));
    assert!(com.contains("IOleWindow"));
    assert!(com.contains("impl IOleWindow_Impl for PreviewHandler_Impl"));
    assert!(com.contains("IObjectWithSite"));
    assert!(com.contains("impl IObjectWithSite_Impl for PreviewHandler_Impl"));
    assert!(com.contains("SetParent(preview, hwnd)"));
    assert!(com.contains("SetKeyboardFocus"));
    assert!(com.contains("GetKeyboardFocus()"));
    assert!(com.contains("Err(e_fail())"));
    assert!(com.contains("Err(e_notimpl())"));
    assert!(com.contains("Err(s_false())"));
    assert!(com.contains("clear_loaded_content"));
    assert!(com.contains("ACTIVE_COM_OBJECTS"));
    assert!(com.contains("SERVER_LOCKS"));
    assert!(com.contains("CreateWindowExW"));
    assert!(com.contains("preview_render_to_hbitmap"));
    assert!(com.contains("PreviewSceneState"));
    assert!(com.contains("preview_window_proc"));
    assert!(com.contains("WM_MOUSEMOVE"));
    assert!(com.contains("WM_MOUSEWHEEL"));
    assert!(com.contains("WM_RBUTTONDOWN"));
    assert!(com.contains("WM_MBUTTONDOWN"));
    assert!(com.contains("render_preview_now"));
    assert!(com.contains("pending_stream"));

    assert!(registration.contains("register_preview_handler_clsid"));
    assert!(registration.contains("register_preview_handlers_list"));
    assert!(registration.contains("register_progid_preview_handler"));
    assert!(registration.contains("PREVIEW_HANDLER_CATEGORY"));
    assert!(registration.contains("PreviewHandlers"));
    assert!(registration.contains("OCCLUVIEW_PREVIEW_CLSID"));
    assert!(registration.contains("PREVHOST_APPID"));
    assert!(registration.contains("set_string(hk, Some(h!(\"AppID\")), PREVHOST_APPID)?;"));

    assert!(wxs.contains(
        "<?define PreviewHandlerCategory = \"{8895B1C6-B41F-4C1C-A562-0D564250836F}\" ?>"
    ));
    assert!(wxs.contains("<?define PreviewClsid = "));
    assert!(wxs.contains("<?define PrevhostAppId = "));
    assert!(wxs.contains("OccluView Preview Handler"));
    assert!(wxs.contains("Name=\"AppID\" Type=\"string\" Value=\"$(var.PrevhostAppId)\""));
    assert!(wxs.contains("Software\\Microsoft\\Windows\\CurrentVersion\\PreviewHandlers"));
    assert!(wxs.contains("ShellEx\\$(var.PreviewHandlerCategory)"));

    assert!(reg.contains("OccluView Preview Handler"));
    assert!(reg.contains("\"AppID\"=\"{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}\""));
    assert!(reg.contains("PreviewHandlers"));
    assert!(reg.contains("ShellEx\\{8895B1C6-B41F-4C1C-A562-0D564250836F}"));
    assert!(smoke.contains("$previewCategory"));
    assert!(smoke.contains("$previewClsid"));
    assert!(smoke.contains("preview CLSID AppID"));
    assert!(smoke.contains("test-preview-handler.ps1"));
    assert!(smoke.contains("Assert-NoInstalledProducts"));
}

#[test]
fn thumbnail_stream_reserves_capacity_before_copying_shell_bytes() {
    let com = include_str!("com.rs");
    let start = com
        .find("fn render_pixels(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_pixels");
    let body = &com[start..];
    let reserve = body
        .find("reserve_thumbnail_stream_job(DEFAULT_THUMBNAIL_TIMEOUT)")
        .expect("stream reservation");
    let read = body
        .find("self.ensure_stream_bytes()")
        .expect("shell stream read");
    let reserved_render = body
        .find("render_thumbnail_shared_or_placeholder_with_reservation(")
        .expect("reserved render path");

    assert!(
        reserve < read,
        "stream bytes must not be copied before budgeting"
    );
    assert!(
        read < reserved_render,
        "the reservation must follow the bytes into the worker"
    );
}

#[test]
fn thumbnail_provider_releases_full_stream_bytes_after_each_request() {
    let com = include_str!("com.rs");
    let start = com
        .find("fn render_pixels(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_pixels");
    let body = &com[start..];
    let guard = body
        .find("ThumbnailStreamBytesGuard::new(&self.bytes)")
        .expect("stream byte release guard");
    let read = body
        .find("self.ensure_stream_bytes()")
        .expect("shell stream read");

    assert!(
        guard < read,
        "stream byte ownership must be guarded before copying"
    );
    assert!(
        com.contains("impl Drop for ThumbnailStreamBytesGuard<'_>")
            && com.contains("Arc::<[u8]>::from([])"),
        "the request guard must release the provider's retained full-file buffer"
    );
}

#[test]
fn preview_pane_has_a_native_right_click_context_menu() {
    let com = combined_com_source();

    // The right-click hook only opens the menu on a stationary click, so a
    // right-*drag* still orbits the camera.
    assert!(
        com.contains("WM_RBUTTONUP") && com.contains("show_context_menu(hwnd, point)"),
        "a stationary right-click should open the context menu"
    );
    assert!(
        com.contains("let dragged = handler.drag_moved.get();"),
        "the menu must not steal a right-drag orbit"
    );

    // Native Win32 popup with per-item bitmap icons.
    assert!(com.contains("CreatePopupMenu"));
    assert!(com.contains("TrackPopupMenuEx"));
    assert!(com.contains("InsertMenuItemW"));
    assert!(com.contains("SetMenuDefaultItem"));
    assert!(com.contains("hbmpItem: bitmap"));
    assert!(com.contains("menu_icon_hbitmap"));
    assert!(
        com.contains("MFS_CHECKED"),
        "wireframe item reflects live state"
    );

    // Command dispatch covers launch, view presets, fit, wireframe, copy.
    assert!(com.contains("PreviewMenuCommand"));
    assert!(com.contains("ShellExecuteW"), "Open/Edit launch the app");
    assert!(com.contains("apply_view_preset"));
    assert!(com.contains("fit_view"));
    assert!(com.contains("set_wireframe"));
    assert!(com.contains("SetClipboardData"), "Copy image writes CF_DIB");
    assert!(com.contains("CF_DIB"));

    // Keyboard niceties (F = fit, W = wireframe).
    assert!(com.contains("WM_KEYDOWN"));
    assert!(com.contains("key_fit_view") && com.contains("key_toggle_wireframe"));

    // App-exe resolution reuses the DLL-sibling convention (no hard-coded path).
    assert!(com.contains("GetModuleFileNameW") && com.contains("APP_EXE_NAME"));
}

#[test]
fn preview_smoke_runs_preview_handler_inside_sta_and_checks_resize() {
    let smoke = include_str!("../../../install/test-preview-handler.ps1");

    assert!(smoke.contains("ApartmentState.STA"));
    assert!(smoke.contains("Type.GetTypeFromCLSID"));
    assert!(smoke.contains("IInitializeWithFile"));
    assert!(smoke.contains("IInitializeWithStream"));
    assert!(smoke.contains("IInitializeWithItem"));
    assert!(smoke.contains("IShellItem"));
    assert!(smoke.contains("IPreviewHandler"));
    assert!(smoke.contains("int TranslateAccelerator(ref MSG pmsg);"));
    assert!(smoke.contains("void Unload();"));
    assert!(smoke.contains("public struct POINT"));
    assert!(smoke.contains("CreateWindowExW"));
    assert!(smoke.contains("SHCreateStreamOnFileEx"));
    assert!(smoke.contains("SHCreateShellItemFromParsingName"));
    assert!(smoke.contains("WS_POPUP | WS_VISIBLE"));
    assert!(smoke.contains("ShowWindow(parent, SW_SHOWNOACTIVATE)"));
    assert!(smoke.contains("FindWindowExW"));
    assert!(smoke.contains("OccluViewPreviewPane"));
    assert!(smoke.contains("GetClassNameW"));
    assert!(smoke.contains("UpdateWindow"));
    assert!(smoke.contains("preview.SetRect(ref resizedRect);"));
    assert!(!smoke.contains("STM_GETIMAGE"));
    assert!(smoke.contains("SendMessageW"));
    assert!(smoke.contains("WM_RBUTTONDOWN"));
    assert!(smoke.contains("WM_MOUSEWHEEL"));
    assert!(smoke.contains("CaptureFrame"));
    assert!(smoke.contains("FramesDiffer"));
    assert!(smoke.contains("VisiblePixels"));
    assert!(smoke.contains("OrbitPreview"));
    assert!(smoke.contains("ZoomPreview"));
    assert!(!smoke.contains("bitmap mismatch"));
    assert!(smoke.contains("preview.Unload();"));
    assert!(smoke.contains("Preview handler left the child preview window alive after Unload."));
    assert!(
        smoke.contains("useStream") && smoke.contains("ProbeFromItem"),
        "preview smoke should execute file, stream, and shell-item initialization paths"
    );
    let offset = |needle: &str| {
        let pos = smoke.find(needle);
        assert!(pos.is_some(), "missing preview ABI marker: {needle}");
        pos.unwrap_or_default()
    };
    let do_preview = offset("void DoPreview();");
    let unload = offset("void Unload();");
    let set_focus = offset("void SetFocus();");
    let query_focus = offset("IntPtr QueryFocus();");
    let translate = offset("int TranslateAccelerator(ref MSG pmsg);");
    let resize = offset("preview.SetRect(ref resizedRect);");
    let update_after_resize = smoke[resize..].find("UpdateWindow(child)");
    assert!(
        update_after_resize.is_some(),
        "missing UpdateWindow(child) call after preview resize"
    );
    assert!(
        smoke.contains("preview.SetFocus();"),
        "preview smoke should exercise SetFocus at runtime"
    );
    assert!(
        smoke.contains("var focused = preview.QueryFocus();"),
        "preview smoke should exercise QueryFocus at runtime"
    );
    assert!(
        smoke.contains("int translateResult = preview.TranslateAccelerator(ref accelerator);"),
        "preview smoke should exercise TranslateAccelerator at runtime"
    );
    assert!(do_preview < unload);
    assert!(unload < set_focus);
    assert!(set_focus < query_focus);
    assert!(query_focus < translate);
}

#[test]
fn com_lazy_stream_paths_release_source_borrow_before_rendering() {
    let com = combined_com_source();

    assert!(com.contains("let source_path = self.source.borrow().path().map(PathBuf::from);"));
    assert!(!com.contains("if let Some(path) = self.source.borrow().path().map(PathBuf::from)"));
}

#[test]
fn preview_render_forces_paint_after_bitmap_refresh() {
    let com = combined_com_source();
    let start = com
        .find("fn render_preview_now(&self)")
        .expect("missing render_preview_now");
    let end = com[start..]
        .find("fn replace_preview_bitmap(&self")
        .expect("missing replace_preview_bitmap after render_preview_now");
    let render_now = &com[start..start + end];

    assert!(
            render_now.contains("RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_UPDATENOW)"),
            "preview render should synchronously invalidate and paint after resize/interaction so preview captures are never a blank host background"
        );
}

#[test]
fn linux_host_has_windows_msvc_build_script() {
    let script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-windows-msvc.sh");
    assert!(script_path.exists());

    let script = include_str!("../../../scripts/build-windows-msvc.sh");
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
    // The shell DLL lives inside Explorer's dllhost.exe: the release profile's
    // panic = "abort" would kill the surrogate on any panic and blank every
    // thumbnail in the folder. The script must build it with release-unwind,
    // matching install/build-msi.ps1.
    assert!(script.contains("--profile release-unwind"));
    assert!(!script.contains("-p occluview-cli"));
    assert!(!script.contains("occluview-cli.exe"));
}

#[test]
fn linux_install_assets_cover_freedesktop_and_deb_packaging() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let linux = repo.join("install/linux");

    assert!(linux.join("ai.occlutrace.OccluView.desktop").exists());
    assert!(linux.join("ai.occlutrace.OccluView.metainfo.xml").exists());
    assert!(linux.join("ai.occlutrace.OccluView.thumbnailer").exists());
    assert!(linux.join("occluview-mime.xml").exists());
    assert!(linux.join("build-deb.sh").exists());
    assert!(linux.join("check-deb.sh").exists());
    assert!(linux.join("copyright").exists());

    let desktop = std::fs::read_to_string(linux.join("ai.occlutrace.OccluView.desktop"))
        .expect("desktop file should be readable");
    assert!(desktop.contains("Exec=occluview %F"));
    assert!(desktop.contains("MimeType=model/stl;model/obj;model/gltf-binary;"));

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
        "usr/share/doc/occluview/README.md",
        "usr/share/doc/occluview/copyright",
        "usr/share/doc/occluview/changelog.gz",
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
    let build_rs = include_str!("../../occluview-app/build.rs");

    assert!(build_rs.contains("CARGO_CFG_WINDOWS"));
    assert!(build_rs.contains("llvm-rc"));
    assert!(build_rs.contains("cargo:rustc-link-arg-bin=occluview="));
    assert!(!build_rs.contains("env::consts::OS != \"windows\""));
}

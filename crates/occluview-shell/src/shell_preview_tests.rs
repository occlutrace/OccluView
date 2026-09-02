//! Contract tests for the Explorer preview handler and its window.

#![allow(clippy::panic, clippy::expect_used)]

use std::path::PathBuf;

#[path = "shell_preview_tests/platform_contracts.rs"]
mod platform_contracts;

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

/// A source file of this crate, read for a contract assertion.
///
/// It panics rather than returning an empty string. A path that stops
/// resolving -- a rename, a move, a typo -- would otherwise turn every
/// assertion about that file into an assertion about "", and the negative
/// ones, which are the assertions worth having, all pass in a vacuum.
fn source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "contract test source {} is missing: {error}",
            path.display()
        )
    })
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
    assert!(com.contains("SetParent(preview, Some(hwnd))"));
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
    assert!(
        !registration.contains("register_preview_handler_appid"),
        "the working main registration must use Windows' standard Prevhost AppID, not an owned surrogate"
    );

    assert!(wxs.contains(
        "<?define PreviewHandlerCategory = \"{8895B1C6-B41F-4C1C-A562-0D564250836F}\" ?>"
    ));
    assert!(wxs.contains("<?define PreviewClsid = "));
    assert!(wxs.contains("<?define PrevhostAppId = "));
    assert!(wxs.contains("{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}"));
    assert!(
        !wxs.contains("cmpPreviewHostRegistration"),
        "the MSI must not create a private Prevhost AppID component"
    );
    assert!(wxs.contains("OccluView Preview Handler"));
    assert!(wxs.contains("Name=\"AppID\" Type=\"string\" Value=\"$(var.PrevhostAppId)\""));
    assert!(wxs.contains("Software\\Microsoft\\Windows\\CurrentVersion\\PreviewHandlers"));
    assert!(wxs.contains("ShellEx\\$(var.PreviewHandlerCategory)"));

    assert!(reg.contains("OccluView Preview Handler"));
    assert!(reg.contains("\"AppID\"=\"{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}\""));
    assert!(!reg.contains("{FD67C578-DBCC-4E10-8E47-63A8E48F7654}"));
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
    let com = include_str!("com/thumbnail_provider.rs");
    let start = com
        .find("fn render_attempt(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_attempt");
    let body = &com[start..];
    let reserve = body
        .find("reserve_thumbnail_stream_job_for_request(request)")
        .expect("stream reservation");
    let read = body
        .find("self.ensure_stream_bytes(request)")
        .expect("shell stream read");
    let reserved_render = body
        .find("try_render_thumbnail_shared_with_reservation(")
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
    let com = include_str!("com/thumbnail_provider.rs");
    let start = com
        .find("fn render_attempt(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_attempt");
    let body = &com[start..];
    let guard = body
        .find("ThumbnailStreamBytesGuard::new(&self.bytes)")
        .expect("stream byte release guard");
    let read = body
        .find("self.ensure_stream_bytes(request)")
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

fn assert_preview_smoke_abi(smoke: &str) {
    assert!(smoke.contains("ApartmentState.STA"));
    assert!(smoke.contains("CoCreateInstance"));
    assert!(smoke.contains("CLSCTX_LOCAL_SERVER = 0x4"));
    assert!(smoke.contains("CLSCTX_INPROC_SERVER = 0x1"));
    assert!(smoke.contains("CreateLocalServerPreviewHandler"));
    assert!(smoke.contains("CreateInProcessPreviewHandler"));
    assert!(smoke.contains("JoinOrThrow(thread, \"Prevhost preview\")"));
    assert!(smoke.contains("JoinOrThrow(thread, \"preview\")"));
    assert!(smoke.contains("JoinOrThrow(thread, \"shell-item preview\")"));
    assert!(smoke.contains("Marshal.GetObjectForIUnknown"));
    assert!(smoke.contains("Marshal.Release(unknown)"));
    assert!(!smoke.contains("Activator.CreateInstance"));
    assert!(!smoke.contains("Type.GetTypeFromCLSID"));
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
    assert!(smoke.contains("WaitForVisibleFrame"));
    assert!(smoke.contains("WaitForChangedFrame"));
    assert!(smoke.contains("PumpMessages"));
    assert!(smoke.contains("PeekMessageW"));
    assert!(smoke.contains("GetWindowThreadProcessId"));
    assert!(smoke.contains("EnsurePreviewHostProcess(child)"));
    assert!(smoke.contains("PREVIEW_HOST_PID="));
    assert!(smoke.contains("ProbePrevhost"));
    assert!(smoke.contains("WaitForPreviewChild"));
    assert!(
        smoke.contains("private static void PumpMessages(IntPtr hwnd)"),
        "the smoke message pump must receive the preview child handle explicitly"
    );
    assert!(
        smoke.contains("PeekMessageW(out message, hwnd"),
        "the smoke pump must service only the test preview child, not consume unrelated STA messages"
    );
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
}

fn preview_smoke_offset(smoke: &str, needle: &str) -> usize {
    let position = smoke.find(needle);
    assert!(position.is_some(), "missing preview ABI marker: {needle}");
    position.unwrap_or_default()
}

fn assert_preview_smoke_interaction_abi_order(smoke: &str) {
    let do_preview = preview_smoke_offset(smoke, "void DoPreview();");
    let unload = preview_smoke_offset(smoke, "void Unload();");
    let set_focus = preview_smoke_offset(smoke, "void SetFocus();");
    let query_focus = preview_smoke_offset(smoke, "IntPtr QueryFocus();");
    let translate = preview_smoke_offset(smoke, "int TranslateAccelerator(ref MSG pmsg);");
    let resize = preview_smoke_offset(smoke, "preview.SetRect(ref resizedRect);");
    assert!(
        smoke[resize..].contains("WaitForVisibleFrame(child, \"initial resized preview frame\")"),
        "the asynchronous Preview Handler smoke must wait for the resized frame instead of capturing it immediately"
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

fn assert_prevhost_preview_contract(smoke: &str) {
    let prevhost_start = preview_smoke_offset(smoke, "public static string ProbePrevhost(");
    let prevhost_end = preview_smoke_offset(smoke, "public static string Probe(");
    let prevhost = &smoke[prevhost_start..prevhost_end];
    for required in [
        "CreateLocalServerPreviewHandler(previewClsid)",
        "IInitializeWithFile",
        "preview.DoPreview();",
        "FindWindowExW(parent, IntPtr.Zero, PreviewChildClass, null)",
        "if (child == IntPtr.Zero)",
        "EnsurePreviewHostProcess(child);",
        "var initialFrame = CaptureFrame(child);",
        "EnsureFrameVisible(initialFrame, frameDescription);",
        "preview.Unload();",
        "if (IsWindow(child))",
    ] {
        assert!(
            prevhost.contains(required),
            "Prevhost first-frame probe missing {required}"
        );
    }
    assert!(
        prevhost.contains("\"Prevhost file first frame\""),
        "the file probe must identify its own captured first frame"
    );
    for forbidden in [
        "WaitForPreviewChild",
        "WaitForVisibleFrame",
        "WaitForVisibleFrame(child",
        "WaitForChangedFrame",
        "PumpMessages",
        "UpdateWindow(",
        "Thread.Sleep",
    ] {
        assert!(
            !prevhost.contains(forbidden),
            "Prevhost first-frame probe must not wait for deferred work: {forbidden}"
        );
    }
}

#[test]
fn prevhost_smoke_exercises_the_stream_contract_used_by_explorer() {
    let smoke = include_str!("../../../install/test-preview-handler.ps1");
    let stream_start = preview_smoke_offset(smoke, "public static string ProbePrevhostStream(");
    let stream_end = preview_smoke_offset(smoke, "public static string Probe(");
    let stream_probe = &smoke[stream_start..stream_end];

    for required in [
        "CreateLocalServerPreviewHandler(previewClsid)",
        "SHCreateStreamOnFileEx(path",
        "((IInitializeWithStream)instance).Initialize(stream, 0);",
        "preview.DoPreview();",
        "EnsurePreviewHostProcess(child);",
        "EnsureFrameVisible(initialFrame, frameDescription);",
        "preview.Unload();",
    ] {
        assert!(
            stream_probe.contains(required),
            "Prevhost stream probe missing {required}"
        );
    }
    assert!(
        stream_probe.contains("\"Prevhost stream first frame\""),
        "the stream probe must identify its own captured first frame"
    );
    assert!(
        smoke
            .contains("$prevhostStreamResult = [OccluViewShellPreviewSmoke]::ProbePrevhostStream("),
        "the lifecycle smoke must execute the cross-Prevhost stream probe"
    );
}

fn assert_in_process_preview_contract(smoke: &str) {
    let detailed_start = preview_smoke_offset(smoke, "public static string Probe(");
    let detailed_end = preview_smoke_offset(smoke, "public static string ProbeFromItem(");
    let detailed_probe = &smoke[detailed_start..detailed_end];
    assert!(
        detailed_probe.contains("CreateInProcessPreviewHandler(previewClsid)"),
        "the detailed render and interaction contract must run in-process"
    );
    assert!(
        !detailed_probe.contains("CreateLocalServerPreviewHandler(previewClsid)"),
        "interaction coverage is separate from the Prevhost first-frame contract"
    );
    assert!(
        !detailed_probe.contains("EnsurePreviewHostProcess(child)"),
        "the detailed in-process contract must not assert that its own child belongs to Prevhost"
    );
    let item_start = preview_smoke_offset(smoke, "public static string ProbeFromItem(");
    let item_end = preview_smoke_offset(smoke, "private static IntPtr WaitForPreviewChild(");
    let item_probe = &smoke[item_start..item_end];
    assert!(
        item_probe.contains("CreateInProcessPreviewHandler(previewClsid)"),
        "shell-item rendering must use the same intentional in-process contract"
    );
    assert!(
        !item_probe.contains("CreateLocalServerPreviewHandler(previewClsid)"),
        "shell-item rendering must not drive a Prevhost child"
    );
    assert!(
        !item_probe.contains("EnsurePreviewHostProcess(child)"),
        "the shell-item in-process contract must not assert that its own child belongs to Prevhost"
    );
}

fn assert_prevhost_runs_before_interaction(smoke: &str) {
    let prevhost_call = preview_smoke_offset(
        smoke,
        "$prevhostFileResult = [OccluViewShellPreviewSmoke]::ProbePrevhost(",
    );
    let file_call =
        preview_smoke_offset(smoke, "$fileResult = [OccluViewShellPreviewSmoke]::Probe(");
    assert!(
        prevhost_call < file_call,
        "the Prevhost first-frame probe must run before detailed in-process rendering"
    );
}

#[test]
fn preview_smokes_prevhost_first_frame_before_in_process_interaction() {
    let smoke = include_str!("../../../install/test-preview-handler.ps1");
    assert_preview_smoke_abi(smoke);
    assert_preview_smoke_interaction_abi_order(smoke);
    assert_prevhost_preview_contract(smoke);
    assert_in_process_preview_contract(smoke);
    assert_prevhost_runs_before_interaction(smoke);
}

#[test]
fn preview_pane_returns_only_after_the_first_frame_is_painted() {
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");
    let factory = include_str!("offscreen_factory.rs");

    let render_start = preview
        .find("fn render_preview_now(&self) -> windows::core::Result<()>")
        .expect("synchronous preview renderer");
    let render_end = preview[render_start..]
        .find("fn replace_preview_bitmap(&self")
        .map(|offset| render_start + offset)
        .expect("bitmap replacement follows the synchronous renderer");
    let render = &preview[render_start..render_end];
    assert!(
        render.contains("self.preview_render_to_hbitmap(width, height)?")
            && render.contains("self.replace_preview_bitmap(hbmp)")
            && render
                .contains("RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW)"),
        "DoPreview must render, publish, and paint the first bitmap before returning success"
    );

    let do_preview_start = preview
        .find("fn DoPreview(")
        .expect("DoPreview implementation");
    let do_preview_end = preview[do_preview_start..]
        .find("fn Unload(")
        .map(|offset| do_preview_start + offset)
        .expect("Unload follows DoPreview");
    let do_preview = &preview[do_preview_start..do_preview_end];
    assert!(
        do_preview.contains("let _ = self.this.ensure_preview_window()?;")
            && do_preview.contains("self.this.render_preview_now()")
            && !do_preview.contains("RenderDeadline"),
        "DoPreview must not defer, deadline, or wait for its first frame"
    );
    assert!(
        window.contains("let _ = handler.render_preview_now();"),
        "WM_SIZE must use the same immediate preview renderer"
    );
    assert!(
        !factory.contains("RenderDeadline")
            && !factory.contains("Condvar")
            && !factory.contains("wait_timeout"),
        "the Preview Pane renderer must not inherit deadline-based startup waiting"
    );
}

#[test]
fn com_lazy_stream_paths_release_source_borrow_before_rendering() {
    let com = combined_com_source();

    assert!(com.contains("let source_path = self.source.borrow().path().map(PathBuf::from);"));
    assert!(!com.contains("if let Some(path) = self.source.borrow().path().map(PathBuf::from)"));
}

#[test]
fn every_com_boundary_is_panic_guarded() {
    // Rust aborts the process when a panic unwinds out of an `extern "system"`
    // fn — the ABI of every #[implement] vtable shim, the Dll* exports, and
    // the wndproc — regardless of the unwind profile the DLL builds with. In a
    // shared surrogate that abort blanks every other file's thumbnail or
    // preview in flight, so each boundary must catch via `com_entry`.
    // Whitespace-normalize so rustfmt's argument wrapping cannot break the
    // assertions; the guard is the call plus its context literal.
    let flatten = |source: &str| source.split_whitespace().collect::<String>();
    let com = flatten(concat!(
        include_str!("com.rs"),
        include_str!("com/thumbnail_provider.rs")
    ));
    let preview = flatten(include_str!("com/preview.rs"));
    let window = flatten(include_str!("com/preview/window.rs"));
    let registration = flatten(include_str!("registration/mod.rs"));

    for guarded in [
        "com_entry(\"IThumbnailProvider::GetThumbnail\"",
        "com_entry(\"thumbnailIInitializeWithStream\"",
        "com_entry(\"thumbnailIInitializeWithFile\"",
        "com_entry(\"thumbnailIInitializeWithItem\"",
        "com_entry(\"thumbnailIClassFactory::CreateInstance\"",
        "com_entry(\"previewIClassFactory::CreateInstance\"",
        "com_entry(\"DllGetClassObject\"",
    ] {
        assert!(com.contains(guarded), "missing panic guard: {guarded}");
    }
    for guarded in [
        "com_entry(\"IPreviewHandler::SetWindow\"",
        "com_entry(\"IPreviewHandler::SetRect\"",
        "com_entry(\"IPreviewHandler::DoPreview\"",
        "com_entry(\"IPreviewHandler::Unload\"",
        "com_entry(\"IObjectWithSite::SetSite\"",
        "com_entry(\"IObjectWithSite::GetSite\"",
        "com_entry(\"previewIInitializeWithStream\"",
        "com_entry(\"previewIInitializeWithFile\"",
        "com_entry(\"previewIInitializeWithItem\"",
    ] {
        assert!(preview.contains(guarded), "missing panic guard: {guarded}");
    }
    assert!(window.contains("com_entry(\"preview_window_proc\""));
    assert!(registration.contains("com_entry(\"DllRegisterServer\""));
    assert!(registration.contains("com_entry(\"DllUnregisterServer\""));
    // The HRESULT vocabulary comes from Win32::Foundation, never hand-typed
    // decimal literals (which once drifted into 0x8000FF85-style non-codes).
    assert!(!com.contains("HRESULT(-2_147_4"));
    assert!(!registration.contains("HRESULT(-2_147_4"));
}

//! Contract tests for the Explorer preview handler and its window.

#![allow(clippy::panic, clippy::expect_used)]

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
    let com = include_str!("com/thumbnail_provider.rs");
    let start = com
        .find("fn render_attempt(&self, spec: ThumbnailSpec)")
        .expect("thumbnail render_attempt");
    let body = &com[start..];
    let reserve = body
        .find("reserve_thumbnail_stream_job(DEFAULT_THUMBNAIL_TIMEOUT)")
        .expect("stream reservation");
    let read = body
        .find("self.ensure_stream_bytes()")
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
fn preview_smokes_separate_private_surrogate_liveness_from_in_process_rendering() {
    let smoke = include_str!("../../../install/test-preview-handler.ps1");

    assert!(smoke.contains("ApartmentState.STA"));
    assert!(smoke.contains("CoCreateInstance"));
    assert!(smoke.contains("CLSCTX_LOCAL_SERVER = 0x4"));
    assert!(smoke.contains("CLSCTX_INPROC_SERVER = 0x1"));
    assert!(smoke.contains("CreateLocalServerPreviewHandler"));
    assert!(smoke.contains("CreateInProcessPreviewHandler"));
    assert!(smoke.contains("JoinOrThrow(thread, \"private-surrogate preview\")"));
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
    assert!(smoke.contains("ProbePrivateSurrogate"));
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

    let private_start = offset("public static string ProbePrivateSurrogate(");
    let private_end = offset("public static string Probe(");
    let private_surrogate = &smoke[private_start..private_end];
    for required in [
        "CreateLocalServerPreviewHandler(previewClsid)",
        "IInitializeWithFile",
        "preview.DoPreview();",
        "WaitForPreviewChild(parent, \"private surrogate preview\")",
        "EnsurePreviewHostProcess(child);",
        "preview.Unload();",
        "if (IsWindow(child))",
    ] {
        assert!(
            private_surrogate.contains(required),
            "private-surrogate liveness probe missing {required}"
        );
    }
    for forbidden in [
        "GetClientRect",
        "UpdateWindow",
        "CaptureFrame",
        "BitBlt",
        "SendMessageW",
        "preview.SetRect",
        "preview.SetFocus",
    ] {
        assert!(
            !private_surrogate.contains(forbidden),
            "private-surrogate liveness probe must not drive a low-integrity child: {forbidden}"
        );
    }

    let detailed_start = offset("public static string Probe(");
    let detailed_end = offset("public static string ProbeFromItem(");
    let detailed_probe = &smoke[detailed_start..detailed_end];
    assert!(
        detailed_probe.contains("CreateInProcessPreviewHandler(previewClsid)"),
        "the detailed render and interaction contract must run in-process"
    );
    assert!(
        !detailed_probe.contains("CreateLocalServerPreviewHandler(previewClsid)"),
        "the detailed render and interaction contract must not pretend it can drive Prevhost's low-integrity child"
    );
    assert!(
        !detailed_probe.contains("EnsurePreviewHostProcess(child)"),
        "the detailed in-process contract must not assert that its own child belongs to Prevhost"
    );
    let item_start = offset("public static string ProbeFromItem(");
    let item_end = offset("private static IntPtr WaitForPreviewChild(");
    let item_probe = &smoke[item_start..item_end];
    assert!(
        item_probe.contains("CreateInProcessPreviewHandler(previewClsid)"),
        "shell-item rendering must use the same intentional in-process contract"
    );
    assert!(
        !item_probe.contains("CreateLocalServerPreviewHandler(previewClsid)"),
        "shell-item rendering must not drive a private Prevhost child"
    );
    assert!(
        !item_probe.contains("EnsurePreviewHostProcess(child)"),
        "the shell-item in-process contract must not assert that its own child belongs to Prevhost"
    );

    let private_call =
        offset("$surrogateResult = [OccluViewShellPreviewSmoke]::ProbePrivateSurrogate(");
    let file_call = offset("$fileResult = [OccluViewShellPreviewSmoke]::Probe(");
    assert!(
        private_call < file_call,
        "the private Prevhost liveness probe must run before detailed in-process rendering"
    );
}

#[test]
fn com_lazy_stream_paths_release_source_borrow_before_rendering() {
    let com = combined_com_source();

    assert!(com.contains("let source_path = self.source.borrow().path().map(PathBuf::from);"));
    assert!(!com.contains("if let Some(path) = self.source.borrow().path().map(PathBuf::from)"));
}

#[test]
fn preview_render_invalidates_without_a_synchronous_paint_round_trip() {
    let com = combined_com_source();
    let start = com
        .find("fn render_scheduled_preview(&self")
        .expect("missing deferred preview render");
    let end = com[start..]
        .find("fn replace_preview_bitmap(&self")
        .expect("missing replace_preview_bitmap after deferred preview render");
    let render_now = &com[start..start + end];

    assert!(
        render_now.contains("InvalidateRect(Some(hwnd), None, false)"),
        "the deferred render should request an ordinary later paint"
    );
    assert!(
        !render_now.contains("RDW_UPDATENOW"),
        "a Preview Handler must never synchronously paint while servicing a shell callback"
    );
}

#[test]
fn preview_reuses_one_process_shared_offscreen_renderer() {
    // prevhost keeps the handler process alive between file clicks; creating a
    // fresh wgpu device + compiling shaders per click was the dominant fixed
    // cost of first paint. The scene loader must borrow the process-shared
    // renderer, and the render path must retire it on failure so a lost device
    // heals instead of failing every later preview.
    let factory = include_str!("offscreen_factory.rs");
    let load = include_str!("preview_scene/load.rs");
    let render = include_str!("preview_scene/render.rs");

    assert!(factory.contains("fn shared_shell_offscreen()"));
    assert!(factory.contains("fn discard_shared_shell_offscreen("));
    assert!(
        !factory.contains("fn prewarm_shared_shell_offscreen()"),
        "preview renderer initialization must have exactly one deferred owner, not a competing activation prewarm"
    );
    assert!(load.contains("let offscreen = shared_shell_offscreen()?;"));
    assert!(
        !load.contains("create_shell_offscreen()"),
        "preview loads must not create a per-file wgpu device"
    );
    assert!(render.contains("discard_shared_shell_offscreen(&self.offscreen)"));
}

#[test]
fn preview_callbacks_coalesce_work_onto_a_private_window_message() {
    // MoveWindow synchronously delivers WM_SIZE when the size changed. The
    // handler must return to Explorer before it parses or renders, then make
    // one later, coalesced render request for the final geometry.
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");

    let set_rect_start = preview.find("fn SetRect(").expect("SetRect impl");
    let set_rect_end = preview[set_rect_start..]
        .find("fn DoPreview(")
        .map(|offset| set_rect_start + offset)
        .expect("DoPreview follows SetRect");
    let set_window_start = preview.find("fn SetWindow(").expect("SetWindow impl");
    let set_window_end = preview[set_window_start..]
        .find("fn SetRect(")
        .map(|offset| set_window_start + offset)
        .expect("SetRect follows SetWindow");

    assert!(
        !preview[set_rect_start..set_rect_end].contains("render_scheduled_preview"),
        "SetRect must not render synchronously"
    );
    assert!(
        !preview[set_window_start..set_window_end].contains("render_scheduled_preview"),
        "SetWindow must not render synchronously"
    );
    let wm_size = window.find("WM_SIZE =>").expect("WM_SIZE arm");
    assert!(
        window[wm_size..].contains("schedule_preview_render"),
        "WM_SIZE must schedule, rather than perform, the resize render"
    );
    let do_preview_start = preview.find("fn DoPreview(").expect("DoPreview impl");
    let do_preview_end = preview[do_preview_start..]
        .find("fn Unload(")
        .map(|offset| do_preview_start + offset)
        .expect("Unload follows DoPreview");
    assert!(
        preview[do_preview_start..do_preview_end].contains("schedule_preview_render"),
        "DoPreview must return before parsing or rendering the source"
    );
}

#[test]
fn preview_render_messages_cannot_outlive_the_window_owner() {
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");

    assert!(preview.contains("pending_render_token"));
    assert!(preview.contains("fn clear_pending_preview_render(&self)"));
    assert!(preview.contains("self.clear_pending_preview_render();"));
    let compact_preview = preview.split_whitespace().collect::<String>();
    assert!(compact_preview.contains("PostMessageW(Some(hwnd),WM_OCCLUVIEW_RENDER_PREVIEW"));
    assert!(window.contains("WM_OCCLUVIEW_RENDER_PREVIEW"));
    assert!(window.contains("render_scheduled_preview(hwnd, wparam)"));
    let nc_destroy = window.find("WM_NCDESTROY =>").expect("WM_NCDESTROY arm");
    assert!(
        window[nc_destroy..].contains("handler.clear_pending_preview_render();"),
        "a parent-driven child destruction must clear the pending token before a new window can schedule work"
    );
    assert!(
        window.contains("window_owns_handler(hwnd, std::ptr::from_ref(handler))"),
        "a deferred message must check that the current HWND still belongs to the handler before dereferencing it"
    );
    let deferred_start = preview
        .find("fn render_scheduled_preview(&self")
        .expect("missing deferred render method");
    let deferred_end = preview[deferred_start..]
        .find("fn clear_pending_preview_render(&self)")
        .map(|offset| deferred_start + offset)
        .expect("missing pending-render cleanup after deferred render method");
    let deferred = &preview[deferred_start..deferred_end];
    assert!(
        deferred.contains("self.pending_render_token.get() != Some(token)"),
        "a stale message must compare its token without clearing a newer render request"
    );
    assert!(
        deferred.contains("self.pending_render_token.set(None);"),
        "only the matching render message may consume the pending token"
    );
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

#[test]
fn class_activation_prewarms_only_the_thumbnail_renderer() {
    // Thumbnail activation is a throughput problem: it runs outside Explorer
    // and has a separate renderer. Preview activation must not race a private
    // preview message with a background device creation thread.
    let com = include_str!("com.rs");

    assert!(com.contains("fn spawn_renderer_prewarm(class: &GUID)"));
    assert!(com.contains("spawn_renderer_prewarm(&requested);"));
    assert!(com.contains("prewarm_thumbnail_renderer"));
    assert!(!com.contains("prewarm_shared_shell_offscreen"));
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
    // The launcher searches Name, GenericName and Keywords. Without keywords a
    // technician who types the file format, or the work, finds nothing.
    assert!(
        desktop.contains("Keywords=") && desktop.contains("STL;"),
        "the desktop entry must be findable by what the user is looking for"
    );

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
        // One icon name per registered type: a scan with no thumbnail yet must
        // still be drawn as a scan, the way the Windows installer draws it.
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
    let build_rs = include_str!("../../occluview-app/build.rs");

    assert!(build_rs.contains("CARGO_CFG_WINDOWS"));
    assert!(build_rs.contains("llvm-rc"));
    assert!(build_rs.contains("cargo:rustc-link-arg-bin=occluview="));
    assert!(!build_rs.contains("env::consts::OS != \"windows\""));
}

#[test]
fn the_preview_window_and_the_com_object_die_together() {
    // The child preview window holds a raw `&PreviewHandler` in GWLP_USERDATA
    // with no AddRef, so their lifetimes have to be tied by hand. Two ordinary
    // routes reach the mismatch:
    //
    //   * A host releases without calling `Unload` -- etiquette, not a COM
    //     requirement, and especially likely after `DoPreview` returned an
    //     error.
    //   * Re-entrancy with a perfectly behaved host: `show_context_menu` runs
    //     `TrackPopupMenuEx`, a modal loop that pumps the STA, so a click on
    //     another file in Explorer can deliver Unload and Release while that
    //     call is still on the stack.
    //
    // And the reverse: when Explorer destroys the parent, the child dies with
    // it and a later Unload would call DestroyWindow on a recycled handle.
    let preview = include_str!("com/preview.rs");
    let window = include_str!("com/preview/window.rs");

    let drop_impl = preview
        .split_once("impl Drop for PreviewHandler {")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let drop_body = drop_impl
        .split_once("\n}")
        .map(|(body, _)| body)
        .unwrap_or_default();
    assert!(
        drop_body.contains("self.destroy_preview_window();"),
        "dropping the COM object must take the window with it, or a live \
         window is left pointing at freed memory"
    );
    let destroys_before_count = drop_body
        .find("destroy_preview_window")
        .zip(drop_body.find("ACTIVE_COM_OBJECTS"))
        .is_some_and(|(window, count)| window < count);
    assert!(
        destroys_before_count,
        "the window must be torn down before the object count drops"
    );

    // Clearing the slot is legal cross-thread; DestroyWindow is not. An
    // orphaned window must degrade to DefWindowProcW, not to a dangling read.
    let destroy = preview
        .split_once("fn destroy_preview_window(&self)")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let clears = destroy.find("SetWindowLongPtrW");
    let destroys = destroy.find("DestroyWindow(hwnd)");
    assert!(
        clears
            .zip(destroys)
            .is_some_and(|(clear, destroy)| clear < destroy),
        "GWLP_USERDATA must be cleared before DestroyWindow, and unconditionally"
    );
    assert!(
        destroy.contains("DeleteObject"),
        "the last rendered bitmap is up to 2048x2048x4 of GDI memory and must not leak"
    );

    assert!(
        window.contains("WM_NCDESTROY"),
        "a window destroyed with its parent must clear the handler's stale HWND"
    );

    // Destroying the window narrows the second route -- the common case then
    // returns 0 from the modal call -- but cannot close it: the frame that
    // opened the menu is still on the stack when Drop runs, and running the
    // selected command from there reads the preview scene and the source
    // stream of a freed object. What closes it is refusing to touch `self`
    // once the window no longer names it.
    let menu = include_str!("com/preview/context_menu.rs");
    let after_tracking = menu
        .split_once("TrackPopupMenuEx(menu,")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let confirms = after_tracking.find("window_owns_handler(hwnd, std::ptr::from_ref(self))");
    let runs = after_tracking.find("self.run_menu_command(hwnd, command)");
    assert!(
        confirms
            .zip(runs)
            .is_some_and(|(confirm, run)| confirm < run),
        "the selected command must not run until the window has confirmed it \
         still points at this handler"
    );

    // The Windows packaging job runs this through test-msi-lifecycle.ps1, so
    // the rule is checked against a real host, not only against the source.
    let smoke = include_str!("../../../install/test-preview-handler.ps1");
    assert!(
        smoke.contains("Release without Unload left the child preview window alive"),
        "the preview smoke should cover a host that releases without Unload"
    );
}

/// The COM boundary guard has to catch, not merely be spelled at every site.
///
/// Windows-only, because the module it tests is: CI runs the workspace suite
/// on windows-latest, which is where this one counts.
///
/// The guard beside it checks that every entry point names `com_entry`. That
/// is all a Linux build can check about the call sites, and it is worth
/// having -- but it says nothing about the body. Deleting the `catch_unwind`
/// from `com_entry` left the whole shell suite green, and `[profile.release]`
/// sets `panic = "abort"` for the cdylib, so what these catches prevent is one
/// bad file taking down `dllhost` and blanking every thumbnail in the folder.
#[cfg(windows)]
#[test]
fn com_entry_returns_the_fallback_when_the_body_panics() {
    let value = crate::com::com_entry("test::body_returns", || 0_u32, || 7);
    assert_eq!(value, 7, "a body that returns must pass its value through");

    let caught = crate::com::com_entry("test::body_panics", || 0_u32, || panic!("boom"));
    assert_eq!(
        caught, 0,
        "a panicking body must come back as the fallback, not unwind into the \
         COM caller"
    );
}

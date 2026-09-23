//! Contract tests for the Explorer preview handler and its window.

#![allow(clippy::panic, clippy::expect_used)]

use std::path::PathBuf;

#[path = "shell_preview_tests/platform_contracts.rs"]
mod platform_contracts;

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
fn com_lazy_stream_paths_release_source_borrow_before_rendering() {
    let com = combined_com_source();

    assert!(com.contains("let source_path = self.source.borrow().path().map(PathBuf::from);"));
    assert!(!com.contains("if let Some(path) = self.source.borrow().path().map(PathBuf::from)"));
}

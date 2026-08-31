//! The COM `IThumbnailProvider` class.
//!
//! Windows Explorer activates this class out-of-process in a `dllhost.exe`
//! surrogate. The class is a thin stub: it stores the file/stream the shell
//! hands it at initialize time, and on `GetThumbnail` it detects the format,
//! renders the mesh, and calls the same `render_thumbnail` code path the CLI
//! uses.
//!
//! Verdict policy at the COM boundary: a broken or unsupported file returns an
//! OccluView placeholder bitmap (a stable verdict Explorer may cache), while a
//! *transient* miss — timeout, saturated queue, GPU fault, unreadable stream —
//! returns a failure `HRESULT`. Explorer's thumbcache permanently stores any
//! bitmap returned with `S_OK`, so answering "busy" with a placeholder would
//! freeze the placeholder into the file's icon until the file is modified.
//! COM ABI errors still return `E_FAIL`. We never propagate a panic across
//! the COM boundary.

// This module is the COM ABI boundary: FFI exports, raw pointer parameters,
// and windows-rs calls that are `unsafe` by definition. The rest of the crate
// stays `#![deny(unsafe_code)]`; this module gates it behind `cfg(windows)`
// (the `cfg` lives on the `pub mod com;` in lib.rs). The pedantic lints below
// are inherent to FFI/COM glue (raw pointer derefs, casts across the ABI) and
// are relaxed here only.
#![allow(
    unsafe_code,
    clippy::missing_safety_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::unnecessary_cast,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    missing_docs
)]

use crate::deferred_source::DeferredSource;
use crate::preview_scene::{win32_preview_orbit_delta, PreviewSceneState};
use crate::stream_read::{read_capped_stream, StreamRead};
use crate::ShellError;
use glam::Vec2;
use occluview_render::ThumbnailSpec;
use occluview_thumbnail::render_thumb::{
    placeholder_for_oversize_input, reserve_thumbnail_stream_job, try_render_thumbnail_file,
    try_render_thumbnail_shared_with_reservation, ThumbnailAttempt, DEFAULT_THUMBNAIL_TIMEOUT,
    MAX_THUMBNAIL_INPUT_BYTES,
};
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use windows::core::{implement, w, IUnknown, Interface, Ref, BOOL, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, HINSTANCE,
    HWND, LPARAM, LRESULT, POINT, RECT, S_FALSE, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint,
    RedrawWindow, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    HGDIOBJ, PAINTSTRUCT, RDW_INVALIDATE, RDW_UPDATENOW, SRCCOPY,
};
use windows::Win32::System::Com::STREAM_SEEK_SET;
use windows::Win32::System::Com::{
    CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream, STATFLAG, STATSTG,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};
use windows::Win32::System::Ole::{
    IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    REG_DWORD, REG_VALUE_TYPE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus as GetKeyboardFocus, ReleaseCapture, SetCapture, SetFocus as SetKeyboardFocus,
};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithFile, IInitializeWithFile_Impl, IInitializeWithStream,
    IInitializeWithStream_Impl,
};
use windows::Win32::UI::Shell::{
    IInitializeWithItem, IInitializeWithItem_Impl, IPreviewHandler, IPreviewHandler_Impl,
    IShellItem, IThumbnailProvider, IThumbnailProvider_Impl, SIGDN_FILESYSPATH, WTSAT_ARGB,
    WTS_ALPHATYPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, MoveWindow, RegisterClassW, SetParent,
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, MSG, WINDOW_EX_STYLE,
    WM_CANCELMODE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

mod thumbnail_provider;

pub use thumbnail_provider::ThumbnailProvider;

mod preview;

use preview::{PreviewHandler, PreviewHandler_Impl};

/// The OccluView thumbnail-provider CLSID.
///
/// The shell's `IThumbnailProvider` *category* CLSID is the well-known
/// `{E357FCCD-A995-4576-B01F-234630154E96}`; entries under
/// `HKCR\<ext>\ShellEx\{...}` point at this implementation CLSID via their
/// default value.
pub const OCCLUVIEW_THUMBNAIL_CLSID: &str = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3045}";
pub const OCCLUVIEW_PREVIEW_CLSID: &str = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3046}";

const OCCLUVIEW_THUMBNAIL_GUID: GUID = GUID::from_u128(0x9f3a1b2c_4d5e_4f60_8a7b_9c0d1e2f3045);
const OCCLUVIEW_PREVIEW_GUID: GUID = GUID::from_u128(0x9f3a1b2c_4d5e_4f60_8a7b_9c0d1e2f3046);
use crate::preview_canvas::center_square_on_canvas;

const MAX_OFFSCREEN_EDGE: u32 = 2048;
const PREVIEW_WINDOW_CLASS_NAME: PCWSTR = w!("OccluViewPreviewPane");
const PREVIEW_LIGHT_BACKGROUND_LINEAR: [f64; 4] = [0.80, 0.82, 0.84, 1.0];
const PREVIEW_DARK_BACKGROUND_LINEAR: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
const PREVIEW_LIGHT_CANVAS_RGBA: [u8; 4] = [204, 209, 214, 255];
const PREVIEW_DARK_CANVAS_RGBA: [u8; 4] = [0, 0, 0, 255];
const ERROR_SUCCESS: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
static ACTIVE_COM_OBJECTS: AtomicUsize = AtomicUsize::new(0);
static SERVER_LOCKS: AtomicUsize = AtomicUsize::new(0);
static PREVIEW_WINDOW_CLASS: OnceLock<Result<(), HRESULT>> = OnceLock::new();
static THUMBNAIL_RENDERER_PREWARM: OnceLock<()> = OnceLock::new();
static PREVIEW_RENDERER_PREWARM: OnceLock<()> = OnceLock::new();

/// This DLL's own module handle, pinned into the process.
///
/// Two kinds of code in this DLL outlive COM's refcount view of it: the
/// preview window class's wndproc (a raw function pointer registered with
/// USER32), and the background threads — prewarm, and render workers that
/// keep going after a caller times out. `DllCanUnloadNow` counts only live
/// COM objects, so without the pin COM could unmap the image while any of
/// those still execute inside it. A pinned module is a small, bounded cost:
/// the shell recycles its surrogate hosts anyway.
pub(super) fn own_pinned_dll_module() -> windows::core::Result<windows::Win32::Foundation::HMODULE>
{
    // An address inside our own mapped image, used to find the DLL's module.
    // Typed as `u16` so it can become a `PCWSTR` (an opaque address here,
    // never dereferenced) without an alignment-widening pointer cast.
    static ANCHOR: u16 = 0;
    let mut module = windows::Win32::Foundation::HMODULE::default();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN;
    let address = PCWSTR(core::ptr::addr_of!(ANCHOR));
    // SAFETY: `address` lies within this DLL; `module` is a valid out-param.
    unsafe { GetModuleHandleExW(flags, address, &mut module) }?;
    Ok(module)
}

/// Start renderer creation the moment the host activates one of our classes.
///
/// Both surrogate hosts activate the class well before the first heavy call
/// (`GetThumbnail` in `dllhost`, `DoPreview` in `prevhost`), and wgpu
/// instance + adapter + device + pipeline creation is a fixed cost of one to
/// several hundred milliseconds. Warming on a background thread overlaps that
/// cost with the shell's Initialize / stream-copy phase; the per-class gates
/// keep it to a single attempt per process, and only the requested class's
/// renderer warms so a thumbnail host never builds the preview device (or
/// vice versa).
fn spawn_renderer_prewarm(class: &GUID) {
    // Pin before the first background thread exists (see
    // `own_pinned_dll_module`); a failed pin only means we skip the warmup.
    if own_pinned_dll_module().is_err() {
        return;
    }
    if *class == OCCLUVIEW_THUMBNAIL_GUID {
        THUMBNAIL_RENDERER_PREWARM.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("occluview-thumbnail-prewarm".to_string())
                .spawn(occluview_thumbnail::render_thumb::prewarm_thumbnail_renderer);
        });
    } else if *class == OCCLUVIEW_PREVIEW_GUID {
        PREVIEW_RENDERER_PREWARM.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("occluview-preview-prewarm".to_string())
                .spawn(crate::offscreen_factory::prewarm_shared_shell_offscreen);
        });
    }
}

fn path_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

/// Build a 32bpp BGRA top-down `HBITMAP` from top-to-bottom RGBA8 pixels.
/// The caller owns the returned handle.
///
/// The offscreen readback already delivers top-down rows (the app viewport
/// paints them into egui untouched); flipping here again vertically MIRRORED
/// every thumbnail and the preview pane, which read as "inverted vertical
/// orbit" in the live preview. Keep this top-down end to end.
fn pixels_to_hbitmap(pixels: &[u8], width: u32, height: u32) -> windows::core::Result<HBITMAP> {
    if width == 0 || height == 0 || pixels.len() != (width * height * 4) as usize {
        return Err(e_fail());
    }
    let mut bgra = vec![0u8; pixels.len()];
    for (dst, src) in bgra.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
        dst[0] = src[2]; // B
        dst[1] = src[1]; // G
        dst[2] = src[0]; // R
        dst[3] = src[3]; // A
    }
    create_top_down_bgra_dib(width, height, &bgra)
}

/// Allocate a 32bpp top-down BGRA DIB and fill it with `bgra`.
///
/// The one place this crate calls `CreateDIBSection`. Written twice -- here
/// and for the context-menu glyphs -- both copies carry the same hand-written
/// GDI leak guard for the null-bits path, inside a DLL that lives in
/// `explorer.exe` for a whole session; fix a leak or a header field in one and
/// the other stays wrong, in a `cfg(windows)` crate no Linux gate compiles.
/// The callers differ only in how they produce the bytes: this one swizzles
/// RGBA, the glyph path premultiplies.
///
/// The caller owns the returned handle.
fn create_top_down_bgra_dib(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> windows::core::Result<HBITMAP> {
    if width == 0 || height == 0 || bgra.len() != (width * height * 4) as usize {
        return Err(e_fail());
    }
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative height = top-down DIB, matching the readback row order.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: width * height * 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: `bitmap_info` is a valid 32bpp BI_RGB DIB descriptor, `bits`
    // is an out-pointer written by GDI, and the returned handle is owned by
    // the caller.
    let hbmp = unsafe { CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    if bits.is_null() {
        // CreateDIBSection succeeded and handed us a bitmap handle even
        // though the pixel buffer pointer came back null; free it here so we
        // don't leak a GDI object on this defensive error path.
        // SAFETY: `hbmp` was just allocated by the CreateDIBSection call
        // above and is not yet owned by anyone else.
        let _ = unsafe { DeleteObject(HGDIOBJ(hbmp.0)) };
        return Err(e_fail());
    }
    // SAFETY: CreateDIBSection allocated at least width*height*4 bytes for
    // this 32bpp DIB, and `bgra` has exactly that many initialized bytes.
    unsafe { std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len()) };
    Ok(hbmp)
}

impl IClassFactory_Impl for PreviewHandler_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        com_entry(
            "preview IClassFactory::CreateInstance",
            || Err(e_fail()),
            || {
                if ppvobject.is_null() {
                    return Err(e_pointer());
                }
                if !punkouter.is_null() {
                    return Err(windows::core::Error::from_hresult(CLASS_E_NOAGGREGATION));
                }
                let provider = PreviewHandler::new();
                let unknown: IUnknown = provider.into();
                // SAFETY: `riid` and `ppvobject` are COM-supplied; `query` follows
                // the COM ABI contract for QueryInterface.
                let hr = unsafe { unknown.query(riid, ppvobject) };
                if hr.is_ok() {
                    Ok(())
                } else {
                    Err(windows::core::Error::from_hresult(hr))
                }
            },
        )
    }

    fn LockServer(&self, flock: BOOL) -> windows::core::Result<()> {
        if flock.as_bool() {
            SERVER_LOCKS.fetch_add(1, Ordering::AcqRel);
        } else {
            SERVER_LOCKS.fetch_sub(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

/// `E_FAIL` as a `windows::core::Error`.
///
/// These helpers wrap the canonical `Win32::Foundation` constants. Earlier
/// revisions hand-transcribed the decimal values and drifted into
/// `0x8000FF85`-style non-codes — still failures, but meaningless to anyone
/// reading an Explorer trace. Never write an HRESULT literal here again.
fn e_fail() -> windows::core::Error {
    windows::core::Error::from_hresult(E_FAIL)
}

/// `E_POINTER` as a `windows::core::Error`.
fn e_pointer() -> windows::core::Error {
    windows::core::Error::from_hresult(E_POINTER)
}

/// `E_NOTIMPL` as a `windows::core::Error`.
fn e_notimpl() -> windows::core::Error {
    windows::core::Error::from_hresult(E_NOTIMPL)
}

/// `S_FALSE` as a `windows::core::Error` so COM returns the non-fatal "not
/// handled" status instead of incorrectly claiming success.
fn s_false() -> windows::core::Error {
    windows::core::Error::from_hresult(S_FALSE)
}

/// Run a COM entry body, converting a panic into `fallback`.
///
/// The `#[implement]` vtable shims are `extern "system"`; Rust aborts the
/// whole process when a panic unwinds past that ABI boundary — even under the
/// unwind profile the DLL ships with — and in a shared surrogate that abort
/// blanks every other file's thumbnail or preview in flight. Catching at each
/// entry keeps one poisoned request from taking the host down.
/// `AssertUnwindSafe` is sound here: a panicking request abandons only its own
/// per-instance state, and the process-wide pool/gate statics recover poisoned
/// locks by design (see `occluview-thumbnail`'s `lock_recover`).
pub(super) fn com_entry<T>(
    context: &'static str,
    fallback: impl FnOnce() -> T,
    body: impl FnOnce() -> T,
) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_panic| {
        tracing::error!(context, "panic caught at the COM boundary");
        fallback()
    })
}

/// `DllGetClassObject` — the COM runtime calls this when our CLSID is
/// activated. Returns an `IClassFactory` for the requested shell class.
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    rclsid: *const std::ffi::c_void,
    riid: *const std::ffi::c_void,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    com_entry(
        "DllGetClassObject",
        || E_FAIL,
        || {
            if ppv.is_null() || riid.is_null() || rclsid.is_null() {
                return E_POINTER;
            }
            // SAFETY: `rclsid` is supplied by COM and points to a GUID for the
            // activation request.
            let requested = unsafe { *(rclsid as *const GUID) };
            spawn_renderer_prewarm(&requested);
            let factory: IUnknown = if requested == OCCLUVIEW_THUMBNAIL_GUID {
                ThumbnailProvider::new().into()
            } else if requested == OCCLUVIEW_PREVIEW_GUID {
                PreviewHandler::new().into()
            } else {
                // SAFETY: ppv is a caller-provided out-pointer.
                unsafe { *ppv = std::ptr::null_mut() };
                return CLASS_E_CLASSNOTAVAILABLE;
            };
            // SAFETY: caller-supplied COM pointers; query follows the ABI contract.
            let hr = unsafe { factory.query(riid as *const GUID, ppv) };
            if hr.is_ok() {
                S_OK
            } else {
                hr
            }
        },
    )
}

/// `DllCanUnloadNow` — the COM runtime asks whether this DLL can be unloaded.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if ACTIVE_COM_OBJECTS.load(Ordering::Acquire) == 0 && SERVER_LOCKS.load(Ordering::Acquire) == 0
    {
        S_OK
    } else {
        S_FALSE
    }
}

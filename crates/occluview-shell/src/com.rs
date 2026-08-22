//! The COM `IThumbnailProvider` class.
//!
//! Windows Explorer activates this class out-of-process in a `dllhost.exe`
//! surrogate. Per the addendum, the class is a **thin stub**: it stores the
//! file/stream the shell hands it at initialize time, and on `GetThumbnail` it
//! detects the format, renders the mesh, and calls the same
//! `render_thumbnail` code path the CLI uses.
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
use crate::render_thumb::{
    placeholder_for_oversize_input, reserve_thumbnail_stream_job, try_render_thumbnail_file,
    try_render_thumbnail_shared_with_reservation, ThumbnailAttempt, DEFAULT_THUMBNAIL_TIMEOUT,
    MAX_THUMBNAIL_INPUT_BYTES,
};
use crate::stream_read::{read_capped_stream, StreamRead};
use crate::ShellError;
use glam::Vec2;
use occluview_render::ThumbnailSpec;
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use windows::core::{implement, w, IUnknown, Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint,
    RedrawWindow, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, PAINTSTRUCT, RDW_INVALIDATE, RDW_UPDATENOW, SRCCOPY,
};
use windows::Win32::System::Com::STREAM_SEEK_SET;
use windows::Win32::System::Com::{
    CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream, STATFLAG, STATSTG,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HMENU, MSG, WINDOW_EX_STYLE,
    WM_CANCELMODE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE,
    WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

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
const E_POINTER_HR: HRESULT = HRESULT(-2_147_467_259);
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(-2_147_221_231);
const CLASS_E_NOAGGREGATION: HRESULT = HRESULT(-2_147_221_232);
const MAX_PREVIEW_EDGE: u32 = 2048;
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

/// The COM class. Holds the bytes read from the shell-provided stream between
/// `Initialize` and `GetThumbnail`.
#[implement(
    IThumbnailProvider,
    IInitializeWithFile,
    IInitializeWithItem,
    IInitializeWithStream,
    IClassFactory
)]
pub struct ThumbnailProvider {
    /// The bytes of the file, captured eagerly for file-backed paths or
    /// loaded lazily from a shell stream at `GetThumbnail` time.
    bytes: std::cell::RefCell<Arc<[u8]>>,
    /// File-backed vs stream-backed activation is tracked lazily until first render.
    source: std::cell::RefCell<DeferredSource<IStream>>,
    /// Set when Explorer handed us a stream larger than the shell size cap.
    oversize_stream_len: std::cell::Cell<Option<usize>>,
}

struct ThumbnailStreamBytesGuard<'a> {
    bytes: &'a std::cell::RefCell<Arc<[u8]>>,
}

impl<'a> ThumbnailStreamBytesGuard<'a> {
    fn new(bytes: &'a std::cell::RefCell<Arc<[u8]>>) -> Self {
        Self { bytes }
    }
}

impl Drop for ThumbnailStreamBytesGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut bytes) = self.bytes.try_borrow_mut() {
            *bytes = Arc::<[u8]>::from([]);
        } else {
            tracing::warn!("thumbnail stream buffer remained borrowed while releasing request");
        }
    }
}

impl ThumbnailProvider {
    /// Construct an empty provider. Used by the class factory.
    pub fn new() -> Self {
        ACTIVE_COM_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self {
            bytes: std::cell::RefCell::new(Arc::<[u8]>::from([])),
            source: std::cell::RefCell::new(DeferredSource::default()),
            oversize_stream_len: std::cell::Cell::new(None),
        }
    }
}

impl Drop for ThumbnailProvider {
    fn drop(&mut self) {
        ACTIVE_COM_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Default for ThumbnailProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailProvider {
    const MIN_STREAM_BUFFER_BYTES: usize = 16 * 1024;
    const STREAM_READ_CHUNK_BYTES: usize = 1024 * 1024;

    fn rewind_stream(stream: &IStream) -> windows::core::Result<()> {
        // SAFETY: the caller owns a valid COM IStream reference for the
        // duration of this synchronous helper.
        unsafe { stream.Seek(0, STREAM_SEEK_SET, None)? };
        Ok(())
    }

    /// Reads the stream into a byte buffer, capped for shell safety.
    fn read_stream(stream: &IStream) -> windows::core::Result<StreamRead> {
        // SAFETY: `stat` is a stack-local zeroed STATSTG owned by us; the
        // STATFLAG_NONAME flag avoids an internal allocation we'd have to free.
        let mut stat: STATSTG = unsafe { std::mem::zeroed() };
        // SAFETY: passing a valid pointer to our zeroed STATSTG.
        unsafe { stream.Stat(&mut stat, STATFLAG(1))? };
        let declared = if stat.cbSize > 0 {
            // STATSTG.cbSize is a u64 union member (_ULARGE_INTEGER QuadPart).
            #[allow(clippy::cast_possible_truncation)]
            let n = stat.cbSize as u64;
            n
        } else {
            0
        };
        Ok(read_capped_stream(
            (declared != 0).then_some(declared),
            MAX_THUMBNAIL_INPUT_BYTES,
            Self::MIN_STREAM_BUFFER_BYTES,
            Self::STREAM_READ_CHUNK_BYTES,
            |buf| {
                let mut read = 0u32;
                // SAFETY: `buf` is a valid write region; `read` is a stack out-param.
                let result = unsafe {
                    stream.Read(
                        buf.as_mut_ptr() as *mut std::ffi::c_void,
                        buf.len() as u32,
                        Some(std::ptr::from_mut(&mut read)),
                    )
                };
                if result.is_err() {
                    tracing::warn!(hresult = ?result, "shell stream read failed");
                    return Err(());
                }
                Ok(read as usize)
            },
        ))
    }

    /// Render at `size` px (square, clamped to 1..=1024) and return the HBITMAP.
    ///
    /// A transient pipeline failure becomes an error HRESULT here, never a
    /// bitmap: Explorer's thumbcache permanently stores any bitmap returned
    /// with `S_OK`, keyed only by the file's modification time, so a "busy
    /// right now" placeholder would freeze into the file's icon until the file
    /// itself changes. A failed extraction shows the format icon for this
    /// browse and stays eligible for re-extraction — usually served instantly
    /// from the process cache the background worker populated meanwhile.
    fn render_to_hbitmap(&self, size: u32) -> windows::core::Result<HBITMAP> {
        let size_px = size.clamp(1, 1024) as u16;
        let spec = ThumbnailSpec {
            size_px,
            ..Default::default()
        };
        match self.thumbnail_attempt(spec) {
            ThumbnailAttempt::Bitmap(pixels) => {
                pixels_to_hbitmap(&pixels, u32::from(size_px), u32::from(size_px))
            }
            ThumbnailAttempt::TransientFailure => Err(e_fail()),
        }
    }

    /// Produce the verdict for this request: cacheable pixels of exactly
    /// `spec.size_px` (a real render or a deterministic placeholder), or a
    /// transient failure the COM layer reports as an error.
    ///
    /// This matters for a *folder* of files, not just one file. A panic
    /// escaping this COM method unwinds across the `extern "system"` ABI —
    /// which Rust turns into an immediate abort of the whole `dllhost`
    /// surrogate regardless of panic profile — and one bad file would blank
    /// the thumbnails of every *other* file the same surrogate is servicing.
    /// Catching here keeps each request isolated.
    fn thumbnail_attempt(&self, spec: ThumbnailSpec) -> ThumbnailAttempt {
        let produced = catch_unwind(AssertUnwindSafe(|| self.render_attempt(spec)));
        let attempt = produced.unwrap_or_else(|_panic| {
            tracing::error!(
                "thumbnail render panicked; reporting transient failure to keep the COM boundary safe"
            );
            ThumbnailAttempt::TransientFailure
        });

        let expected = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;
        match attempt {
            ThumbnailAttempt::Bitmap(pixels) if pixels.len() == expected => {
                ThumbnailAttempt::Bitmap(pixels)
            }
            ThumbnailAttempt::Bitmap(pixels) => {
                tracing::warn!(
                    got = pixels.len(),
                    expected,
                    "thumbnail pixels had an unexpected size; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            }
            ThumbnailAttempt::TransientFailure => ThumbnailAttempt::TransientFailure,
        }
    }

    /// The underlying verdict producer. May read the shell stream; a stream
    /// read failure is transient (cloud placeholder hydration, network
    /// hiccup), while over-budget and decode verdicts are deterministic
    /// placeholders the shell may cache.
    fn render_attempt(&self, spec: ThumbnailSpec) -> ThumbnailAttempt {
        if let Some(byte_len) = self.oversize_stream_len.get() {
            return ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len));
        }
        // Bind the owned path first so the `source` borrow is released before
        // `ensure_stream_bytes` may borrow it mutably.
        let source_path = self.source.borrow().path().map(PathBuf::from);
        if let Some(path) = source_path {
            return try_render_thumbnail_file(&path, spec, DEFAULT_THUMBNAIL_TIMEOUT);
        }
        let _stream_bytes_guard = ThumbnailStreamBytesGuard::new(&self.bytes);
        let Some(reservation) = reserve_thumbnail_stream_job(DEFAULT_THUMBNAIL_TIMEOUT) else {
            tracing::warn!(
                "thumbnail stream budget was busy; reporting transient failure instead of overcommitting dllhost"
            );
            return ThumbnailAttempt::TransientFailure;
        };
        let ext = self.source.borrow().extension().map(str::to_owned);
        let bytes = match self.ensure_stream_bytes() {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                // The stream was already consumed (a repeat GetThumbnail on an
                // exhausted instance) or it failed mid-read. Neither says
                // anything about the file's content, so fail rather than hand
                // the cache a stand-in bitmap.
                tracing::warn!(
                    "shell stream unavailable for this request; reporting transient failure"
                );
                return ThumbnailAttempt::TransientFailure;
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "shell stream read failed; reporting transient failure"
                );
                return ThumbnailAttempt::TransientFailure;
            }
        };
        if let Some(byte_len) = self.oversize_stream_len.get() {
            ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len))
        } else {
            try_render_thumbnail_shared_with_reservation(
                ext,
                bytes,
                spec,
                DEFAULT_THUMBNAIL_TIMEOUT,
                reservation,
            )
        }
    }

    /// Read the pending shell stream into shared bytes.
    ///
    /// `Ok(Some(bytes))` is a complete copy; `Ok(None)` means no bytes are
    /// available and no deterministic verdict applies (stream consumed, or the
    /// read failed partway) — except the over-cap case, which sets
    /// `oversize_stream_len` and returns empty bytes for the caller's
    /// deterministic oversize placeholder.
    fn ensure_stream_bytes(&self) -> windows::core::Result<Option<Arc<[u8]>>> {
        if !self.bytes.borrow().is_empty() {
            return Ok(Some(self.bytes.borrow().clone()));
        }

        let Some(stream_result) = self.source.borrow_mut().consume_pending_stream(
            |stream, _extension| -> windows::core::Result<StreamRead> {
                ThumbnailProvider::rewind_stream(&stream)?;
                ThumbnailProvider::read_stream(&stream)
            },
        ) else {
            return Ok(None);
        };

        match stream_result? {
            StreamRead::Complete(bytes) => {
                let bytes = Arc::<[u8]>::from(bytes);
                *self.bytes.borrow_mut() = bytes.clone();
                self.oversize_stream_len.set(None);
                Ok(Some(bytes))
            }
            StreamRead::OverCap { byte_len } => {
                *self.bytes.borrow_mut() = Arc::<[u8]>::from([]);
                self.oversize_stream_len.set(Some(byte_len));
                Ok(Some(Arc::<[u8]>::from([])))
            }
            StreamRead::ReadFailed => {
                *self.bytes.borrow_mut() = Arc::<[u8]>::from([]);
                Ok(None)
            }
        }
    }

    fn initialize_path(&self, path: PathBuf) {
        *self.bytes.borrow_mut() = Arc::<[u8]>::from([]);
        self.source
            .borrow_mut()
            .initialize_path(path.clone(), path_extension(&path));
        self.oversize_stream_len.set(None);
    }
}

impl IThumbnailProvider_Impl for ThumbnailProvider_Impl {
    /// Explorer calls this after `Initialize`. `cx` is the max square edge in
    /// pixels; we render exactly that size.
    fn GetThumbnail(
        &self,
        cx: u32,
        phbmp: *mut HBITMAP,
        pdwalpha: *mut WTS_ALPHATYPE,
    ) -> windows::core::Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(e_pointer());
        }
        let hbmp = self.this.render_to_hbitmap(cx)?;
        // SAFETY: phbmp is a caller-provided out-pointer; the shell owns the
        // handle we write through it.
        unsafe { *phbmp = hbmp };
        // SAFETY: pdwalpha is a caller-provided out-pointer.
        unsafe { *pdwalpha = WTSAT_ARGB };
        Ok(())
    }
}

impl IInitializeWithStream_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a read-only stream over the file (handles
    /// MotW / OneDrive placeholders).
    fn Initialize(&self, pstream: Option<&IStream>, _grfmode: u32) -> windows::core::Result<()> {
        let stream = pstream.ok_or_else(e_pointer)?;
        self.this
            .source
            .borrow_mut()
            .initialize_stream(stream.clone());
        *self.this.bytes.borrow_mut() = Arc::<[u8]>::from([]);
        self.this.oversize_stream_len.set(None);
        Ok(())
    }
}

impl IInitializeWithFile_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a filesystem path. This path keeps the file
    /// extension available, which is more reliable than pure magic-byte
    /// probing for text formats and HPS variants.
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
        let path_string = unsafe { pszfilepath.to_string() }.map_err(|_| e_fail())?;
        let path = PathBuf::from(&path_string);
        self.this.initialize_path(path);
        Ok(())
    }
}

impl IInitializeWithItem_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with an item. This gives us a filesystem path on
    /// Explorer code paths that do not use `IInitializeWithFile`, preserving
    /// extension hints for HPS and using mmap-backed file loading.
    fn Initialize(&self, psi: Option<&IShellItem>, _grfmode: u32) -> windows::core::Result<()> {
        let item = psi.ok_or_else(e_pointer)?;
        // SAFETY: `GetDisplayName(SIGDN_FILESYSPATH)` returns a CoTaskMem
        // allocated null-terminated UTF-16 path. We copy it into a Rust String
        // before freeing the COM allocation.
        let path_ptr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path_string = unsafe { path_ptr.to_string() }.map_err(|_| {
            // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
            unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
            e_fail()
        })?;
        // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
        unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
        self.this.initialize_path(PathBuf::from(path_string));
        Ok(())
    }
}

fn path_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
}

fn center_square_on_canvas(
    square: &[u8],
    side_px: u16,
    width: u32,
    height: u32,
    background: [u8; 4],
) -> Vec<u8> {
    let width = width.max(1) as usize;
    let height = height.max(1) as usize;
    let side = usize::from(side_px).min(width).min(height).max(1);
    let mut canvas = vec![0u8; width * height * 4];
    for px in canvas.chunks_exact_mut(4) {
        px.copy_from_slice(&background);
    }
    if square.len() < side * side * 4 {
        return canvas;
    }
    let x0 = (width - side) / 2;
    let y0 = (height - side) / 2;
    for y in 0..side {
        let src = y * side * 4;
        let dst = ((y0 + y) * width + x0) * 4;
        canvas[dst..dst + side * 4].copy_from_slice(&square[src..src + side * 4]);
    }
    canvas
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
    let hbmp = unsafe {
        CreateDIBSection(
            HDC::default(),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        )
    }?;
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

impl IClassFactory_Impl for ThumbnailProvider_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if ppvobject.is_null() {
            return Err(e_pointer());
        }
        if punkouter.is_some() {
            return Err(windows::core::Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        let provider = ThumbnailProvider::new();
        let unknown: IUnknown = provider.into();
        // SAFETY: `riid` and `ppvobject` are COM-supplied; `query` follows the
        // COM ABI contract for QueryInterface.
        let hr = unsafe { unknown.query(riid, ppvobject) };
        if hr.is_ok() {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(hr))
        }
    }

    fn LockServer(&self, _flock: BOOL) -> windows::core::Result<()> {
        // No-op: the surrogate manages the process lifetime; we don't need a
        // server lock count for correctness.
        Ok(())
    }
}

impl IClassFactory_Impl for PreviewHandler_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if ppvobject.is_null() {
            return Err(e_pointer());
        }
        if punkouter.is_some() {
            return Err(windows::core::Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        let provider = PreviewHandler::new();
        let unknown: IUnknown = provider.into();
        // SAFETY: `riid` and `ppvobject` are COM-supplied; `query` follows the
        // COM ABI contract for QueryInterface.
        let hr = unsafe { unknown.query(riid, ppvobject) };
        if hr.is_ok() {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(hr))
        }
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

/// `E_FAIL` (0x80004005) as a `windows::core::Error`.
fn e_fail() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(-2_147_418_235))
}

/// `E_POINTER` (0x80004003) as a `windows::core::Error`.
fn e_pointer() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(-2_147_418_237))
}

/// `E_NOTIMPL` (0x80004001) as a `windows::core::Error`.
fn e_notimpl() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(-2_147_418_239))
}

/// `S_FALSE` (0x00000001) as a `windows::core::Error` so COM returns the
/// non-fatal "not handled" status instead of incorrectly claiming success.
fn s_false() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(1))
}

/// `DllGetClassObject` — the COM runtime calls this when our CLSID is
/// activated. Returns an `IClassFactory` for the requested shell class.
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    rclsid: *const std::ffi::c_void,
    riid: *const std::ffi::c_void,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() || rclsid.is_null() {
        return E_POINTER_HR;
    }
    // SAFETY: `rclsid` is supplied by COM and points to a GUID for the
    // activation request.
    let requested = unsafe { *(rclsid as *const GUID) };
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
        HRESULT(0) // S_OK
    } else {
        hr
    }
}

/// `DllCanUnloadNow` — the COM runtime asks whether this DLL can be unloaded.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if ACTIVE_COM_OBJECTS.load(Ordering::Acquire) == 0 && SERVER_LOCKS.load(Ordering::Acquire) == 0
    {
        HRESULT(0) // S_OK
    } else {
        HRESULT(1) // S_FALSE
    }
}

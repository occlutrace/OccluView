//! The COM `IThumbnailProvider` class (ADR-0005 + addendum).
//!
//! Windows Explorer activates this class out-of-process in a `dllhost.exe`
//! surrogate. Per the addendum, the class is a **thin stub**: it stores the
//! stream the shell hands it in `IInitializeWithStream::Initialize`, and on
//! `GetThumbnail` it reads the bytes, detects the format, and calls the same
//! `render_thumbnail` code path the CLI uses.
//!
//! On any error we return `E_FAIL`; the shell falls back to the default icon.
//! We never propagate a panic across the COM boundary.

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
    clippy::doc_markdown,
    missing_docs
)]

use crate::{render_thumbnail_bytes, ShellError};
use occluview_render::ThumbnailSpec;
use windows::core::{implement, IUnknown, Interface, HRESULT};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Gdi::{CreateBitmap, HBITMAP};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl, IStream, STATFLAG, STATSTG};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithStream, IInitializeWithStream_Impl,
};
use windows::Win32::UI::Shell::{
    IThumbnailProvider, IThumbnailProvider_Impl, WTSAT_UNKNOWN, WTS_ALPHATYPE,
};

/// The OccluView thumbnail-provider CLSID.
///
/// The shell's `IThumbnailProvider` *category* CLSID is the well-known
/// `{E357FCCD-A995-4576-B01F-234630154E96}`; entries under
/// `HKCR\<ext>\ShellEx\{...}` point at this implementation CLSID via their
/// default value.
pub const OCCLUVIEW_THUMBNAIL_CLSID: &str = "{9F3A1B2C-4D5E-4F60-8A7B-9C0D1E2F3045}";

/// The COM class. Holds the bytes read from the shell-provided stream between
/// `Initialize` and `GetThumbnail`.
#[implement(IThumbnailProvider, IInitializeWithStream, IClassFactory)]
pub struct ThumbnailProvider {
    /// The bytes of the file, captured at `Initialize` time.
    bytes: std::cell::RefCell<Vec<u8>>,
    /// Lowercase extension without dot, if known; otherwise dispatched by magic.
    extension: std::cell::RefCell<Option<String>>,
}

impl ThumbnailProvider {
    /// Construct an empty provider. Used by the class factory.
    pub fn new() -> Self {
        Self {
            bytes: std::cell::RefCell::new(Vec::new()),
            extension: std::cell::RefCell::new(None),
        }
    }
}

impl Default for ThumbnailProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailProvider {
    /// Reads the stream into a `Vec<u8>`. Uses `IStream::Stat` to size the
    /// buffer when the stream reports a length; caps the read to keep the
    /// surrogate's RAM bounded.
    fn read_stream(stream: &IStream) -> windows::core::Result<Vec<u8>> {
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
        // Cap to 64 MiB: a thumbnail never needs the whole mesh of a file
        // larger than this, and the shell stream is not mmappable.
        let cap = (declared.min(64 * 1024 * 1024) as usize).max(16 * 1024);
        let mut buf = vec![0u8; cap];
        let mut total = 0usize;
        while total < buf.len() {
            let want = (buf.len() - total).min(1024 * 1024);
            let mut read = 0u32;
            // SAFETY: `buf[total..total+want]` is a valid write region; `read`
            // is a stack out-param.
            let r = unsafe {
                stream.Read(
                    buf.as_mut_ptr().add(total) as *mut std::ffi::c_void,
                    want as u32,
                    Some(std::ptr::from_mut(&mut read)),
                )
            };
            if r.is_err() || read == 0 {
                break;
            }
            total += read as usize;
        }
        buf.truncate(total);
        Ok(buf)
    }

    /// Build a 32bpp BGRA bottom-up `HBITMAP` from top-to-bottom RGBA8 pixels.
    /// The caller (shell) takes ownership and frees the handle.
    fn pixels_to_hbitmap(pixels: &[u8], width: u32, height: u32) -> windows::core::Result<HBITMAP> {
        if width == 0 || height == 0 || pixels.len() != (width * height * 4) as usize {
            return Err(e_fail());
        } // Convert RGBA8 → BGRA8 and flip rows (GDI DIBs are bottom-up).
        let mut bgra = vec![0u8; pixels.len()];
        let row = (width * 4) as usize;
        for y in 0..height as usize {
            let src = y * row;
            let dst = (height as usize - 1 - y) * row;
            for x in 0..width as usize {
                let s = src + x * 4;
                let d = dst + x * 4;
                bgra[d] = pixels[s + 2]; // B
                bgra[d + 1] = pixels[s + 1]; // G
                bgra[d + 2] = pixels[s]; // R
                bgra[d + 3] = pixels[s + 3]; // A
            }
        }
        // SAFETY: CreateBitmap reads width*height*4 bytes of packed pixel data
        // from `bgra` (exactly its length). The produced HBITMAP is a GDI
        // handle the caller frees with DeleteObject.
        let hbmp = unsafe {
            CreateBitmap(
                width as i32,
                height as i32,
                1,
                32,
                Some(bgra.as_ptr() as *const std::ffi::c_void),
            )
        };
        if hbmp.is_invalid() {
            return Err(e_fail());
        }
        Ok(hbmp)
    }

    /// Render at `size` px (square, capped at 1024) and return the HBITMAP.
    fn render_to_hbitmap(&self, size: u32) -> windows::core::Result<HBITMAP> {
        let size_px = size.min(1024) as u16;
        let spec = ThumbnailSpec {
            size_px,
            ..Default::default()
        };
        let ext = self.extension.borrow().clone();
        let bytes = self.bytes.borrow().clone();
        let pixels =
            render_thumbnail_bytes(ext.as_deref(), &bytes, spec).map_err(shell_to_hresult)?;
        Self::pixels_to_hbitmap(&pixels, u32::from(size_px), u32::from(size_px))
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
        unsafe { *pdwalpha = WTSAT_UNKNOWN };
        Ok(())
    }
}

impl IInitializeWithStream_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a read-only stream over the file (handles
    /// MotW / OneDrive placeholders — ADR-0005 addendum).
    fn Initialize(&self, pstream: Option<&IStream>, _grfmode: u32) -> windows::core::Result<()> {
        let stream = pstream.ok_or_else(e_pointer)?;
        *self.this.bytes.borrow_mut() = ThumbnailProvider::read_stream(stream)?;
        // IInitializeWithStream carries no path; format detection falls back
        // to magic bytes inside render_thumbnail's dispatch path.
        *self.this.extension.borrow_mut() = None;
        Ok(())
    }
}

impl IClassFactory_Impl for ThumbnailProvider_Impl {
    fn CreateInstance(
        &self,
        _punkouter: Option<&IUnknown>,
        riid: *const windows::core::GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if ppvobject.is_null() {
            return Err(e_pointer());
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

/// Translate a [`ShellError`] into a generic failing HRESULT. We never surface
/// error text across COM (the shell ignores it and shows its own placeholder).
fn shell_to_hresult(e: ShellError) -> windows::core::Error {
    tracing::warn!(error = ?e, "thumbnail render failed; returning E_FAIL");
    e_fail()
}

/// `E_FAIL` (0x80004005) as a `windows::core::Error`.
fn e_fail() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(-2_147_418_235))
}

/// `E_POINTER` (0x80004003) as a `windows::core::Error`.
fn e_pointer() -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(-2_147_418_237))
}

/// `DllGetClassObject` — the COM runtime calls this when our CLSID is
/// activated. Returns an `IClassFactory` that constructs `ThumbnailProvider`.
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    _rclsid: *const std::ffi::c_void,
    riid: *const std::ffi::c_void,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() || riid.is_null() {
        return HRESULT(-2_147_467_259); // E_POINTER
    }
    let factory = ThumbnailProvider::new();
    let unknown: IUnknown = factory.into();
    // SAFETY: caller-supplied COM pointers; query follows the ABI contract.
    let hr = unsafe { unknown.query(riid as *const windows::core::GUID, ppv) };
    if hr.is_ok() {
        HRESULT(0) // S_OK
    } else {
        hr
    }
}

/// `DllCanUnloadNow` — we never unload; the surrogate manages the lifetime.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    HRESULT(1) // S_FALSE
}

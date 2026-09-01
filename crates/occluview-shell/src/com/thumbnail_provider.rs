//! The `IThumbnailProvider` COM class.
//!
//! Split out of `com.rs` to hold the workspace's 800-line file budget. The
//! shared COM plumbing — HRESULT helpers, `com_entry`, the DLL exports, the
//! GDI bitmap conversion — stays in the parent module; this file owns the
//! provider's state machine from `Initialize` through `GetThumbnail`.

use super::{
    com_entry, e_fail, e_pointer, implement, path_extension, pixels_to_hbitmap, read_capped_stream,
    reserve_thumbnail_stream_job_for_request, try_render_thumbnail_file_with_request,
    try_render_thumbnail_shared_with_reservation, Arc, AssertUnwindSafe, CoTaskMemFree,
    DeferredSource, IClassFactory, IClassFactory_Impl, IInitializeWithFile,
    IInitializeWithFile_Impl, IInitializeWithItem, IInitializeWithItem_Impl, IInitializeWithStream,
    IInitializeWithStream_Impl, IShellItem, IStream, IThumbnailProvider, IThumbnailProvider_Impl,
    IUnknown, Interface, Ordering, PathBuf, Ref, StreamRead, ThumbnailAttempt,
    ThumbnailRenderRequest, ThumbnailSpec, ACTIVE_COM_OBJECTS, BOOL, CLASS_E_NOAGGREGATION,
    DEFAULT_THUMBNAIL_TIMEOUT, GUID, HBITMAP, MAX_OFFSCREEN_EDGE, MAX_THUMBNAIL_INPUT_BYTES,
    PCWSTR, SIGDN_FILESYSPATH, STREAM_SEEK_SET, WTSAT_ARGB, WTS_ALPHATYPE,
};
use super::{placeholder_for_oversize_input, STATFLAG, STATSTG};
use std::panic::catch_unwind;

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

    pub(super) fn rewind_stream(stream: &IStream) -> windows::core::Result<()> {
        // SAFETY: the caller owns a valid COM IStream reference for the
        // duration of this synchronous helper.
        unsafe { stream.Seek(0, STREAM_SEEK_SET, None)? };
        Ok(())
    }

    /// Reads the stream into a byte buffer, capped for shell safety.
    pub(super) fn read_stream(stream: &IStream) -> windows::core::Result<StreamRead> {
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

    /// Render at `size` px (square, clamped to 1..=2048) and return the HBITMAP.
    ///
    /// The ceiling matches the renderer's `downlevel_defaults` texture limit
    /// (`max_texture_dimension_2d = 2048`). Explorer's documented cache
    /// request cap is 1024, but high-DPI extra-large views can ask for the
    /// bigger cache buckets; when the request exceeds the ceiling the shell
    /// scales our smaller bitmap per the `GetThumbnail` contract ("the Shell
    /// draws the returned bitmap at this size or smaller").
    ///
    /// A transient pipeline failure becomes an error HRESULT here, never a
    /// bitmap: Explorer's thumbcache permanently stores any bitmap returned
    /// with `S_OK`, keyed only by the file's modification time, so a "busy
    /// right now" placeholder would freeze into the file's icon until the file
    /// itself changes. A failed extraction shows the format icon for this
    /// browse and stays eligible for re-extraction — usually served instantly
    /// from the process cache the background worker populated meanwhile.
    fn render_to_hbitmap(&self, size: u32) -> windows::core::Result<HBITMAP> {
        let size_px = size.clamp(1, MAX_OFFSCREEN_EDGE) as u16;
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
        let request = ThumbnailRenderRequest::new(DEFAULT_THUMBNAIL_TIMEOUT);
        if let Some(byte_len) = self.oversize_stream_len.get() {
            return ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len));
        }
        // Bind the owned path first so the `source` borrow is released before
        // `ensure_stream_bytes` may borrow it mutably.
        let source_path = self.source.borrow().path().map(PathBuf::from);
        if let Some(path) = source_path {
            return try_render_thumbnail_file_with_request(&path, spec, request);
        }
        let _stream_bytes_guard = ThumbnailStreamBytesGuard::new(&self.bytes);
        let Some(reservation) = reserve_thumbnail_stream_job_for_request(request) else {
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
            try_render_thumbnail_shared_with_reservation(ext, bytes, spec, reservation)
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
        com_entry(
            "IThumbnailProvider::GetThumbnail",
            || Err(e_fail()),
            || {
                if phbmp.is_null() || pdwalpha.is_null() {
                    return Err(e_pointer());
                }
                let hbmp = self.this.render_to_hbitmap(cx)?;
                // SAFETY: phbmp is a caller-provided out-pointer; the shell owns
                // the handle we write through it.
                unsafe { *phbmp = hbmp };
                // SAFETY: pdwalpha is a caller-provided out-pointer.
                unsafe { *pdwalpha = WTSAT_ARGB };
                Ok(())
            },
        )
    }
}

impl IInitializeWithStream_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a read-only stream over the file (handles
    /// MotW / OneDrive placeholders).
    fn Initialize(&self, pstream: Ref<'_, IStream>, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IInitializeWithStream",
            || Err(e_fail()),
            || {
                let stream = pstream.ok()?;
                self.this
                    .source
                    .borrow_mut()
                    .initialize_stream(stream.clone());
                *self.this.bytes.borrow_mut() = Arc::<[u8]>::from([]);
                self.this.oversize_stream_len.set(None);
                Ok(())
            },
        )
    }
}

impl IInitializeWithFile_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a filesystem path. This path keeps the file
    /// extension available, which is more reliable than pure magic-byte
    /// probing for text formats and HPS variants.
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IInitializeWithFile",
            || Err(e_fail()),
            || {
                let path_string = unsafe { pszfilepath.to_string() }.map_err(|_| e_fail())?;
                let path = PathBuf::from(&path_string);
                self.this.initialize_path(path);
                Ok(())
            },
        )
    }
}

impl IInitializeWithItem_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with an item. This gives us a filesystem path on
    /// Explorer code paths that do not use `IInitializeWithFile`, preserving
    /// extension hints for HPS and using mmap-backed file loading.
    fn Initialize(&self, psi: Ref<'_, IShellItem>, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IInitializeWithItem",
            || Err(e_fail()),
            || {
                let item = psi.ok()?;
                // SAFETY: `GetDisplayName(SIGDN_FILESYSPATH)` returns a CoTaskMem
                // allocated null-terminated UTF-16 path. We copy it into a Rust
                // String before freeing the COM allocation.
                let path_ptr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
                let path_string = unsafe { path_ptr.to_string() }.map_err(|_| {
                    // SAFETY: freeing the COM-owned pointer returned by
                    // GetDisplayName.
                    unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
                    e_fail()
                })?;
                // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
                unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
                self.this.initialize_path(PathBuf::from(path_string));
                Ok(())
            },
        )
    }
}

impl IClassFactory_Impl for ThumbnailProvider_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IClassFactory::CreateInstance",
            || Err(e_fail()),
            || {
                if ppvobject.is_null() {
                    return Err(e_pointer());
                }
                if !punkouter.is_null() {
                    return Err(windows::core::Error::from_hresult(CLASS_E_NOAGGREGATION));
                }
                let provider = ThumbnailProvider::new();
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

    fn LockServer(&self, _flock: BOOL) -> windows::core::Result<()> {
        // No-op: the surrogate manages the process lifetime; we don't need a
        // server lock count for correctness.
        Ok(())
    }
}

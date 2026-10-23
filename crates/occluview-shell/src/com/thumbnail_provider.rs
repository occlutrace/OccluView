//! The `IThumbnailProvider` COM class.
//!
//! The shared COM plumbing — HRESULT helpers, `com_entry`, the DLL exports,
//! the GDI bitmap conversion — lives in the parent module; this file owns the
//! provider's state machine from `Initialize` through `GetThumbnail`.

use super::{
    ACTIVE_COM_OBJECTS, Arc, AssertUnwindSafe, BOOL, CLASS_E_NOAGGREGATION, CoTaskMemFree,
    DEFAULT_THUMBNAIL_TIMEOUT, DeferredSource, GUID, HBITMAP, IClassFactory, IClassFactory_Impl,
    IInitializeWithFile, IInitializeWithFile_Impl, IInitializeWithItem, IInitializeWithItem_Impl,
    IInitializeWithStream, IInitializeWithStream_Impl, IShellItem, IStream, IThumbnailProvider,
    IThumbnailProvider_Impl, IUnknown, Interface, MAX_OFFSCREEN_EDGE, Ordering, PCWSTR, PathBuf,
    Ref, SIGDN_FILESYSPATH, StreamRead, ThumbnailAttempt, ThumbnailRenderRequest, ThumbnailSpec,
    WTS_ALPHATYPE, WTSAT_ARGB, com_entry, e_fail, e_pointer, implement, path_extension,
    pixels_to_hbitmap, reserve_thumbnail_stream_job_for_request,
    try_render_thumbnail_file_with_request, try_render_thumbnail_shared_with_reservation,
};
use super::{placeholder_for_oversize_input, stream};
use crate::deferred_source::SourceToken;
#[cfg(feature = "diagnostic-logs")]
use crate::shell_diagnostics::{
    ShellDiagnosticAdapter, ShellDiagnosticComponent, ShellDiagnosticErrorClass,
    ShellDiagnosticOutcome, ShellDiagnosticStage, elapsed_ms_since, prepare_shell_diagnostics,
    record_shell_event, record_shell_failure,
};
use std::os::windows::ffi::OsStringExt;
use std::panic::catch_unwind;
#[cfg(feature = "diagnostic-logs")]
use std::time::Instant;

/// The COM class. Holds the bytes read from the shell-provided stream between
/// `Initialize` and `GetThumbnail`.
// `Agile = false` for the same reason as the preview handler: the fields below
// are `RefCell`/`Cell`, so the class is `!Sync` and must not advertise a
// free-threaded marshaler. See the note on `PreviewHandler`.
#[implement(
    IThumbnailProvider,
    IInitializeWithFile,
    IInitializeWithItem,
    IInitializeWithStream,
    IClassFactory,
    Agile = false
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
    source: &'a std::cell::RefCell<DeferredSource<IStream>>,
    activation: SourceToken,
}

impl<'a> ThumbnailStreamBytesGuard<'a> {
    fn new(
        bytes: &'a std::cell::RefCell<Arc<[u8]>>,
        source: &'a std::cell::RefCell<DeferredSource<IStream>>,
        activation: SourceToken,
    ) -> Self {
        Self {
            bytes,
            source,
            activation,
        }
    }
}

impl Drop for ThumbnailStreamBytesGuard<'_> {
    fn drop(&mut self) {
        // A custom `IStream` can reenter this handler on the same apartment.
        // If that happened, this guard belongs to the old activation and must
        // not erase bytes installed by the newer one.
        let still_current = self
            .source
            .try_borrow()
            .is_ok_and(|source| source.is_current(self.activation));
        if !still_current {
            return;
        }
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
    fn render_to_hbitmap(
        &self,
        size: u32,
        #[cfg(feature = "diagnostic-logs")] started: Instant,
    ) -> windows::core::Result<HBITMAP> {
        let size_px = size.clamp(1, MAX_OFFSCREEN_EDGE) as u16;
        let spec = ThumbnailSpec {
            size_px,
            ..Default::default()
        };
        match self.thumbnail_attempt(spec) {
            ThumbnailAttempt::Bitmap(pixels) => {
                let bitmap = pixels_to_hbitmap(&pixels, u32::from(size_px), u32::from(size_px));
                #[cfg(feature = "diagnostic-logs")]
                if bitmap.is_ok() {
                    record_shell_event(
                        ShellDiagnosticComponent::Thumbnail,
                        ShellDiagnosticStage::BitmapPublish,
                        ShellDiagnosticOutcome::Completed,
                        ShellDiagnosticAdapter::NotObserved,
                        elapsed_ms_since(started),
                    );
                } else {
                    record_shell_failure(
                        ShellDiagnosticComponent::Thumbnail,
                        ShellDiagnosticStage::BitmapPublish,
                        ShellDiagnosticAdapter::NotObserved,
                        ShellDiagnosticErrorClass::Windows,
                        elapsed_ms_since(started),
                    );
                }
                bitmap
            }
            ThumbnailAttempt::TransientFailure => {
                #[cfg(feature = "diagnostic-logs")]
                record_shell_failure(
                    ShellDiagnosticComponent::Thumbnail,
                    ShellDiagnosticStage::Render,
                    ShellDiagnosticAdapter::NotObserved,
                    ShellDiagnosticErrorClass::Transient,
                    elapsed_ms_since(started),
                );
                Err(e_fail())
            }
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
        // The guard and the loader share this activation token. A reentrant
        // Initialize/Unload revokes it, so the old request cannot install its
        // bytes over the newer activation.
        let activation = self.source.borrow().token();
        let _stream_bytes_guard =
            ThumbnailStreamBytesGuard::new(&self.bytes, &self.source, activation);
        let Some(reservation) = reserve_thumbnail_stream_job_for_request(request) else {
            tracing::warn!(
                "thumbnail stream budget was busy; reporting transient failure instead of overcommitting dllhost"
            );
            return ThumbnailAttempt::TransientFailure;
        };
        let ext = self.source.borrow().extension().map(str::to_owned);
        let bytes = match self.ensure_stream_bytes(request, activation) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                // The stream was already consumed (a repeat GetThumbnail on an
                // exhausted instance), it failed mid-read, or this activation
                // was revoked while the external stream call ran. None says
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

        // The owned stream is retired when `ensure_stream_bytes` returns. A
        // custom `IStream::Release` can reenter Initialize/Unload there, so
        // fence again before consuming oversize state or rendering the old
        // activation's bytes.
        if !self.source.borrow().is_current(activation) {
            return ThumbnailAttempt::TransientFailure;
        }
        let attempt = if let Some(byte_len) = self.oversize_stream_len.get() {
            ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len))
        } else {
            try_render_thumbnail_shared_with_reservation(ext, bytes, spec, reservation)
        };

        // No shell stream reference is dropped after this point, but keep the
        // fence explicit before returning a cacheable bitmap to the caller.
        if !self.source.borrow().is_current(activation) {
            return ThumbnailAttempt::TransientFailure;
        }
        attempt
    }

    /// Read the pending shell stream into shared bytes.
    ///
    /// `Ok(Some(bytes))` is a complete copy; `Ok(None)` means no bytes are
    /// available and no deterministic verdict applies (stream consumed, the
    /// read failed partway, or the activation was revoked before the result
    /// could be installed) — except the over-cap case, which sets
    /// `oversize_stream_len` and returns empty bytes for the caller's
    /// deterministic oversize placeholder. The caller must recheck the
    /// activation token after this returns: the local stream's final COM
    /// `Release` can reenter Initialize/Unload.
    fn ensure_stream_bytes(
        &self,
        request: ThumbnailRenderRequest,
        activation: SourceToken,
    ) -> windows::core::Result<Option<Arc<[u8]>>> {
        if !self.source.borrow().is_current(activation) {
            return Ok(None);
        }
        let cached = self.bytes.borrow().clone();
        if !cached.is_empty() {
            return Ok(Some(cached));
        }

        // Move the stream and its extension out under a short borrow, then let
        // that borrow end before any external COM Seek/Stat/Read. Releasing
        // the handler borrow is what lets a custom stream reenter Initialize
        // or Unload without a RefCell panic.
        let pending = self.source.borrow_mut().take_pending_stream();
        let Some((stream, _extension, taken_activation)) = pending else {
            return Ok(None);
        };
        if taken_activation != activation {
            return Ok(None);
        }

        let stream_result = stream::rewind_and_read_until(&stream, request.response_deadline());

        // A reentrant activation may have completed while the external calls
        // were running. Check the fence before installing bytes or oversize
        // state for the old activation.
        if !self.source.borrow().is_current(activation) {
            return Ok(None);
        }

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
            StreamRead::ReadFailed | StreamRead::TimedOut => {
                *self.bytes.borrow_mut() = Arc::<[u8]>::from([]);
                Ok(None)
            }
        }
    }

    fn initialize_path(&self, path: PathBuf) {
        *self.bytes.borrow_mut() = Arc::<[u8]>::from([]);
        let retired = self
            .source
            .borrow_mut()
            .initialize_path(path.clone(), path_extension(&path));
        self.oversize_stream_len.set(None);
        // Drop the retired COM reference only after the source borrow is gone;
        // its Release can reenter Initialize/Unload.
        drop(retired);
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
                #[cfg(feature = "diagnostic-logs")]
                let started = Instant::now();
                #[cfg(feature = "diagnostic-logs")]
                prepare_shell_diagnostics();
                #[cfg(feature = "diagnostic-logs")]
                record_shell_event(
                    ShellDiagnosticComponent::Thumbnail,
                    ShellDiagnosticStage::Activation,
                    ShellDiagnosticOutcome::Started,
                    ShellDiagnosticAdapter::NotObserved,
                    0,
                );
                if phbmp.is_null() || pdwalpha.is_null() {
                    #[cfg(feature = "diagnostic-logs")]
                    record_shell_failure(
                        ShellDiagnosticComponent::Thumbnail,
                        ShellDiagnosticStage::ComReturn,
                        ShellDiagnosticAdapter::NotObserved,
                        ShellDiagnosticErrorClass::Windows,
                        elapsed_ms_since(started),
                    );
                    return Err(e_pointer());
                }
                let result = self.this.render_to_hbitmap(
                    cx,
                    #[cfg(feature = "diagnostic-logs")]
                    started,
                );
                if let Ok(hbmp) = &result {
                    // SAFETY: phbmp is a caller-provided out-pointer; the shell owns
                    // the handle we write through it.
                    unsafe { *phbmp = *hbmp };
                    // SAFETY: pdwalpha is a caller-provided out-pointer.
                    unsafe { *pdwalpha = WTSAT_ARGB };
                }
                #[cfg(feature = "diagnostic-logs")]
                if result.is_ok() {
                    record_shell_event(
                        ShellDiagnosticComponent::Thumbnail,
                        ShellDiagnosticStage::ComReturn,
                        ShellDiagnosticOutcome::Completed,
                        ShellDiagnosticAdapter::NotObserved,
                        elapsed_ms_since(started),
                    );
                } else {
                    record_shell_failure(
                        ShellDiagnosticComponent::Thumbnail,
                        ShellDiagnosticStage::ComReturn,
                        ShellDiagnosticAdapter::NotObserved,
                        ShellDiagnosticErrorClass::Transient,
                        elapsed_ms_since(started),
                    );
                }
                result.map(|_| ())
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
                // Acquire our owned reference before evaluating the
                // `borrow_mut()` receiver below. `Ref::ok` returns `&IStream`;
                // AddRef must not run while the handler RefCell is borrowed.
                let activation = self.this.source.borrow().token();
                let stream = pstream.ok()?.clone();
                let still_current = self.this.source.borrow().is_current(activation);
                if !still_current {
                    return Err(e_fail());
                }
                let retired = self.this.source.borrow_mut().initialize_stream(stream);
                *self.this.bytes.borrow_mut() = Arc::<[u8]>::from([]);
                self.this.oversize_stream_len.set(None);
                // Drop the retired COM reference only after the source borrow
                // is gone; its Release can reenter Initialize/Unload.
                drop(retired);
                Ok(())
            },
        )
    }
}

impl IInitializeWithFile_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with a filesystem path. This path keeps the file
    /// extension available, which is more reliable than pure magic-byte
    /// probing for text formats.
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IInitializeWithFile",
            || Err(e_fail()),
            || {
                // SAFETY: `to_string()` on a path with an unpaired surrogate
                // returns E_FAIL, so such a file would get no thumbnail at all
                // even though Explorer handed it over. An OS string carries the
                // path unchanged.
                let path = std::ffi::OsString::from_wide(unsafe { pszfilepath.as_wide() });
                self.this.initialize_path(PathBuf::from(path));
                Ok(())
            },
        )
    }
}

impl IInitializeWithItem_Impl for ThumbnailProvider_Impl {
    /// Called by the shell with an item. This gives us a filesystem path on
    /// Explorer code paths that do not use `IInitializeWithFile`, which
    /// preserves the extension hint used by format detection.
    fn Initialize(&self, psi: Ref<'_, IShellItem>, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "thumbnail IInitializeWithItem",
            || Err(e_fail()),
            || {
                let activation = self.this.source.borrow().token();
                let item = psi.ok()?;
                // SAFETY: `GetDisplayName(SIGDN_FILESYSPATH)` returns a CoTaskMem
                // allocated null-terminated UTF-16 path. We copy it into a Rust
                // OS string before freeing the COM allocation.
                let path_ptr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
                // SAFETY: GetDisplayName returned a live NUL-terminated path.
                // Preserve every filename code unit before freeing its COM allocation.
                let path = std::ffi::OsString::from_wide(unsafe { path_ptr.as_wide() });
                // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
                unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
                let still_current = self.this.source.borrow().is_current(activation);
                if !still_current {
                    return Err(e_fail());
                }
                self.this.initialize_path(PathBuf::from(path));
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

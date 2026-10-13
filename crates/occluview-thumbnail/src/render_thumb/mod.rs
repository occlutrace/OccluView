//! The safe, Windows-agnostic core of thumbnail generation.
//!
//! The COM class calls into
//! [`render_thumbnail`] - this function does all the work and is unit-testable
//! without Windows. It loads the file via `occluview-formats`, frames the
//! camera with the dental occlusal default, and renders an offscreen frame via
//! `occluview-render`.
//!
//! Thumbnails intentionally use the same canonical occlusal framing as the app
//! viewport. Explorer preview should be a small version of what opens in the
//! viewer, not a separately auto-rotated interpretation of the mesh.

#![cfg_attr(
    test,
    allow(
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::expect_used
    )
)]

use crate::placeholder::{placeholder_thumbnail, placeholder_thumbnail_kind};
use crate::ThumbnailError;
use occluview_formats::FormatError;
use occluview_render::{AdapterPolicy, RenderDeadline, ThumbnailSpec};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Condvar;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

mod cache;
mod concurrency;
mod fallback;
mod loading;
mod rendering;

#[cfg(test)]
mod tests;

use cache::{
    oversize_input_error, thumbnail_background_key, thumbnail_file_cache,
    thumbnail_file_content_cache, thumbnail_file_content_key, thumbnail_stream_cache,
    FileThumbnailPreflightError, StreamThumbnailPreflightError, ThumbnailFileCacheKey,
    ThumbnailFileContentKey, ThumbnailFileMetadata, ThumbnailRequestKey,
};
#[cfg(test)]
use concurrency::render_coalesced_thumbnail;
use concurrency::{
    render_coalesced_thumbnail_by, run_thumbnail_job_by, run_thumbnail_job_by_deadline,
    ThumbnailJobOutcome, ThumbnailJobPermit, ThumbnailJobProgress, ThumbnailRendererPool,
};
pub use fallback::placeholder_for_oversize_input;
use fallback::placeholder_kind_for_error;
use loading::{
    load_thumbnail_mesh_from_bytes, load_thumbnail_mesh_from_bytes_kind,
    load_thumbnail_mesh_from_file, load_thumbnail_mesh_from_opened_file, prepare_file_thumbnail_render,
    prepare_stream_thumbnail_render, thumbnail_format_kind_from_file_bytes,
};

/// Default maximum wall-clock wait for a shell thumbnail request.
pub const DEFAULT_THUMBNAIL_TIMEOUT: Duration = Duration::from_millis(6_000);
/// Device setup happens outside Explorer's first thumbnail request. A real
/// request still retains its own shorter response deadline when it waits for
/// this shared pool slot.
const THUMBNAIL_RENDERER_PREWARM_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum GPU/render time for an already-detached cache warmer.
///
/// A caller that has exhausted its response budget must never receive the
/// warmer's result. Keeping this separately bounded preserves the useful
/// second-paint cache behaviour without silently extending an Explorer COM
/// call or leaving an unbounded worker behind.
const BACKGROUND_CACHE_WARM_TIMEOUT: Duration = DEFAULT_THUMBNAIL_TIMEOUT;

#[cfg(test)]
static THUMBNAIL_FILE_CACHE_CHANGED: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();

#[cfg(test)]
fn thumbnail_file_cache_changed() -> &'static (Mutex<()>, Condvar) {
    THUMBNAIL_FILE_CACHE_CHANGED.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

#[cfg(test)]
fn notify_thumbnail_file_cache_changed() {
    let (lock, changed) = thumbnail_file_cache_changed();
    let _guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    changed.notify_all();
}

#[cfg(test)]
fn wait_for_thumbnail_file_cache(timeout: Duration, mut cached: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    let (lock, changed) = thumbnail_file_cache_changed();
    let mut guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    loop {
        if cached() {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let (next_guard, result) = changed
            .wait_timeout(guard, remaining)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard = next_guard;
        if result.timed_out() && !cached() {
            return false;
        }
    }
}

/// Immutable timing and adapter policy for one thumbnail request.
///
/// Both absolute deadlines are fixed when the Shell request enters the
/// pipeline. The response deadline applies to stages that observe it, but it
/// cannot interrupt synchronous source I/O already in progress, such as a
/// shell stream's `Seek`, `Stat`, or `Read` call. A worker that outlives the
/// response may write only to the process cache, using the separately bounded
/// cache-warm deadline captured at the same instant.
#[derive(Clone, Copy, Debug)]
pub struct ThumbnailRenderRequest {
    response_deadline: Instant,
    cache_warm_deadline: Instant,
    response_timeout: Duration,
    adapter_policy: AdapterPolicy,
}

impl ThumbnailRenderRequest {
    /// Create a normal Shell request at the current instant.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self::from_started_at(
            Instant::now(),
            timeout,
            crate::offscreen_factory::default_thumbnail_adapter_policy(),
        )
    }

    /// Create a request from a known entry instant.
    ///
    /// This is public to make the end-to-end deadline contract directly
    /// testable by callers that own the Shell boundary.
    #[must_use]
    pub fn from_started_at(
        started_at: Instant,
        response_timeout: Duration,
        adapter_policy: AdapterPolicy,
    ) -> Self {
        let cache_warm_timeout = response_timeout.max(BACKGROUND_CACHE_WARM_TIMEOUT);
        Self {
            response_deadline: started_at + response_timeout,
            cache_warm_deadline: started_at + cache_warm_timeout,
            response_timeout,
            adapter_policy,
        }
    }

    /// Absolute cutoff for the COM response.
    #[must_use]
    pub const fn response_deadline(self) -> Instant {
        self.response_deadline
    }

    /// Absolute cutoff for an already-detached cache worker.
    #[must_use]
    pub const fn cache_warm_deadline(self) -> Instant {
        self.cache_warm_deadline
    }

    /// The configured COM-response budget, retained only for fixed-category
    /// diagnostics. Timing decisions use the absolute deadline above.
    #[must_use]
    pub const fn response_timeout(self) -> Duration {
        self.response_timeout
    }

    /// Renderer selection policy fixed for this request.
    #[must_use]
    pub const fn adapter_policy(self) -> AdapterPolicy {
        self.adapter_policy
    }
}
/// Maximum stream size the shell thumbnail path will parse.
pub const MAX_THUMBNAIL_INPUT_BYTES: usize = 192 * 1024 * 1024;
/// Maximum local-file thumbnail input size, bounding the owned read buffer.
pub const MAX_THUMBNAIL_FILE_BYTES: usize = 512 * 1024 * 1024;

static THUMBNAIL_INFLIGHT: OnceLock<
    Mutex<std::collections::HashMap<ThumbnailRequestKey, Arc<concurrency::InflightThumbnail>>>,
> = OnceLock::new();
static THUMBNAIL_FILE_CACHE: OnceLock<Mutex<cache::ThumbnailFileCache>> = OnceLock::new();
static THUMBNAIL_FILE_CONTENT_CACHE: OnceLock<Mutex<cache::ThumbnailFileContentCache>> =
    OnceLock::new();
static THUMBNAIL_STREAM_CACHE: OnceLock<Mutex<cache::ThumbnailStreamCache>> = OnceLock::new();
static THUMBNAIL_RENDERER_POOL: OnceLock<ThumbnailRendererPool> = OnceLock::new();
static THUMBNAIL_JOB_GATE: OnceLock<concurrency::ThumbnailJobGate> = OnceLock::new();

/// Prepare the one shared thumbnail renderer before Explorer requests pixels.
///
/// This is called from a background class-activation warmup, never from the
/// COM rendering callback. The pool serializes this setup with real requests,
/// so no duplicate D3D devices are created under a folder burst.
pub fn prewarm_thumbnail_renderer() {
    let _ = ThumbnailRendererPool::shared().with_renderer(
        RenderDeadline::after(THUMBNAIL_RENDERER_PREWARM_TIMEOUT),
        crate::offscreen_factory::default_thumbnail_adapter_policy(),
        |_| Ok(()),
    );
}

/// Outcome of one thumbnail request against the shared pipeline.
///
/// The distinction exists because Explorer's thumbnail cache permanently
/// stores any bitmap a provider returns with `S_OK`, keyed by the file's
/// modification time ("once a thumbnail is computed ... it is cached and your
/// handler won't be called again for that item unless you invalidate the cache
/// by updating the modification date" — Microsoft, `RecipeThumbnailProvider`).
/// Returning a placeholder for a *transient* condition therefore freezes that
/// placeholder into the file's icon until the file itself changes. Only
/// verdicts that will reproduce on every future attempt may become pixels.
#[derive(Debug, Eq, PartialEq)]
pub enum ThumbnailAttempt {
    /// Pixels the shell may cache: a real render, or a deterministic
    /// placeholder verdict (corrupt file, unsupported payload, over budget).
    Bitmap(Vec<u8>),
    /// No verdict inside the caller's budget: the job queue was saturated, the
    /// render timed out, the GPU faulted, or the source stream misbehaved.
    /// The worker may still be finishing in the background and will publish
    /// into the process cache; the COM layer must answer with a failure
    /// `HRESULT` so Explorer retries instead of caching a stand-in bitmap.
    TransientFailure,
}

impl ThumbnailAttempt {
    /// Collapse to pixels, substituting the deterministic placeholder for a
    /// transient failure. This is the freedesktop-thumbnailer contract (the
    /// CLI must always emit a PNG) and the pre-cache-aware compatibility
    /// behavior of the `*_or_placeholder` entry points.
    #[must_use]
    pub fn into_pixels_or_placeholder(self, spec: ThumbnailSpec) -> Vec<u8> {
        match self {
            Self::Bitmap(pixels) => pixels,
            Self::TransientFailure => placeholder_thumbnail(spec),
        }
    }
}

/// Capacity reserved before a shell stream is copied into memory.
///
/// Explorer's isolated thumbnail path initializes providers with `IStream`.
/// Reserving first keeps a mixed folder from materializing every large file in
/// `dllhost` before the ordinary decode/render gate can apply.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ThumbnailJobReservation {
    permit: ThumbnailJobPermit,
    /// When the request that took this reservation must be finished.
    ///
    /// Fixed at reservation time so slot wait, stream copy, and rendering share
    /// one request budget.
    request: ThumbnailRenderRequest,
}

#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
/// Reserve one bounded stream thumbnail job before copying shell bytes.
pub fn reserve_thumbnail_stream_job(timeout: Duration) -> Option<ThumbnailJobReservation> {
    reserve_thumbnail_stream_job_for_request(ThumbnailRenderRequest::new(timeout))
}

/// Reserve a stream lane under a request deadline fixed at Shell entry.
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub fn reserve_thumbnail_stream_job_for_request(
    request: ThumbnailRenderRequest,
) -> Option<ThumbnailJobReservation> {
    concurrency::ThumbnailJobGate::shared()
        .acquire_by(request.response_deadline())
        .map(|permit| ThumbnailJobReservation { permit, request })
}

/// Load `bytes` (a file with the given lowercase extension, no dot) and render
/// a thumbnail per `spec`. Returns RGBA8 pixels in row-major order, length
/// `spec.size_px * spec.size_px * 4`, top-to-bottom.
///
/// Blocking: runs the offscreen render to completion on the calling thread.
/// The COM stub invokes this on a worker thread under a
/// Job Object with a watchdog.
///
/// # Errors
/// See [`ThumbnailError`]. The shell layer translates decode errors into a
/// branded placeholder and transient errors into a failure `HRESULT`.
pub fn render_thumbnail(
    extension: &str,
    bytes: &[u8],
    spec: ThumbnailSpec,
) -> Result<Vec<u8>, ThumbnailError> {
    render_thumbnail_bytes(Some(extension), bytes, spec)
}

/// Load `bytes` with an optional file extension hint and render a thumbnail.
///
/// This is the entry point for shell streams where Windows may not provide a
/// file path. It never falls back to a fake default extension.
///
/// # Errors
/// Returns [`ThumbnailError::Format`] if inference or parsing fails, and
/// [`ThumbnailError::Render`] if offscreen rendering fails.
pub fn render_thumbnail_bytes(
    extension: Option<&str>,
    bytes: &[u8],
    spec: ThumbnailSpec,
) -> Result<Vec<u8>, ThumbnailError> {
    let mesh = load_thumbnail_mesh_from_bytes(extension, bytes)?;
    rendering::render_mesh_thumbnail(mesh, spec, RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT))
}

/// Load a local file through the shared reader and render a thumbnail.
///
/// This path is preferred for Explorer `IInitializeWithFile` /
/// `IInitializeWithItem` initialization because it keeps the extension hint
/// for HPS, which the byte-stream entry point cannot: the hint is what selects
/// the container parser.
///
/// # Errors
/// Returns [`ThumbnailError::Format`] for unsupported/malformed inputs and
/// [`ThumbnailError::Render`] for GPU/offscreen failures.
pub fn render_thumbnail_file(path: &Path, spec: ThumbnailSpec) -> Result<Vec<u8>, ThumbnailError> {
    let metadata = cache::thumbnail_file_metadata(path)?;
    let mesh = load_thumbnail_mesh_from_file(path, metadata)?;
    rendering::render_mesh_thumbnail(mesh, spec, RenderDeadline::after(DEFAULT_THUMBNAIL_TIMEOUT))
}

/// Render a thumbnail or return the deterministic fallback placeholder.
///
/// This is the COM-facing safe path: Explorer receives a bitmap even when the
/// file is malformed, unsupported, or rendering fails.
#[must_use]
pub fn render_thumbnail_or_placeholder(
    extension: Option<&str>,
    bytes: &[u8],
    spec: ThumbnailSpec,
) -> Vec<u8> {
    let extension = extension.map(ToOwned::to_owned);
    let bytes = Arc::<[u8]>::from(bytes.to_vec());
    render_thumbnail_shared_or_placeholder_with_timeout(
        extension,
        bytes,
        spec,
        DEFAULT_THUMBNAIL_TIMEOUT,
    )
}

/// Render a local file thumbnail or return the deterministic fallback
/// placeholder.
#[must_use]
pub fn render_thumbnail_file_or_placeholder(path: &Path, spec: ThumbnailSpec) -> Vec<u8> {
    render_thumbnail_file_or_placeholder_with_timeout(path, spec, DEFAULT_THUMBNAIL_TIMEOUT)
}

/// Render with a bounded wait or return the deterministic placeholder.
///
/// The worker thread may finish after the caller has returned; that is still
/// better than blocking Explorer's thumbnail worker beyond the time budget.
#[must_use]
pub fn render_thumbnail_or_placeholder_with_timeout(
    extension: Option<&str>,
    bytes: &[u8],
    spec: ThumbnailSpec,
    timeout: Duration,
) -> Vec<u8> {
    let extension = extension.map(ToOwned::to_owned);
    let bytes = Arc::<[u8]>::from(bytes.to_vec());
    render_thumbnail_shared_or_placeholder_with_timeout(extension, bytes, spec, timeout)
}

/// Render a local file with a bounded wait or return the deterministic
/// placeholder.
#[must_use]
pub fn render_thumbnail_file_or_placeholder_with_timeout(
    path: &Path,
    spec: ThumbnailSpec,
    timeout: Duration,
) -> Vec<u8> {
    try_render_thumbnail_file(path, spec, timeout).into_pixels_or_placeholder(spec)
}

/// Render a local file with a bounded wait, reporting transient failures
/// instead of masking them as placeholder pixels.
///
/// This is the shell-facing entry point: see [`ThumbnailAttempt`] for why a
/// timeout must not become a bitmap when the caller is Explorer's thumbnail
/// cache.
#[must_use]
pub fn try_render_thumbnail_file(
    path: &Path,
    spec: ThumbnailSpec,
    timeout: Duration,
) -> ThumbnailAttempt {
    try_render_thumbnail_file_with_request(path, spec, ThumbnailRenderRequest::new(timeout))
}

/// Render a local file for a Shell request whose deadline was fixed at entry.
#[must_use]
pub fn try_render_thumbnail_file_with_request(
    path: &Path,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
) -> ThumbnailAttempt {
    if Instant::now() >= request.response_deadline() {
        return ThumbnailAttempt::TransientFailure;
    }

    // Extension support is a path-only, deterministic verdict. Keep it outside
    // the worker so unrelated files in a mixed folder do not consume a lane.
    if !path_has_supported_thumbnail_extension(path) {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail file extension is not registered for OccluView; returning placeholder"
        );
        return ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec));
    }

    let path = path.to_path_buf();
    let outcome = run_thumbnail_job_by_deadline(request.response_deadline(), move |progress| {
        render_file_thumbnail_worker(path, spec, request, progress);
    });
    thumbnail_attempt_for_file_worker_outcome(outcome, spec, request)
}

fn path_has_supported_thumbnail_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| {
            crate::SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension == *supported)
        })
}

fn render_file_thumbnail_worker(
    path: PathBuf,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
    progress: std::sync::mpsc::SyncSender<ThumbnailJobProgress<ThumbnailAttempt>>,
) {
    let plan = match prepare_file_thumbnail_render(&path) {
        Ok(plan) => plan,
        Err(FileThumbnailPreflightError::UnsupportedExtension) => {
            let _ = progress.send(ThumbnailJobProgress::Finished(
                ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec)),
            ));
            return;
        }
        Err(FileThumbnailPreflightError::Metadata(error)) => {
            // Transient: the common causes are a sharing violation while the
            // scanner is still writing the file, or a file that vanished
            // mid-browse. Neither is a verdict about the file's content.
            tracing::warn!(?error, extension = ?path.extension(), "thumbnail file metadata failed");
            let _ = progress.send(ThumbnailJobProgress::Finished(
                ThumbnailAttempt::TransientFailure,
            ));
            return;
        }
        Err(FileThumbnailPreflightError::NotAFile) => {
            // Deterministic: a directory or a named pipe does not become a
            // scan by being asked twice. Refusing here keeps the bounded
            // worker from opening a pipe with no writer and blocking forever.
            tracing::warn!(
                extension = ?path.extension(),
                "thumbnail path is not a regular file; returning placeholder"
            );
            let _ = progress.send(ThumbnailJobProgress::Finished(
                ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec)),
            ));
            return;
        }
        Err(FileThumbnailPreflightError::Oversize { byte_len }) => {
            let _ = progress.send(ThumbnailJobProgress::Finished(
                ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len)),
            ));
            return;
        }
    };

    let source = match occluview_formats::dispatch::read_opened_file_bytes_with_limit(
        &path,
        MAX_THUMBNAIL_FILE_BYTES as u64,
    ) {
        Ok(source) => source,
        Err(FormatError::TooLarge { bytes, .. }) => {
            let byte_len = usize::try_from(bytes).unwrap_or(usize::MAX);
            let _ = progress.send(ThumbnailJobProgress::Finished(ThumbnailAttempt::Bitmap(
                placeholder_for_oversize_input(spec, byte_len),
            )));
            return;
        }
        Err(error) => {
            tracing::warn!(?error, extension = ?path.extension(), "thumbnail file read failed");
            let _ = progress.send(ThumbnailJobProgress::Finished(
                ThumbnailAttempt::TransientFailure,
            ));
            return;
        }
    };

    let opened_metadata = cache::thumbnail_file_metadata_from_std(source.initial_metadata());
    if !source.initial_metadata().is_file()
        || opened_metadata != plan.metadata
        || u64::try_from(source.file_bytes().as_slice().len()).unwrap_or(u64::MAX)
            != plan.metadata.byte_len
    {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail source changed between preflight and bounded read"
        );
        let _ = progress.send(ThumbnailJobProgress::Finished(
            ThumbnailAttempt::TransientFailure,
        ));
        return;
    }

    let kind = match thumbnail_format_kind_from_file_bytes(source.file_bytes()) {
        Ok(kind) => kind,
        Err(error) => {
            let attempt = thumbnail_attempt_for_job_outcome(
                ThumbnailJobOutcome::Finished(Err(error)),
                spec,
                request.response_timeout(),
                "file",
            );
            let _ = progress.send(ThumbnailJobProgress::Finished(attempt));
            return;
        }
    };
    let cacheable_by_primary_bytes = file_thumbnail_can_share_primary_bytes(kind);
    let content_key = cacheable_by_primary_bytes
        .then(|| thumbnail_file_content_key(&path, source.file_bytes().as_slice()));

    // The preflight was path based; verify the held source handle and current
    // path after preparation, before cached pixels can be returned.
    if !thumbnail_file_source_matches(&path, &source, &plan.metadata) {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail source changed during content preparation"
        );
        let _ = progress.send(ThumbnailJobProgress::Finished(
            ThumbnailAttempt::TransientFailure,
        ));
        return;
    }

    if let Some(content_key) = content_key.as_ref() {
        if let Some(pixels) = file_content_cache_lookup(content_key, spec) {
            if !thumbnail_file_source_matches(&path, &source, &plan.metadata) {
                let _ = progress.send(ThumbnailJobProgress::Finished(
                    ThumbnailAttempt::TransientFailure,
                ));
                return;
            }
            if let Ok(mut path_cache) = thumbnail_file_cache().lock() {
                path_cache.insert_with_background(
                    plan.cache_key.clone(),
                    spec.size_px,
                    thumbnail_background_key(spec.background),
                    &pixels,
                );
            }
            let _ = progress.send(ThumbnailJobProgress::Finished(ThumbnailAttempt::Bitmap(
                pixels,
            )));
            return;
        }
    }

    let cache_keys = if cacheable_by_primary_bytes {
        FileThumbnailCacheKeys {
            path: Some(plan.cache_key.clone()),
            content: content_key.clone(),
        }
    } else {
        FileThumbnailCacheKeys {
            path: None,
            content: None,
        }
    };
    let metadata = plan.metadata;
    if !cacheable_by_primary_bytes {
        let attempt = render_file_thumbnail_mesh_job(
            path,
            source,
            kind,
            metadata,
            cache_keys,
            spec,
            request,
            Some(progress.clone()),
        );
        let _ = progress.send(ThumbnailJobProgress::Finished(attempt));
        return;
    }
    let Some(content_key) = content_key else {
        let _ = progress.send(ThumbnailJobProgress::Finished(
            ThumbnailAttempt::TransientFailure,
        ));
        return;
    };
    let inflight_key = ThumbnailRequestKey::FileContent {
        cache_key: content_key,
        size_px: spec.size_px,
        background: thumbnail_background_key(spec.background),
    };
    let progress_for_render = progress.clone();
    let attempt = render_coalesced_thumbnail_by(
        inflight_key,
        request.response_deadline(),
        move || {
            render_file_thumbnail_mesh_job(
                path,
                source,
                kind,
                metadata,
                cache_keys,
                spec,
                request,
                Some(progress_for_render),
            )
        },
    );
    let _ = progress.send(ThumbnailJobProgress::Finished(attempt));
}

fn file_thumbnail_can_share_primary_bytes(kind: occluview_formats::FormatKind) -> bool {
    !matches!(
        kind,
        occluview_formats::FormatKind::Obj | occluview_formats::FormatKind::Ply
    )
}

fn thumbnail_file_source_matches(
    path: &Path,
    source: &occluview_formats::dispatch::OpenedFileBytes,
    expected: &ThumbnailFileMetadata,
) -> bool {
    // Freshness is checked by length and timestamp, not by filesystem identity.
    // A replacement preserving both can pass this fence; the owned buffer
    // still keeps its digest and parser input consistent with each other.
    source
        .current_metadata()
        .ok()
        .filter(std::fs::Metadata::is_file)
        .is_some_and(|metadata| cache::thumbnail_file_metadata_from_std(&metadata) == *expected)
        && !cache::thumbnail_file_changed_since(path, expected)
}

fn file_content_cache_lookup(
    key: &ThumbnailFileContentKey,
    spec: ThumbnailSpec,
) -> Option<Vec<u8>> {
    thumbnail_file_content_cache()
        .lock()
        .ok()
        .and_then(|mut cache| {
            cache.get_with_background(key, spec.size_px, thumbnail_background_key(spec.background))
        })
}

struct FileThumbnailCacheKeys {
    path: Option<ThumbnailFileCacheKey>,
    content: Option<ThumbnailFileContentKey>,
}

#[cfg(test)]
fn render_file_thumbnail_job(
    path: PathBuf,
    metadata: ThumbnailFileMetadata,
    cache_keys: FileThumbnailCacheKeys,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
) -> ThumbnailAttempt {
    let source = match occluview_formats::dispatch::read_opened_file_bytes_with_limit(
        &path,
        MAX_THUMBNAIL_FILE_BYTES as u64,
    ) {
        Ok(source) => source,
        Err(error) => {
            return thumbnail_attempt_for_job_outcome(
                ThumbnailJobOutcome::Finished(Err(error.into())),
                spec,
                request.response_timeout(),
                "file",
            );
        }
    };
    if !source.initial_metadata().is_file()
        || cache::thumbnail_file_metadata_from_std(source.initial_metadata()) != metadata
    {
        return ThumbnailAttempt::TransientFailure;
    }
    let kind = match thumbnail_format_kind_from_file_bytes(source.file_bytes()) {
        Ok(kind) => kind,
        Err(error) => {
            return thumbnail_attempt_for_job_outcome(
                ThumbnailJobOutcome::Finished(Err(error)),
                spec,
                request.response_timeout(),
                "file",
            );
        }
    };
    render_file_thumbnail_mesh_job(path, source, kind, metadata, cache_keys, spec, request, None)
}

fn render_file_thumbnail_mesh_job(
    path: PathBuf,
    source: occluview_formats::dispatch::OpenedFileBytes,
    kind: occluview_formats::FormatKind,
    metadata: ThumbnailFileMetadata,
    cache_keys: FileThumbnailCacheKeys,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
    progress: Option<std::sync::mpsc::SyncSender<ThumbnailJobProgress<ThumbnailAttempt>>>,
) -> ThumbnailAttempt {
    let may_warm_cache = cache_keys.path.is_some() || cache_keys.content.is_some();
    if !may_warm_cache && Instant::now() >= request.response_deadline() {
        return ThumbnailAttempt::TransientFailure;
    }
    if !thumbnail_file_source_matches(&path, &source, &metadata) {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail source changed before parsing; reporting transient failure"
        );
        return ThumbnailAttempt::TransientFailure;
    }
    let mesh = match load_thumbnail_mesh_from_opened_file(&path, &source, kind) {
        Ok(mesh) => mesh,
        Err(error) => {
            if !thumbnail_file_source_matches(&path, &source, &metadata) {
                return ThumbnailAttempt::TransientFailure;
            }
            return thumbnail_attempt_for_job_outcome(
                ThumbnailJobOutcome::Finished(Err(error)),
                spec,
                request.response_timeout(),
                "file",
            );
        }
    };
    if !thumbnail_file_source_matches(&path, &source, &metadata) {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail source changed during parsing; reporting transient failure"
        );
        return ThumbnailAttempt::TransientFailure;
    }
    if !may_warm_cache && Instant::now() >= request.response_deadline() {
        return ThumbnailAttempt::TransientFailure;
    }
    if let Some(progress) = &progress {
        let _ = progress.send(ThumbnailJobProgress::Prepared);
    }
    // The parser has consumed the owned primary snapshot. Release up to the
    // file cap before waiting on renderer setup or GPU work.
    drop(source);
    let result = rendering::render_mesh_thumbnail_with_adapter_policy(
        mesh,
        spec,
        RenderDeadline::at(if may_warm_cache {
            request.cache_warm_deadline()
        } else {
            request.response_deadline()
        }),
        request.adapter_policy(),
    );
    if cache::thumbnail_file_changed_since(&path, &metadata) {
        tracing::warn!(
            extension = ?path.extension(),
            "thumbnail source changed during rendering; reporting transient failure"
        );
        return ThumbnailAttempt::TransientFailure;
    }
    if let Ok(pixels) = &result {
        if cache_keys.path.is_some() || cache_keys.content.is_some() {
            cache_file_thumbnail(
                cache_keys.path,
                cache_keys.content,
                spec.size_px,
                thumbnail_background_key(spec.background),
                pixels,
            );
        }
    }
    thumbnail_attempt_for_job_outcome(
        ThumbnailJobOutcome::Finished(result),
        spec,
        request.response_timeout(),
        "file",
    )
}

fn thumbnail_attempt_for_file_worker_outcome(
    outcome: ThumbnailJobOutcome<ThumbnailAttempt>,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
) -> ThumbnailAttempt {
    thumbnail_attempt_for_thumbnail_worker_outcome(outcome, spec, request, "file")
}

fn thumbnail_attempt_for_thumbnail_worker_outcome(
    outcome: ThumbnailJobOutcome<ThumbnailAttempt>,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
    source: &'static str,
) -> ThumbnailAttempt {
    match outcome {
        ThumbnailJobOutcome::Finished(attempt) => {
            if Instant::now() < request.response_deadline() {
                attempt
            } else {
                tracing::warn!(
                    ?request.response_timeout,
                    source,
                    "thumbnail completion arrived after the response deadline; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            }
        }
        ThumbnailJobOutcome::SetupTimedOut => thumbnail_attempt_for_job_outcome(
            ThumbnailJobOutcome::<Result<Vec<u8>, ThumbnailError>>::SetupTimedOut,
            spec,
            request.response_timeout(),
            source,
        ),
        ThumbnailJobOutcome::RenderTimedOut => thumbnail_attempt_for_job_outcome(
            ThumbnailJobOutcome::<Result<Vec<u8>, ThumbnailError>>::RenderTimedOut,
            spec,
            request.response_timeout(),
            source,
        ),
        ThumbnailJobOutcome::Failed => thumbnail_attempt_for_job_outcome(
            ThumbnailJobOutcome::<Result<Vec<u8>, ThumbnailError>>::Failed,
            spec,
            request.response_timeout(),
            source,
        ),
    }
}

/// Translate a worker outcome into the cache-safety split: decode verdicts
/// become pixels, everything a retry could plausibly fix stays a failure.
///
/// A render/GPU error is transient rather than a plain placeholder: a lost
/// device or a driver reset says nothing about the file, and the retry path is
/// cheap because the pool discards the sick renderer and the next attempt gets a fresh one. An I/O
/// error mid-decode is transient for the same reason the metadata preflight
/// is: on Windows the common cause is a sharing violation while the scanner
/// is still writing the file, which the next browse will not reproduce.
fn thumbnail_attempt_for_job_outcome(
    outcome: ThumbnailJobOutcome<Result<Vec<u8>, ThumbnailError>>,
    spec: ThumbnailSpec,
    timeout: Duration,
    source: &'static str,
) -> ThumbnailAttempt {
    match outcome {
        ThumbnailJobOutcome::Finished(Ok(pixels)) => ThumbnailAttempt::Bitmap(pixels),
        ThumbnailJobOutcome::Finished(Err(error)) => match &error {
            // Deferred means this process cannot read a recognized feature,
            // which can depend on its supported formats or configured keys.
            // A cacheable placeholder would survive that process-level change.
            ThumbnailError::Format(FormatError::Deferred { .. }) => {
                tracing::warn!(
                    ?error,
                    source,
                    "thumbnail source needs a supported feature or key unavailable in this process; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            }
            ThumbnailError::Format(FormatError::Io(_)) => {
                tracing::warn!(
                    ?error,
                    source,
                    "thumbnail source I/O failed mid-decode; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            }
            ThumbnailError::Format(_) => {
                let kind = placeholder_kind_for_error(&error);
                tracing::warn!(
                    ?error,
                    ?kind,
                    source,
                    "thumbnail decode failed; returning placeholder verdict"
                );
                ThumbnailAttempt::Bitmap(placeholder_thumbnail_kind(spec, kind))
            }
            ThumbnailError::Render(_) => {
                tracing::warn!(
                    ?error,
                    source,
                    "thumbnail render failed; reporting transient failure"
                );
                ThumbnailAttempt::TransientFailure
            }
        },
        ThumbnailJobOutcome::SetupTimedOut => {
            tracing::warn!(
                ?timeout,
                source,
                "thumbnail exceeded its end-to-end budget before preparation completed; reporting transient failure"
            );
            ThumbnailAttempt::TransientFailure
        }
        ThumbnailJobOutcome::RenderTimedOut => {
            tracing::warn!(
                ?timeout,
                source,
                "thumbnail render timed out after renderer checkout; reporting transient failure"
            );
            ThumbnailAttempt::TransientFailure
        }
        ThumbnailJobOutcome::Failed => {
            tracing::warn!(
                source,
                "thumbnail worker failed; reporting transient failure"
            );
            ThumbnailAttempt::TransientFailure
        }
    }
}

fn cache_file_thumbnail(
    path_cache_key: Option<ThumbnailFileCacheKey>,
    content_cache_key: Option<ThumbnailFileContentKey>,
    size_px: u16,
    background: [u64; 4],
    pixels: &[u8],
) {
    if let Some(path_cache_key) = path_cache_key {
        if let Ok(mut cache) = thumbnail_file_cache().lock() {
            cache.insert_with_background(path_cache_key, size_px, background, pixels);
        }
    }
    if let Some(content_key) = content_cache_key {
        if let Ok(mut cache) = thumbnail_file_content_cache().lock() {
            cache.insert_with_background(content_key, size_px, background, pixels);
        }
    }
    #[cfg(test)]
    notify_thumbnail_file_cache_changed();
}

#[must_use]
/// Render shared stream bytes with a bounded wait and placeholder fallback.
pub fn render_thumbnail_shared_or_placeholder_with_timeout(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    timeout: Duration,
) -> Vec<u8> {
    try_render_thumbnail_shared_with_request(
        extension,
        bytes,
        spec,
        ThumbnailRenderRequest::new(timeout),
    )
    .into_pixels_or_placeholder(spec)
}

#[must_use]
/// Render shared stream bytes with a bounded wait, reporting transient
/// failures instead of masking them as placeholder pixels.
pub fn try_render_thumbnail_shared(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    timeout: Duration,
) -> ThumbnailAttempt {
    try_render_thumbnail_shared_with_request(
        extension,
        bytes,
        spec,
        ThumbnailRenderRequest::new(timeout),
    )
}

/// Render shared stream bytes for a Shell request whose deadline was fixed at
/// entry, reporting transient failures instead of cacheable placeholder pixels.
#[must_use]
pub fn try_render_thumbnail_shared_with_request(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
) -> ThumbnailAttempt {
    try_render_thumbnail_shared_impl(extension, bytes, spec, request, None)
}

#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
/// Render shared stream bytes using a previously acquired job reservation,
/// reporting transient failures instead of masking them as placeholder pixels.
pub fn try_render_thumbnail_shared_with_reservation(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    reservation: ThumbnailJobReservation,
) -> ThumbnailAttempt {
    try_render_thumbnail_shared_impl(
        extension,
        bytes,
        spec,
        reservation.request,
        Some(reservation),
    )
}

fn try_render_thumbnail_shared_impl(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
    reservation: Option<ThumbnailJobReservation>,
) -> ThumbnailAttempt {
    if Instant::now() >= request.response_deadline() {
        return ThumbnailAttempt::TransientFailure;
    }

    let worker_request = reservation.as_ref().map_or(request, |reserved| {
        debug_assert_eq!(
            request.response_deadline(),
            reserved.request.response_deadline(),
            "the copied stream must keep the reservation's original response deadline"
        );
        reserved.request
    });
    let work = move |progress| {
        render_stream_thumbnail_worker(
            extension,
            bytes,
            spec,
            worker_request,
            progress,
            prepare_stream_thumbnail_render,
        );
    };
    let outcome = match reservation {
        Some(reserved) => run_thumbnail_job_by(
            reserved.permit,
            reserved.request.response_deadline(),
            work,
        ),
        None => run_thumbnail_job_by_deadline(request.response_deadline(), work),
    };
    thumbnail_attempt_for_thumbnail_worker_outcome(outcome, spec, worker_request, "stream")
}

fn render_stream_thumbnail_worker<F>(
    extension: Option<String>,
    bytes: Arc<[u8]>,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
    progress: std::sync::mpsc::SyncSender<ThumbnailJobProgress<ThumbnailAttempt>>,
    prepare: F,
) where
    F: FnOnce(
        Option<&str>,
        &[u8],
    ) -> Result<cache::StreamThumbnailRenderPlan, StreamThumbnailPreflightError>,
{
    // Probing and hashing are deliberately inside this bounded worker. If the
    // response expires during either step, the worker keeps its permit and the
    // post-preflight fence prevents even a cache hit from answering late.
    let preflight = prepare(extension.as_deref(), bytes.as_ref());
    if Instant::now() >= request.response_deadline() {
        let _ = progress.send(ThumbnailJobProgress::Finished(
            ThumbnailAttempt::TransientFailure,
        ));
        return;
    }
    let plan = match preflight {
        Ok(plan) => plan,
        Err(StreamThumbnailPreflightError::Oversize { byte_len }) => {
            let _ = progress.send(ThumbnailJobProgress::Finished(ThumbnailAttempt::Bitmap(
                placeholder_for_oversize_input(spec, byte_len),
            )));
            return;
        }
        Err(StreamThumbnailPreflightError::Format(error)) => {
            // Deterministic: neither magic bytes nor the extension hint mapped
            // these bytes to a reader we ship. Retrying cannot change that.
            tracing::warn!(?error, "thumbnail stream format inference failed; returning placeholder");
            let _ = progress.send(ThumbnailJobProgress::Finished(ThumbnailAttempt::Bitmap(
                placeholder_thumbnail(spec),
            )));
            return;
        }
    };

    let cached = thumbnail_stream_cache().lock().ok().and_then(|mut cache| {
        cache.get_with_background(
            &plan.cache_key,
            spec.size_px,
            thumbnail_background_key(spec.background),
        )
    });
    if let Some(pixels) = cached {
        let _ = progress.send(ThumbnailJobProgress::Finished(ThumbnailAttempt::Bitmap(
            pixels,
        )));
        return;
    }

    let inflight_key = ThumbnailRequestKey::Stream {
        cache_key: plan.cache_key.clone(),
        size_px: spec.size_px,
        background: thumbnail_background_key(spec.background),
    };
    let cache_key_for_store = plan.cache_key;
    let kind = plan.kind;
    let progress_for_render = progress.clone();
    let attempt = render_coalesced_thumbnail_by(
        inflight_key,
        request.response_deadline(),
        move || {
            let result = (|| -> Result<Vec<u8>, ThumbnailError> {
                let mesh = load_thumbnail_mesh_from_bytes_kind(kind, bytes.as_ref())?;
                let _ = progress_for_render.send(ThumbnailJobProgress::Prepared);
                rendering::render_mesh_thumbnail_with_adapter_policy(
                    mesh,
                    spec,
                    RenderDeadline::at(request.cache_warm_deadline()),
                    request.adapter_policy(),
                )
            })();
            // The worker may outlive the caller's response deadline while it warms
            // this bounded cache for the next repaint.
            if let Ok(pixels) = &result {
                if let Ok(mut cache) = thumbnail_stream_cache().lock() {
                    cache.insert_with_background(
                        cache_key_for_store,
                        spec.size_px,
                        thumbnail_background_key(spec.background),
                        pixels,
                    );
                }
            }
            thumbnail_attempt_for_job_outcome(
                ThumbnailJobOutcome::Finished(result),
                spec,
                request.response_timeout(),
                "stream",
            )
        },
    );
    let _ = progress.send(ThumbnailJobProgress::Finished(attempt));
}

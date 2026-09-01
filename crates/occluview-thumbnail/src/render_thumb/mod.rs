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
    load_thumbnail_mesh_from_file, prepare_file_thumbnail_render, prepare_stream_thumbnail_render,
};

/// Default maximum wall-clock wait for a shell thumbnail request.
pub const DEFAULT_THUMBNAIL_TIMEOUT: Duration = Duration::from_millis(6_000);
/// Maximum GPU/render time for an already-detached cache warmer.
///
/// A caller that has exhausted its response budget must never receive the
/// warmer's result. Keeping this separately bounded preserves the useful
/// second-paint cache behaviour without silently extending an Explorer COM
/// call or leaving an unbounded worker behind.
const BACKGROUND_CACHE_WARM_TIMEOUT: Duration = DEFAULT_THUMBNAIL_TIMEOUT;

/// Immutable timing and adapter policy for one thumbnail request.
///
/// Both absolute deadlines are fixed when the Shell request enters the
/// pipeline. The response deadline bounds everything that may answer COM. A
/// worker that outlives that response may write only to the process cache,
/// using the separately bounded cache-warm deadline captured at the same
/// instant.
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
/// Maximum local-file thumbnail input size. File-backed thumbnails use mmap,
/// so this policy can be higher than the stream cap without duplicating the
/// file into the COM surrogate's heap.
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

/// Outcome of one thumbnail request against the shared pipeline.
///
/// The distinction exists because Explorer's thumbnail cache permanently
/// stores ANY bitmap a provider returns with `S_OK`, keyed by the file's
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
    /// Fixed at reservation time, so that the wait for a slot, the shell's
    /// copy of the stream and the render itself all spend one budget. They
    /// used to take a budget each: eight seconds for the slot, an unbounded
    /// copy, then a fresh six for the render -- and under Apartment hosting
    /// the whole folder queues behind that.
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

/// Load a local file via the shared mmap-backed reader and render a thumbnail.
///
/// This path is preferred for Explorer `IInitializeWithFile` /
/// `IInitializeWithItem` initialization because it keeps the extension hint
/// for HPS and avoids an extra full-file copy for large STL/PLY/OBJ files.
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
/// timeout must NOT become a bitmap when the caller is Explorer's thumbnail
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
    let plan = match prepare_file_thumbnail_render(path) {
        Ok(plan) => plan,
        Err(FileThumbnailPreflightError::UnsupportedExtension) => {
            // Deterministic: the extension is simply not ours. This repeats on
            // every attempt, so the placeholder is a correct cacheable verdict.
            // Extension, not path: this runs inside dllhost over whatever
            // folder Explorer is showing, and a scan's file name is the case.
            tracing::warn!(
                extension = ?path.extension(),
                "thumbnail file extension is not registered for OccluView; returning placeholder"
            );
            return ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec));
        }
        Err(FileThumbnailPreflightError::Metadata(error)) => {
            // Transient: the common causes are a sharing violation while the
            // scanner is still writing the file, or a file that vanished
            // mid-browse. Neither is a verdict about the file's content.
            tracing::warn!(?error, extension = ?path.extension(), "thumbnail file metadata failed");
            return ThumbnailAttempt::TransientFailure;
        }
        Err(FileThumbnailPreflightError::NotAFile) => {
            // Deterministic: a directory or a named pipe does not become a
            // scan by being asked twice. Refusing here is also what keeps the
            // open below from blocking on a pipe with no writer -- which it
            // would do on the caller's thread, before any deadline applies.
            tracing::warn!(
                extension = ?path.extension(),
                "thumbnail path is not a regular file; returning placeholder"
            );
            return ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec));
        }
        Err(FileThumbnailPreflightError::Oversize { byte_len }) => {
            return ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len));
        }
    };

    // Do not trust the metadata key as the first lookup. Some file systems
    // preserve both byte length and coarse mtime when a file is replaced; an
    // early path-cache hit would then show the previous mesh indefinitely.
    // Content hashing is bounded and also deduplicates copied CAD exports.
    let (content_key, content_hit) = file_content_cache_lookup(path, &plan, spec);
    if let Some(pixels) = content_hit {
        return ThumbnailAttempt::Bitmap(pixels);
    }

    let inflight_key = match content_key.clone() {
        Some(cache_key) => ThumbnailRequestKey::FileContent {
            cache_key,
            size_px: spec.size_px,
            background: thumbnail_background_key(spec.background),
        },
        None => ThumbnailRequestKey::File {
            cache_key: plan.cache_key.clone(),
            size_px: spec.size_px,
            background: thumbnail_background_key(spec.background),
        },
    };
    let path_cache_key = plan.cache_key;
    let metadata = plan.metadata;
    let path = path.to_path_buf();
    let cache_keys = FileThumbnailCacheKeys {
        path: path_cache_key,
        content: content_key,
    };
    render_coalesced_thumbnail_by(inflight_key, request.response_deadline(), move || {
        render_file_thumbnail_job(path, metadata, cache_keys, spec, request)
    })
}

fn file_content_cache_lookup(
    path: &Path,
    plan: &cache::FileThumbnailRenderPlan,
    spec: ThumbnailSpec,
) -> (Option<ThumbnailFileContentKey>, Option<Vec<u8>>) {
    // If the file changes while Explorer is probing it, hashing is merely an
    // optimization failure: fall back to the path key and keep the contract.
    let content_key = thumbnail_file_content_key(path, &plan.metadata).ok();
    let Some(key) = content_key.as_ref() else {
        return (None, None);
    };
    let Some(pixels) = thumbnail_file_content_cache()
        .lock()
        .ok()
        .and_then(|mut cache| {
            cache.get_with_background(key, spec.size_px, thumbnail_background_key(spec.background))
        })
    else {
        return (content_key, None);
    };
    if let Ok(mut path_cache) = thumbnail_file_cache().lock() {
        path_cache.insert_with_background(
            plan.cache_key.clone(),
            spec.size_px,
            thumbnail_background_key(spec.background),
            &pixels,
        );
    }
    (content_key, Some(pixels))
}

struct FileThumbnailCacheKeys {
    path: ThumbnailFileCacheKey,
    content: Option<ThumbnailFileContentKey>,
}

fn render_file_thumbnail_job(
    path: PathBuf,
    metadata: ThumbnailFileMetadata,
    cache_keys: FileThumbnailCacheKeys,
    spec: ThumbnailSpec,
    request: ThumbnailRenderRequest,
) -> ThumbnailAttempt {
    let measured_path = path.clone();
    let result = run_thumbnail_job_by_deadline(request.response_deadline(), move |progress| {
        let result = (|| -> Result<Vec<u8>, ThumbnailError> {
            let mesh = load_thumbnail_mesh_from_file(&path, metadata)?;
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            rendering::render_mesh_thumbnail_with_adapter_policy(
                mesh,
                spec,
                RenderDeadline::at(request.cache_warm_deadline()),
                request.adapter_policy(),
            )
        })();
        if let Ok(pixels) = &result {
            cache_file_thumbnail(
                cache_keys.path,
                cache_keys.content,
                spec.size_px,
                thumbnail_background_key(spec.background),
                pixels,
            );
        }
        let _ = progress.send(ThumbnailJobProgress::Finished(result));
    });

    // "Truncated" is a verdict about the file, and the shell keeps verdicts
    // forever. When the file is still being written -- a scanner exporting into
    // the folder the operator is looking at -- it is a verdict about the
    // moment, and the badge would outlive the cause. The preflight measured
    // this file; if it no longer matches, say so instead.
    if let ThumbnailJobOutcome::Finished(Err(ThumbnailError::Format(FormatError::Truncated {
        ..
    }))) = &result
    {
        if cache::thumbnail_file_changed_since(&measured_path, &metadata) {
            tracing::warn!(
                extension = ?measured_path.extension(),
                "thumbnail source changed while it was being read; reporting transient failure"
            );
            return ThumbnailAttempt::TransientFailure;
        }
    }
    thumbnail_attempt_for_job_outcome(result, spec, request.response_timeout(), "file")
}

/// Translate a worker outcome into the cache-safety split: decode verdicts
/// become pixels, everything a retry could plausibly fix stays a failure.
///
/// A render/GPU error is deliberately on the transient side even though the
/// old behavior painted a plain placeholder: a lost device or a driver reset
/// says nothing about the file, and the retry path is cheap because the pool
/// discards the sick renderer and the next attempt gets a fresh one. An I/O
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
            // Whether a key is configured is a property of this process, not
            // of the file: an encrypted package that cannot be opened today
            // opens tomorrow, on a build that has the key or a machine where
            // the environment carries one. A cacheable placeholder would
            // survive that change -- the shell keeps verdicts against the
            // file's timestamp, which does not move when a key appears -- and
            // every encrypted scan in every folder would keep the grey cube.
            ThumbnailError::Format(FormatError::Deferred { .. }) => {
                tracing::warn!(
                    ?error,
                    source,
                    "thumbnail source needs a key this process does not have; \
                     reporting transient failure"
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
    path_cache_key: ThumbnailFileCacheKey,
    content_cache_key: Option<ThumbnailFileContentKey>,
    size_px: u16,
    background: [u64; 4],
    pixels: &[u8],
) {
    if let Ok(mut cache) = thumbnail_file_cache().lock() {
        cache.insert_with_background(path_cache_key, size_px, background, pixels);
    }
    if let Some(content_key) = content_cache_key {
        if let Ok(mut cache) = thumbnail_file_content_cache().lock() {
            cache.insert_with_background(content_key, size_px, background, pixels);
        }
    }
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
    let plan = match prepare_stream_thumbnail_render(extension.as_deref(), bytes.as_ref()) {
        Ok(plan) => plan,
        Err(StreamThumbnailPreflightError::Oversize { byte_len }) => {
            return ThumbnailAttempt::Bitmap(placeholder_for_oversize_input(spec, byte_len));
        }
        Err(StreamThumbnailPreflightError::Format(error)) => {
            // Deterministic: neither magic bytes nor the extension hint mapped
            // these bytes to a reader we ship. Retrying cannot change that.
            tracing::warn!(
                ?error,
                "thumbnail stream format inference failed before worker startup; returning placeholder"
            );
            return ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec));
        }
    };
    if let Ok(mut cache) = thumbnail_stream_cache().lock() {
        if let Some(pixels) = cache.get_with_background(
            &plan.cache_key,
            spec.size_px,
            thumbnail_background_key(spec.background),
        ) {
            return ThumbnailAttempt::Bitmap(pixels);
        }
    }

    let inflight_key = ThumbnailRequestKey::Stream {
        cache_key: plan.cache_key.clone(),
        size_px: spec.size_px,
        background: thumbnail_background_key(spec.background),
    };
    render_coalesced_thumbnail_by(inflight_key, request.response_deadline(), move || {
        let cache_key_for_store = plan.cache_key.clone();
        let kind = plan.kind;
        let work = move |progress: std::sync::mpsc::SyncSender<
            ThumbnailJobProgress<Result<Vec<u8>, ThumbnailError>>,
        >| {
            let result = (|| -> Result<Vec<u8>, ThumbnailError> {
                let mesh = load_thumbnail_mesh_from_bytes_kind(kind, bytes.as_ref())?;
                let _ = progress.send(ThumbnailJobProgress::Prepared);
                rendering::render_mesh_thumbnail_with_adapter_policy(
                    mesh,
                    spec,
                    RenderDeadline::at(request.cache_warm_deadline()),
                    request.adapter_policy(),
                )
            })();
            // See the file path: cache from the worker so a render that
            // outran the caller's deadline still lands in the cache for the
            // next repaint instead of being thrown away.
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
            let _ = progress.send(ThumbnailJobProgress::Finished(result));
        };
        let result = match reservation {
            Some(ThumbnailJobReservation {
                permit,
                request: reserved_request,
            }) => {
                debug_assert_eq!(
                    request.response_deadline(),
                    reserved_request.response_deadline(),
                    "the copied stream must keep the reservation's original response deadline"
                );
                run_thumbnail_job_by(permit, reserved_request.response_deadline(), work)
            }
            None => run_thumbnail_job_by_deadline(request.response_deadline(), work),
        };

        thumbnail_attempt_for_job_outcome(result, spec, request.response_timeout(), "stream")
    })
}

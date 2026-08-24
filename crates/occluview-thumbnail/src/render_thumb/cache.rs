use super::{
    rendering, Duration, Mutex, Path, ThumbnailError, MAX_THUMBNAIL_FILE_BYTES,
    MAX_THUMBNAIL_INPUT_BYTES, MAX_THUMBNAIL_SETUP_TIMEOUT, THUMBNAIL_FILE_CACHE,
    THUMBNAIL_FILE_CONTENT_CACHE, THUMBNAIL_STREAM_CACHE,
};
use occluview_formats::{FormatError, FormatKind};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

const MAX_CACHED_FILE_THUMBNAILS: usize = 96;
const MAX_CACHED_FILE_THUMBNAIL_BYTES: usize = 32 * 1024 * 1024;
const EXACT_CONTENT_HASH_BYTES: u64 = 16 * 1024 * 1024;
const EXACT_CONTENT_HASH_BYTES_USIZE: usize = 16 * 1024 * 1024;
const CONTENT_HASH_SAMPLE_BYTES: u64 = 64 * 1024;
const CONTENT_HASH_SAMPLE_BYTES_USIZE: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum ThumbnailRequestKey {
    File {
        cache_key: ThumbnailFileCacheKey,
        size_px: u16,
        background: [u64; 4],
    },
    FileContent {
        cache_key: ThumbnailFileContentKey,
        size_px: u16,
        background: [u64; 4],
    },
    Stream {
        cache_key: ThumbnailStreamCacheKey,
        size_px: u16,
        background: [u64; 4],
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ThumbnailFileCacheKey {
    pub(super) path: PathBuf,
    pub(super) byte_len: u64,
    pub(super) modified_nanos: u128,
}

/// Content identity used to share work between different paths containing the
/// same mesh. The hash is exact for ordinary small shell inputs and sampled for
/// very large files so opening a folder never turns metadata preflight into a
/// second full-file read. The format tag and byte length prevent cross-format
/// or truncated-file reuse.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ThumbnailFileContentKey {
    format_tag: u8,
    byte_len: u64,
    fingerprint: [u8; 32],
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ThumbnailStreamCacheKey {
    kind_tag: u8,
    byte_len: usize,
    fingerprint: [u8; 32],
}

#[derive(Clone, Debug)]
struct CachedThumbnail {
    size_px: u16,
    background: [u64; 4],
    pixels: Vec<u8>,
}

pub(super) struct ThumbnailCache<K> {
    order: VecDeque<K>,
    entries: HashMap<K, Vec<CachedThumbnail>>,
    total_bytes: usize,
    max_files: usize,
    max_bytes: usize,
}

pub(super) type ThumbnailFileCache = ThumbnailCache<ThumbnailFileCacheKey>;
pub(super) type ThumbnailFileContentCache = ThumbnailCache<ThumbnailFileContentKey>;
pub(super) type ThumbnailStreamCache = ThumbnailCache<ThumbnailStreamCacheKey>;

#[derive(Clone, Copy, Debug)]
pub(super) struct ThumbnailFileMetadata {
    pub(super) byte_len: u64,
    pub(super) modified_nanos: u128,
}

#[derive(Debug)]
pub(super) struct FileThumbnailRenderPlan {
    pub(super) metadata: ThumbnailFileMetadata,
    pub(super) cache_key: ThumbnailFileCacheKey,
    pub(super) wait_timeout: Duration,
}

#[derive(Debug)]
pub(super) enum FileThumbnailPreflightError {
    UnsupportedExtension,
    Metadata(ThumbnailError),
    /// The path names something that is not a regular file: a directory, a
    /// named pipe, a device. Opening a pipe with no writer blocks forever.
    NotAFile,
    Oversize {
        byte_len: usize,
    },
}

#[derive(Debug)]
pub(super) struct StreamThumbnailRenderPlan {
    pub(super) kind: FormatKind,
    pub(super) cache_key: ThumbnailStreamCacheKey,
    pub(super) wait_timeout: Duration,
}

#[derive(Debug)]
pub(super) enum StreamThumbnailPreflightError {
    Oversize { byte_len: usize },
    Format(FormatError),
}

impl ThumbnailFileCacheKey {
    pub(super) fn new(path: &Path, metadata: &ThumbnailFileMetadata) -> Self {
        Self {
            path: path.to_path_buf(),
            byte_len: metadata.byte_len,
            modified_nanos: metadata.modified_nanos,
        }
    }
}

impl<K> ThumbnailCache<K>
where
    K: Clone + Eq + Hash,
{
    pub(super) fn new(max_files: usize, max_bytes: usize) -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            total_bytes: 0,
            max_files,
            max_bytes,
        }
    }

    #[cfg(test)]
    pub(super) fn get(&mut self, key: &K, size_px: u16) -> Option<Vec<u8>> {
        self.get_with_background(key, size_px, [0; 4])
    }

    pub(super) fn get_with_background(
        &mut self,
        key: &K,
        size_px: u16,
        background: [u64; 4],
    ) -> Option<Vec<u8>> {
        let pixels = self.entries.get(key).and_then(|thumbnails| {
            if let Some(exact) = thumbnails
                .iter()
                .find(|thumb| thumb.size_px == size_px && thumb.background == background)
            {
                return Some(exact.pixels.clone());
            }
            thumbnails
                .iter()
                .filter(|thumb| {
                    thumb.background == background
                        && size_px > 0
                        && thumb.size_px > size_px
                        && thumb.size_px % size_px == 0
                })
                .min_by_key(|thumb| thumb.size_px)
                .map(|thumb| {
                    rendering::downsample_rgba_premultiplied(&thumb.pixels, thumb.size_px, size_px)
                })
        })?;
        self.touch(key.clone());
        Some(pixels)
    }

    #[cfg(test)]
    pub(super) fn insert(&mut self, key: K, size_px: u16, pixels: &[u8]) {
        self.insert_with_background(key, size_px, [0; 4], pixels);
    }

    pub(super) fn insert_with_background(
        &mut self,
        key: K,
        size_px: u16,
        background: [u64; 4],
        pixels: &[u8],
    ) {
        let thumbnails = self.entries.entry(key.clone()).or_default();
        if let Some(existing) = thumbnails
            .iter_mut()
            .find(|thumb| thumb.size_px == size_px && thumb.background == background)
        {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(existing.pixels.len())
                .saturating_add(pixels.len());
            existing.pixels.clear();
            existing.pixels.extend_from_slice(pixels);
        } else {
            self.total_bytes = self.total_bytes.saturating_add(pixels.len());
            thumbnails.push(CachedThumbnail {
                size_px,
                background,
                pixels: pixels.to_vec(),
            });
            thumbnails.sort_by_key(|thumb| thumb.size_px);
        }
        self.touch(key);
        self.evict_to_budget();
    }

    fn touch(&mut self, key: K) {
        if let Some(position) = self.order.iter().position(|existing| existing == &key) {
            let _ = self.order.remove(position);
        }
        self.order.push_back(key);
    }

    fn evict_to_budget(&mut self) {
        while self.order.len() > self.max_files || self.total_bytes > self.max_bytes {
            let Some(oldest_key) = self.order.pop_front() else {
                break;
            };
            if let Some(thumbnails) = self.entries.remove(&oldest_key) {
                for thumbnail in thumbnails {
                    self.total_bytes = self.total_bytes.saturating_sub(thumbnail.pixels.len());
                }
            }
        }
    }
}

pub(super) fn thumbnail_background_key(background: [f64; 4]) -> [u64; 4] {
    [
        background[0].to_bits(),
        background[1].to_bits(),
        background[2].to_bits(),
        background[3].to_bits(),
    ]
}

impl Default for ThumbnailFileCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_FILE_THUMBNAILS, MAX_CACHED_FILE_THUMBNAIL_BYTES)
    }
}

impl Default for ThumbnailFileContentCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_FILE_THUMBNAILS, MAX_CACHED_FILE_THUMBNAIL_BYTES)
    }
}

impl Default for ThumbnailStreamCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_FILE_THUMBNAILS, MAX_CACHED_FILE_THUMBNAIL_BYTES)
    }
}

pub(super) fn thumbnail_file_cache() -> &'static Mutex<ThumbnailFileCache> {
    THUMBNAIL_FILE_CACHE.get_or_init(|| Mutex::new(ThumbnailFileCache::default()))
}

pub(super) fn thumbnail_file_content_cache() -> &'static Mutex<ThumbnailFileContentCache> {
    THUMBNAIL_FILE_CONTENT_CACHE.get_or_init(|| Mutex::new(ThumbnailFileContentCache::default()))
}

pub(super) fn thumbnail_stream_cache() -> &'static Mutex<ThumbnailStreamCache> {
    THUMBNAIL_STREAM_CACHE.get_or_init(|| Mutex::new(ThumbnailStreamCache::default()))
}

pub(super) fn thumbnail_file_metadata(
    path: &Path,
) -> Result<ThumbnailFileMetadata, ThumbnailError> {
    thumbnail_file_metadata_checked(path).map(|(metadata, _)| metadata)
}

/// The metadata, and whether the path names a regular file.
///
/// The caller needs the second answer before it opens anything: a folder
/// holding `pipe.stl` with no writer on the other end blocks the opening
/// thread for as long as the process lives, and under the shell that thread is
/// the one every other file in the folder is queued behind.
pub(super) fn thumbnail_file_metadata_checked(
    path: &Path,
) -> Result<(ThumbnailFileMetadata, bool), ThumbnailError> {
    let metadata = std::fs::metadata(path).map_err(|e| file_io_error(path, e))?;
    let is_regular_file = metadata.is_file();
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok((
        ThumbnailFileMetadata {
            byte_len: metadata.len(),
            modified_nanos,
        },
        is_regular_file,
    ))
}

/// Build a bounded content key for a file-backed thumbnail.
///
/// The content key lets Explorer's common copy-heavy folders (for example,
/// exported CAD projects) share one decode and one GPU render across
/// differently named files. Callers compute it before consulting the cheap
/// path cache so same-size replacements cannot return stale pixels.
pub(super) fn thumbnail_file_content_key(
    path: &Path,
    metadata: &ThumbnailFileMetadata,
) -> Result<ThumbnailFileContentKey, ThumbnailError> {
    let mut file = std::fs::File::open(path).map_err(|e| file_io_error(path, e))?;
    let mut hasher = Sha256::new();
    let format_tag = thumbnail_file_format_tag(path);
    hasher.update([format_tag]);
    hasher.update(metadata.byte_len.to_le_bytes());

    if metadata.byte_len <= EXACT_CONTENT_HASH_BYTES {
        hasher.update([0u8]);
        hash_file_range(&mut file, &mut hasher, 0, metadata.byte_len)
            .map_err(|e| file_io_error(path, e))?;
    } else {
        // Hash three labelled, position-aware windows. The labels prevent
        // ambiguous concatenations and the offsets make equal windows at
        // different positions distinct.
        for (label, start) in [
            (b"head".as_slice(), 0),
            (b"middle".as_slice(), metadata.byte_len / 2),
            (
                b"tail".as_slice(),
                metadata.byte_len.saturating_sub(CONTENT_HASH_SAMPLE_BYTES),
            ),
        ] {
            let length = CONTENT_HASH_SAMPLE_BYTES.min(metadata.byte_len.saturating_sub(start));
            hasher.update(label);
            hasher.update(start.to_le_bytes());
            hasher.update(length.to_le_bytes());
            hash_file_range(&mut file, &mut hasher, start, length)
                .map_err(|e| file_io_error(path, e))?;
        }
    }

    Ok(ThumbnailFileContentKey {
        format_tag,
        byte_len: metadata.byte_len,
        fingerprint: hasher.finalize().into(),
    })
}

fn hash_file_range(
    file: &mut std::fs::File,
    hasher: &mut Sha256,
    start: u64,
    length: u64,
) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = length;
    let mut buffer = [0u8; 16 * 1024];
    while remaining > 0 {
        let request = usize::try_from(remaining)
            .unwrap_or(buffer.len())
            .min(buffer.len());
        let read = file.read(&mut buffer[..request])?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        remaining = remaining.saturating_sub(u64::try_from(read).unwrap_or(u64::MAX));
    }
    Ok(())
}

fn thumbnail_file_format_tag(path: &Path) -> u8 {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("stl") => 1,
        Some("ply") => 2,
        Some("obj") => 3,
        Some("glb" | "gltf") => 4,
        Some("3mf") => 5,
        Some("off") => 6,
        Some(extension)
            if extension == occluview_formats::LEGACY_HPS_EXTENSION || extension == "hps" =>
        {
            7
        }
        _ => 0,
    }
}

fn file_io_error(path: &Path, error: std::io::Error) -> ThumbnailError {
    ThumbnailError::Format(FormatError::Io(std::io::Error::new(
        error.kind(),
        format!("{}: {error}", path.display()),
    )))
}

pub(super) fn oversize_input_error(byte_len: usize) -> ThumbnailError {
    ThumbnailError::Format(FormatError::Malformed {
        format: "thumbnail stream",
        offset: MAX_THUMBNAIL_INPUT_BYTES,
        reason: format!(
            "shell thumbnail input exceeds {MAX_THUMBNAIL_INPUT_BYTES} bytes (got {byte_len})"
        ),
    })
}

pub(super) fn oversize_file_error(byte_len: usize) -> ThumbnailError {
    ThumbnailError::Format(FormatError::Malformed {
        format: "thumbnail file",
        offset: MAX_THUMBNAIL_FILE_BYTES,
        reason: format!(
            "shell thumbnail file exceeds {MAX_THUMBNAIL_FILE_BYTES} bytes (got {byte_len})"
        ),
    })
}

impl ThumbnailStreamCacheKey {
    pub(super) fn new(kind: FormatKind, bytes: &[u8]) -> Self {
        Self {
            kind_tag: thumbnail_kind_tag(kind),
            byte_len: bytes.len(),
            fingerprint: thumbnail_bytes_fingerprint(bytes),
        }
    }
}

fn thumbnail_kind_tag(kind: FormatKind) -> u8 {
    match kind {
        FormatKind::Stl => 1,
        FormatKind::Ply => 2,
        FormatKind::Obj => 3,
        FormatKind::Gltf => 4,
        FormatKind::Threemf => 5,
        FormatKind::Off => 6,
        FormatKind::Hps => 7,
    }
}

fn thumbnail_bytes_fingerprint(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    if bytes.len() <= EXACT_CONTENT_HASH_BYTES_USIZE {
        hasher.update([0u8]);
        hasher.update(bytes);
    } else {
        let sample = CONTENT_HASH_SAMPLE_BYTES_USIZE;
        for (label, start) in [
            (b"head".as_slice(), 0usize),
            (b"middle".as_slice(), bytes.len() / 2),
            (b"tail".as_slice(), bytes.len().saturating_sub(sample)),
        ] {
            let end = start.saturating_add(sample).min(bytes.len());
            hasher.update(label);
            hasher.update(u64::try_from(start).unwrap_or(u64::MAX).to_le_bytes());
            hasher.update(
                u64::try_from(end.saturating_sub(start))
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            hasher.update(&bytes[start..end]);
        }
    }
    hasher.finalize().into()
}

pub(super) fn thumbnail_setup_timeout(render_timeout: Duration) -> Duration {
    render_timeout
        .checked_mul(4)
        .unwrap_or(MAX_THUMBNAIL_SETUP_TIMEOUT)
        .min(MAX_THUMBNAIL_SETUP_TIMEOUT)
}

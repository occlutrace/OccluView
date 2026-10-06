//! File input, memory admission, and scene assembly for format readers.

use crate::dispatch::{
    dispatch_by_extension_loaded_with_companion_budget, dispatch_by_extension_with_key_provider,
    dispatch_by_kind_loaded_admitted, probe_kind, LoadedMesh,
};
use crate::error::FormatError;
use crate::hps::HpsKeyProvider;
use crate::memory::{
    check_scene_estimate, estimate_file_peak_bytes, SCENE_IMPORT_MEMORY_BUDGET_BYTES,
};
use crate::probe::FormatKind;
use occluview_core::{Mesh, Scene, SceneMesh};
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest single file the viewer will read into memory.
///
/// The number comes from the corpus, not from a round figure: the largest real
/// scan in the test corpus is a 41 MB intraoral OBJ, and dental
/// packages with embedded textures reach a few hundred MB. A gigabyte is
/// therefore ~25x the largest known scan — far enough that no real scan is
/// refused, close enough that a mistaken pick (a video, a disk image, a
/// multi-gigabyte CBCT export) fails in the reader instead of in the
/// allocator. It is also above the shell thumbnail's own 512 MiB file cap, so
/// the viewer never refuses a file the Explorer preview is willing to render.
pub const MAX_IMPORT_BYTES: u64 = 1 << 30;

/// Bytes of file data that may be in flight while a multi-file import parses.
///
/// `MAX_IMPORT_BYTES` bounds one file; a folder of them is the other half of
/// the same problem. Two arch scans of 250 MB each parse together; a single
/// gigabyte file parses alone, because a batch always accepts its first file.
pub const IMPORT_BATCH_BUDGET_BYTES: u64 = 512 << 20;

/// Files parsed at once during a multi-file import.
///
/// The cores are shared with the renderer and with whatever else the operator
/// is doing, and parsing is memory-hungry rather than CPU-hungry, so two is the
/// point where a second file is worth it and more are not.
pub const IMPORT_PARALLELISM: usize = 2;

type PathResult<T> = Result<T, (PathBuf, FormatError)>;

struct AdmittedInput {
    path: PathBuf,
    bytes: FileBytes,
    kind: FormatKind,
}

/// Owned file bytes. Parsing must not depend on a file that another process
/// may replace or truncate while the import is in progress.
pub struct FileBytes {
    extension: String,
    bytes: Vec<u8>,
}

impl FileBytes {
    /// Return the normalized lowercase extension used for dispatch.
    #[must_use]
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// Borrow the file contents as a byte slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Dispatch the loaded bytes through the canonical format readers.
    ///
    /// # Errors
    /// See [`crate::dispatch_by_extension`].
    pub fn dispatch(&self) -> Result<Mesh, FormatError> {
        self.dispatch_with_key_provider(&crate::hps::NoHpsKeyProvider)
    }

    /// Dispatch the loaded bytes through the canonical format readers with an
    /// HPS key provider.
    ///
    /// # Errors
    /// See [`dispatch_by_extension_with_key_provider`].
    pub fn dispatch_with_key_provider(
        &self,
        key_provider: &dyn HpsKeyProvider,
    ) -> Result<Mesh, FormatError> {
        dispatch_by_extension_with_key_provider(self.extension(), self.as_slice(), key_provider)
    }
}

fn normalized_extension(path: &Path) -> Result<String, FormatError> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(FormatError::Unsupported {
            extension: String::new(),
        })
}

/// Read a file into owned bytes, refusing anything above [`MAX_IMPORT_BYTES`].
///
/// A concurrent truncation during the read may yield a parse error, but a
/// later truncation cannot invalidate this buffer.
///
/// # Errors
/// - [`FormatError::Io`] if the file cannot be opened or read.
/// - [`FormatError::TooLarge`] if the file exceeds the limit.
/// - [`FormatError::Unsupported`] when the file has no UTF-8 extension.
pub fn read_file_bytes(path: &Path) -> Result<FileBytes, FormatError> {
    read_file_bytes_with_limit(path, MAX_IMPORT_BYTES)
}

/// As [`read_file_bytes`], with a caller-chosen limit.
///
/// The thumbnail host passes its own, smaller budget: it runs inside Explorer,
/// where an over-large read costs more than a missing preview.
///
/// The size is checked twice. The metadata check refuses the ordinary case
/// before a byte is read; the length check after the read covers a file that
/// grew between the two, which is the window a hostile or merely busy writer
/// would use.
///
/// # Errors
/// See [`read_file_bytes`].
pub fn read_file_bytes_with_limit(path: &Path, limit: u64) -> Result<FileBytes, FormatError> {
    let extension = normalized_extension(path)?;
    let file = std::fs::File::open(path).map_err(FormatError::Io)?;
    let metadata = file.metadata().map_err(FormatError::Io)?;
    if metadata.len() > limit {
        return Err(FormatError::TooLarge {
            bytes: metadata.len(),
            limit,
        });
    }
    let capacity = usize::try_from(metadata.len()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    let mut reader = file.take(limit.saturating_add(1));
    reader.read_to_end(&mut bytes).map_err(FormatError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(FormatError::TooLarge {
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            limit,
        });
    }
    Ok(FileBytes { extension, bytes })
}

/// Group files into the batches a multi-file import parses together.
///
/// A batch takes files while it has room for their bytes and has not reached
/// the concurrency limit. The first file always joins, so a file larger than
/// the budget is parsed on its own rather than refused: the per-file limit is
/// what decides whether it may be read at all.
pub(crate) fn import_batches(
    sizes: &[u64],
    budget: u64,
    parallelism: usize,
) -> Vec<std::ops::Range<usize>> {
    let parallelism = parallelism.max(1);
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = 0_u64;
    for (index, size) in sizes.iter().enumerate() {
        let is_first = index == start;
        let full = index - start >= parallelism;
        let over_budget = !is_first && bytes.saturating_add(*size) > budget;
        if full || over_budget {
            batches.push(start..index);
            start = index;
            bytes = 0;
        }
        bytes = bytes.saturating_add(*size);
    }
    if start < sizes.len() {
        batches.push(start..sizes.len());
    }
    batches
}

/// Read owned file bytes, then dispatch by extension.
///
/// # Errors
/// - [`FormatError::Io`] if the file cannot be read.
/// - See [`crate::dispatch_by_extension`] for parse errors.
pub fn read_file(path: &Path) -> Result<Mesh, FormatError> {
    read_file_with_key_provider(path, &crate::hps::NoHpsKeyProvider)
}

/// Read a file with an HPS key provider.
///
/// # Errors
/// See [`read_file`].
pub fn read_file_with_key_provider(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    read_file_loaded_with_key_provider(path, key_provider).map(|loaded| loaded.mesh)
}

/// As [`read_file_with_key_provider`], additionally reporting the probed
/// kind and its import-unit interpretation.
///
/// # Errors
/// See [`read_file`].
pub fn read_file_loaded_with_key_provider(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
) -> Result<LoadedMesh, FormatError> {
    read_file_loaded_shaded(path, key_provider, crate::MeshShading::Reconstructed)
}

/// As [`read_file_with_key_provider`], choosing how vertex normals are
/// produced.
///
/// # Errors
/// See [`FormatError`].
pub fn read_file_shaded(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    read_file_loaded_shaded(path, key_provider, shading).map(|loaded| loaded.mesh)
}

/// Read a file with the requested shading, retaining its detected kind and units.
///
/// # Errors
/// See [`read_file_shaded`].
pub fn read_file_loaded_shaded(
    path: &Path,
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<LoadedMesh, FormatError> {
    let bytes = read_file_bytes(path)?;
    // The picture beside the file is found before the mesh is read, because a
    // PLY keeps its per-corner texture coordinates only when one exists.
    let mut atlas = None;
    let mut loaded = dispatch_by_extension_loaded_with_companion_budget(
        bytes.extension(),
        bytes.as_slice(),
        key_provider,
        shading,
        &mut |kind| {
            atlas = crate::companions::find(
                path,
                crate::companions::LocateKind::for_kind(kind),
                bytes.as_slice(),
            )?;
            Ok(atlas.is_some())
        },
    )?;
    crate::companions::attach(&mut loaded.mesh, atlas);
    Ok(loaded)
}

/// Read multiple files into a [`Scene`], wrapping each [`Mesh`] in a
/// [`SceneMesh`]. The canonical dental use case is loading an upper + lower
/// arch pair as a two-mesh scene.
///
/// Each mesh is placed at the origin with an identity transform; the caller
/// (app / thumbnail framer) repositions them as needed via `SceneMesh`'s
/// transform field, or just relies on `Scene::bbox()` to frame the union.
///
/// Every layer carries its import-unit interpretation
/// ([`SceneMesh::import_units`]); coordinates themselves are untouched —
/// normalization to millimeters happens exactly once, at the point a policy
/// applies a non-unity scale, and no v1 policy does yet.
///
/// **Fail-fast:** returns the first `(path, error)` pair encountered. The
/// caller decides whether to abort or offer "skip + continue" — for v1 we
/// abort, which keeps the error path simple and predictable.
///
/// # Errors
/// - The `Err` variant carries the path that failed and its [`FormatError`].
pub fn read_files(paths: &[PathBuf]) -> Result<Scene, (PathBuf, FormatError)> {
    read_files_with_key_provider(paths, &crate::hps::NoHpsKeyProvider)
}

/// Read multiple files into a [`Scene`], using an HPS key provider.
///
/// # Errors
/// See [`read_files`].
pub fn read_files_with_key_provider(
    paths: &[PathBuf],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Scene, (PathBuf, FormatError)> {
    read_files_with_memory_budget(paths, key_provider, 0)
}

/// Read files into a scene while reserving memory for layers already retained
/// by the caller.
///
/// Each parse batch is admitted from per-format peak estimates before readers
/// allocate geometry. The caller supplies the current scene estimate so Add
/// and Open account for the layers kept alive during decoding.
///
/// # Errors
/// The `Err` variant carries the path that failed and its [`FormatError`].
pub fn read_files_with_memory_budget(
    paths: &[PathBuf],
    key_provider: &dyn HpsKeyProvider,
    retained_scene_bytes: u64,
) -> Result<Scene, (PathBuf, FormatError)> {
    let mut scene = Scene::new();
    let Some(first_path) = paths.first() else {
        return Ok(scene);
    };
    check_scene_estimate(retained_scene_bytes).map_err(|error| (first_path.clone(), error))?;

    let batches = import_batches(
        &file_sizes(paths),
        IMPORT_BATCH_BUDGET_BYTES,
        IMPORT_PARALLELISM,
    );
    for batch in batches {
        let current_scene_bytes =
            retained_scene_bytes.saturating_add(scene.estimated_memory_bytes());
        if current_scene_bytes > SCENE_IMPORT_MEMORY_BUDGET_BYTES {
            return Err((
                paths[batch.start].clone(),
                FormatError::MemoryBudgetExceeded {
                    estimated_bytes: current_scene_bytes,
                    limit: SCENE_IMPORT_MEMORY_BUDGET_BYTES,
                },
            ));
        }
        let batch_paths = &paths[batch.clone()];
        let inputs = read_batch_inputs(batch_paths, current_scene_bytes)?;
        let admitted = admit_batch(inputs, current_scene_bytes, &batch_paths[0])?;
        let meshes = parse_batch(admitted, key_provider);
        for result in meshes {
            let loaded = result?;
            scene.add(SceneMesh::new(loaded.mesh).with_import_units(loaded.units));
        }
    }
    let final_estimate = retained_scene_bytes.saturating_add(scene.estimated_memory_bytes());
    check_scene_estimate(final_estimate).map_err(|error| (first_path.clone(), error))?;
    Ok(scene)
}

fn file_sizes(paths: &[PathBuf]) -> Vec<u64> {
    paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .map_or(0, |metadata| metadata.len())
                .min(MAX_IMPORT_BYTES)
        })
        .collect()
}

fn read_batch_inputs(
    paths: &[PathBuf],
    current_scene_bytes: u64,
) -> PathResult<Vec<(PathBuf, FileBytes)>> {
    let mut inputs = Vec::with_capacity(paths.len());
    let mut raw_bytes = 0_u64;
    for path in paths {
        let file_size = std::fs::metadata(path)
            .map_err(FormatError::Io)
            .map(|metadata| metadata.len())
            .map_err(|error| (path.clone(), error))?;
        if file_size > MAX_IMPORT_BYTES {
            return Err((
                path.clone(),
                FormatError::TooLarge {
                    bytes: file_size,
                    limit: MAX_IMPORT_BYTES,
                },
            ));
        }
        let raw_limit = SCENE_IMPORT_MEMORY_BUDGET_BYTES
            .saturating_sub(current_scene_bytes)
            .saturating_sub(raw_bytes)
            .min(MAX_IMPORT_BYTES);
        if file_size > raw_limit {
            return Err((
                path.clone(),
                memory_budget_error(
                    current_scene_bytes
                        .saturating_add(raw_bytes)
                        .saturating_add(file_size),
                ),
            ));
        }
        let file_bytes = read_file_bytes_with_limit(path, raw_limit).map_err(|error| {
            if matches!(error, FormatError::TooLarge { limit, .. } if limit < MAX_IMPORT_BYTES) {
                let estimated_bytes = current_scene_bytes
                    .saturating_add(raw_bytes)
                    .saturating_add(raw_limit.saturating_add(1));
                (path.clone(), memory_budget_error(estimated_bytes))
            } else {
                (path.clone(), error)
            }
        })?;
        raw_bytes = raw_bytes
            .saturating_add(u64::try_from(file_bytes.as_slice().len()).unwrap_or(u64::MAX));
        inputs.push((path.clone(), file_bytes));
    }
    Ok(inputs)
}

fn admit_batch(
    inputs: Vec<(PathBuf, FileBytes)>,
    current_scene_bytes: u64,
    fallback_path: &Path,
) -> PathResult<Vec<AdmittedInput>> {
    let raw_batch_bytes = inputs.iter().fold(0_u64, |total, (_, bytes)| {
        total.saturating_add(u64::try_from(bytes.as_slice().len()).unwrap_or(u64::MAX))
    });
    let first_path = inputs
        .first()
        .map_or_else(|| fallback_path.to_path_buf(), |(path, _)| path.clone());
    let mut admitted = Vec::with_capacity(inputs.len());
    let mut estimated_batch_bytes = 0_u64;
    for (path, bytes) in inputs {
        let kind = probe_kind(bytes.extension(), bytes.as_slice())
            .map_err(|error| (path.clone(), error))?;
        let input_bytes = u64::try_from(bytes.as_slice().len()).unwrap_or(u64::MAX);
        let reserved_bytes =
            current_scene_bytes.saturating_add(raw_batch_bytes.saturating_sub(input_bytes));
        let estimate = estimate_file_peak_bytes(kind, bytes.as_slice(), reserved_bytes)
            .map_err(|error| (path.clone(), error))?
            .saturating_add(crate::memory::estimate_companion_peak_bytes(
                kind,
                bytes.as_slice(),
            ));
        estimated_batch_bytes = estimated_batch_bytes.saturating_add(estimate);
        admitted.push(AdmittedInput { path, bytes, kind });
    }
    let total_estimate = current_scene_bytes.saturating_add(estimated_batch_bytes);
    check_scene_estimate(total_estimate).map_err(|error| (first_path, error))?;
    Ok(admitted)
}

fn parse_batch(
    admitted: Vec<AdmittedInput>,
    key_provider: &dyn HpsKeyProvider,
) -> Vec<PathResult<LoadedMesh>> {
    admitted
        .into_par_iter()
        .map(|input| {
            let atlas = crate::companions::find(
                &input.path,
                crate::companions::LocateKind::for_kind(input.kind),
                input.bytes.as_slice(),
            )
            .map_err(|error| (input.path.clone(), error))?;
            let mut loaded = dispatch_by_kind_loaded_admitted(
                input.kind,
                input.bytes.as_slice(),
                key_provider,
                crate::MeshShading::Reconstructed,
                atlas.is_some(),
            )
            .map_err(|error| (input.path.clone(), error))?;
            crate::companions::attach(&mut loaded.mesh, atlas);
            Ok(loaded)
        })
        .collect()
}

fn memory_budget_error(estimated_bytes: u64) -> FormatError {
    FormatError::MemoryBudgetExceeded {
        estimated_bytes,
        limit: SCENE_IMPORT_MEMORY_BUDGET_BYTES,
    }
}

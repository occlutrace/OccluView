//! Dispatch a file to its format reader by extension.
//!
//! This is the integration seam used by `occluview-app`, `occluview-cli`, and
//! `occluview-shell`. Each concrete reader is wired in here.

use crate::error::FormatError;
use crate::hps::HpsKeyProvider;
use crate::probe::FormatKind;
use crate::units::{policy_for, UnitInterpretation};
use occluview_core::{Mesh, Scene, SceneMesh};
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest single file the viewer will read into memory.
///
/// The number comes from the corpus, not from a round figure: the largest real
/// scan on the maintainer's machine is a 41 MB intraoral OBJ, and dental
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
/// The box this is developed on has 12 cores shared with the renderer and with
/// whatever else the operator is doing; parsing is memory-hungry rather than
/// CPU-hungry, so two is the point where a second file is worth it and a
/// twelfth is not.
pub const IMPORT_PARALLELISM: usize = 2;

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
    /// See [`dispatch_by_extension`].
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

/// Read `bytes` as the format indicated by `kind`, returning a [`Mesh`].
///
/// # Errors
/// - [`FormatError::Malformed`] for recognized formats whose reader is
///   intentionally deferred (currently 3MF).
pub fn dispatch_by_kind(kind: FormatKind, bytes: &[u8]) -> Result<Mesh, FormatError> {
    dispatch_by_kind_with_key_provider(kind, bytes, &crate::hps::NoHpsKeyProvider)
}

/// Read `bytes` as the format indicated by `kind`, using `key_provider` for
/// encrypted HPS `CE` sources.
///
/// # Errors
/// See [`dispatch_by_kind`].
pub fn dispatch_by_kind_with_key_provider(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    dispatch_by_kind_shaded(kind, bytes, key_provider, crate::MeshShading::Reconstructed)
}

/// As [`dispatch_by_kind_with_key_provider`], choosing how vertex normals are
/// produced.
///
/// Only the three formats a scanner writes take the policy; the rest are read
/// the one way they have always been read, because they are rare enough on the
/// thumbnail path that the plumbing would cost more than the milliseconds.
/// A parsed mesh together with how it was detected and what its
/// coordinates mean. Readers return bare [`Mesh`] geometry; the import-unit
/// interpretation rides alongside so callers never have to re-derive it
/// from the file extension (which magic probing may have overruled).
#[derive(Clone, Debug)]
pub struct LoadedMesh {
    /// Parsed geometry, in file-native coordinates.
    pub mesh: Mesh,
    /// Format selected by magic probing (falling back to the extension).
    pub kind: FormatKind,
    /// Import-unit policy for that kind (see [`policy_for`]).
    pub units: UnitInterpretation,
}

/// Read `bytes` with an explicit format kind, choosing how vertex normals
/// are produced.
///
/// # Errors
/// See [`FormatError`].
pub fn dispatch_by_kind_shaded(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    dispatch_by_kind_loaded(kind, bytes, key_provider, shading).map(|loaded| loaded.mesh)
}

/// As [`dispatch_by_kind_shaded`], additionally reporting the detected kind
/// and its import-unit interpretation.
///
/// # Errors
/// See [`dispatch_by_kind_shaded`].
pub fn dispatch_by_kind_loaded(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<LoadedMesh, FormatError> {
    let mesh = match kind {
        FormatKind::Stl => crate::stl::read_shaded(bytes, shading),
        FormatKind::Ply => crate::ply::read_shaded(bytes, shading),
        FormatKind::Obj => crate::obj::read_shaded(bytes, shading),
        // `.gltf` is JSON, and `probe` maps both extensions to this kind, so a
        // JSON file reaches a reader that only accepts the GLB container and
        // was told "not a glTF file: bad signature" for a file that *is* a
        // glTF. Defer only what actually looks like JSON, so a truncated or
        // corrupted `.glb` still fails as one.
        FormatKind::Gltf if looks_like_json(bytes) => Err(FormatError::Deferred {
            format: "glTF",
            reason: ".gltf (JSON) is not read; export .glb".to_string(),
        }),
        FormatKind::Gltf => crate::gltf::read(bytes),
        FormatKind::Off => crate::off::read(bytes),
        // Implement natively when demand appears.
        FormatKind::Threemf => Err(FormatError::Malformed {
            format: "occluview-formats",
            offset: 0,
            reason: format!("reader for {kind:?} not yet implemented"),
        }),
        FormatKind::Hps => crate::hps::read_with_key_provider(bytes, key_provider),
    }?;
    Ok(LoadedMesh {
        mesh,
        kind,
        units: policy_for(kind),
    })
}

/// Convenience: read `bytes` using the reader selected by file extension.
///
/// # Errors
/// See [`FormatError`] and [`dispatch_by_kind`].
/// Convenience: read `bytes` using the reader selected by file extension.
///
/// **Magic wins over extension.** Real-world dental files are frequently
/// mislabeled (a re-export renames `.stl` to `.ply`, or vice versa). We probe
/// the leading bytes first; only if the magic is silent do we trust the
/// extension. This is the same heuristic `stl`/`ply`/`solid`-byte check that
/// the STL reader uses internally, centralized here so every caller benefits.
///
/// # Errors
/// See [`FormatError`] and [`dispatch_by_kind`].
pub fn dispatch_by_extension(extension: &str, bytes: &[u8]) -> Result<Mesh, FormatError> {
    dispatch_by_extension_with_key_provider(extension, bytes, &crate::hps::NoHpsKeyProvider)
}

/// Convenience: read `bytes` by extension/magic with an HPS key provider.
///
/// # Errors
/// See [`dispatch_by_extension`].
pub fn dispatch_by_extension_with_key_provider(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
) -> Result<Mesh, FormatError> {
    dispatch_by_extension_shaded(
        extension,
        bytes,
        key_provider,
        crate::MeshShading::Reconstructed,
    )
}

/// As [`dispatch_by_extension_with_key_provider`], choosing how vertex normals
/// are produced.
///
/// # Errors
/// See [`FormatError`].
pub fn dispatch_by_extension_shaded(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<Mesh, FormatError> {
    dispatch_by_extension_loaded(extension, bytes, key_provider, shading).map(|loaded| loaded.mesh)
}

/// As [`dispatch_by_extension_shaded`], additionally reporting the probed
/// kind and its import-unit interpretation. The units follow the probed
/// kind — not the raw extension — so a mislabeled file that magic probing
/// re-routes still gets the policy of the format actually parsed.
///
/// # Errors
/// See [`dispatch_by_extension_shaded`].
pub fn dispatch_by_extension_loaded(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<LoadedMesh, FormatError> {
    // The BOM is stripped by `probe` (for signature matching) and by each text
    // reader (PLY, ASCII STL), NOT here. Stripping it in front of the whole
    // format layer removed three bytes from every container, including a binary
    // STL whose free-form 80-byte header happened to begin with those bytes:
    // the triangle count then came from the wrong offset and a valid file was
    // misread. Each layer that interprets text skips the mark itself.
    // Magic-first: if the bytes declare a format, honor it over the extension.
    // `probe` falls back to the extension when the magic is ambiguous (e.g.
    // binary STL with a zero header), so this is safe.
    let kind = match crate::probe::probe(Some(extension), bytes) {
        Ok(kind) => kind,
        // probe only fails when neither magic nor extension match; surface that.
        Err(e) => match e {
            FormatError::Unsupported { .. } => {
                // probe rejected the extension too — preserve the original
                // "unsupported extension" error.
                return Err(FormatError::Unsupported {
                    extension: extension.to_string(),
                });
            }
            other => return Err(other),
        },
    };
    dispatch_by_kind_loaded(kind, bytes, key_provider, shading)
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
/// grew between the two, which is exactly the window a hostile or merely busy
/// writer would use.
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
fn import_batches(sizes: &[u64], budget: u64, parallelism: usize) -> Vec<std::ops::Range<usize>> {
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
/// - See [`dispatch_by_extension`] for parse errors.
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
    let bytes = read_file_bytes(path)?;
    let mut loaded = dispatch_by_extension_loaded(
        bytes.extension(),
        bytes.as_slice(),
        key_provider,
        crate::MeshShading::Reconstructed,
    )?;
    // A PLY from another tool, and any OBJ, names its image beside the file;
    // the reader sees bytes only, so finding it is this layer's job.
    crate::companions::attach(
        &mut loaded.mesh,
        path,
        crate::companions::LocateKind::for_kind(loaded.kind),
        bytes.as_slice(),
    );
    Ok(loaded)
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
    let bytes = read_file_bytes(path)?;
    // The probed format decides the companion lookup, exactly as it decides
    // which reader runs.
    let kind =
        crate::probe::probe(Some(bytes.extension()), bytes.as_slice()).unwrap_or(FormatKind::Ply);
    let mut mesh =
        dispatch_by_extension_shaded(bytes.extension(), bytes.as_slice(), key_provider, shading)?;
    crate::companions::attach(
        &mut mesh,
        path,
        crate::companions::LocateKind::for_kind(kind),
        bytes.as_slice(),
    );
    Ok(mesh)
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
    let mut scene = Scene::new();
    if let [path] = paths {
        let loaded = read_file_loaded_with_key_provider(path, key_provider)
            .map_err(|e| (path.clone(), e))?;
        scene.add(SceneMesh::new(loaded.mesh).with_import_units(loaded.units));
        return Ok(scene);
    }

    // Sizes first, so the batch plan knows what it is about to hold. A file
    // whose metadata cannot be read gets size zero and its own real error from
    // the read below.
    let sizes = paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .map_or(0, |metadata| metadata.len())
                .min(MAX_IMPORT_BYTES)
        })
        .collect::<Vec<_>>();

    for batch in import_batches(&sizes, IMPORT_BATCH_BUDGET_BYTES, IMPORT_PARALLELISM) {
        let meshes = paths[batch]
            .par_iter()
            .map(|path| {
                read_file_loaded_with_key_provider(path, key_provider)
                    .map_err(|e| (path.clone(), e))
            })
            .collect::<Vec<_>>();

        for result in meshes {
            let loaded = result?;
            scene.add(SceneMesh::new(loaded.mesh).with_import_units(loaded.units));
        }
    }
    Ok(scene)
}

/// The bytes of `bytes` without a leading UTF-8 byte-order mark.
///
/// A BOM is metadata, not content: no format here declares it as part of its
/// signature, and a tool that writes one means the file that follows.
/// True when `bytes` starts an object, which is how a `.gltf` (JSON) file
/// begins and how a GLB never does.
///
/// A leading byte-order mark is skipped first because `probe` already strips it
/// before routing, so the two must agree about the same file.
pub(crate) fn looks_like_json(bytes: &[u8]) -> bool {
    matches!(
        strip_utf8_bom(bytes)
            .iter()
            .find(|b| !b.is_ascii_whitespace()),
        Some(b'{')
    )
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => rest,
        None => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal valid binary STL: 1 triangle in the XY plane, normal +Z.
    fn one_triangle_binary_stl() -> Vec<u8> {
        one_triangle_binary_stl_with_x_offset(0.0)
    }

    fn one_triangle_binary_stl_with_x_offset(x_offset: f32) -> Vec<u8> {
        let mut out = vec![0u8; 84];
        out[80..84].copy_from_slice(&1u32.to_le_bytes());
        let tri: [f32; 12] = [
            0.0,
            0.0,
            1.0, // normal
            x_offset,
            0.0,
            0.0, // v0
            x_offset + 1.0,
            0.0,
            0.0, // v1
            x_offset,
            1.0,
            0.0, // v2
        ];
        for f in tri {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]); // attribute byte count
        out
    }

    /// A sparse file of the requested length: instant, and it still reports
    /// the size that a real oversized file would.
    fn sparse_file(directory: &Path, name: &str, len: u64) -> PathBuf {
        let path = directory.join(name);
        let file = std::fs::File::create(&path).expect("create");
        file.set_len(len).expect("set_len");
        path
    }

    #[test]
    fn a_file_above_the_limit_is_refused_before_it_is_read() {
        let directory = tempdir();
        let path = sparse_file(&directory, "huge.stl", 4096);

        // `FileBytes` has no `Debug` on purpose: a byte buffer that can hold a
        // whole scan must not be printable by accident.
        let Err(error) = read_file_bytes_with_limit(&path, 2048) else {
            panic!("a file above the limit must be refused");
        };

        match error {
            FormatError::TooLarge { bytes, limit } => {
                assert_eq!(bytes, 4096);
                assert_eq!(limit, 2048);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_exactly_at_the_limit_is_read() {
        let directory = tempdir();
        let path = sparse_file(&directory, "exact.stl", 4096);

        let bytes = read_file_bytes_with_limit(&path, 4096).expect("read");

        assert_eq!(bytes.as_slice().len(), 4096);
        assert_eq!(bytes.extension(), "stl");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn files_are_batched_by_byte_budget_and_parallelism() {
        // Two at a time, and never more bytes than the budget in flight.
        assert_eq!(import_batches(&[1, 1, 1], 1024, 2), vec![0..2, 2..3]);
        assert_eq!(import_batches(&[600, 600, 1], 1024, 4), vec![0..1, 1..3]);
        // A file larger than the budget parses alone rather than being dropped:
        // the per-file limit is what decides whether it may be read at all.
        assert_eq!(import_batches(&[4096, 1], 1024, 4), vec![0..1, 1..2]);
        // Exactly filling the budget keeps the batch together.
        assert_eq!(import_batches(&[512, 512], 1024, 4), vec![0..2]);
        assert_eq!(import_batches(&[512, 513], 1024, 4), vec![0..1, 1..2]);
        assert!(import_batches(&[], 1024, 2).is_empty());
        assert_eq!(import_batches(&[1], 1024, 2), vec![0..1]);
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "occluview-dispatch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).expect("temp dir");
        base
    }

    fn zip_with_file(path: &str, bytes: &[u8]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            archive.start_file(path, options).expect("start zip file");
            archive.write_all(bytes).expect("write zip file");
            archive.finish().expect("finish zip file");
        }
        cursor.into_inner()
    }

    #[test]
    fn stl_dispatches_and_reads() {
        let bytes = one_triangle_binary_stl();
        let mesh = dispatch_by_extension("stl", &bytes).expect("STL should read");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn loaded_dispatch_reports_probed_kind_and_units() {
        use occluview_core::units::{SourceUnit, UnitConfidence};

        let bytes = one_triangle_binary_stl();
        let loaded = dispatch_by_extension_loaded(
            "stl",
            &bytes,
            &crate::hps::NoHpsKeyProvider,
            crate::MeshShading::Reconstructed,
        )
        .expect("STL should read");
        assert_eq!(loaded.mesh.triangle_count(), 1);
        assert_eq!(loaded.kind, FormatKind::Stl);
        assert_eq!(loaded.units.declared, SourceUnit::Unitless);
        assert_eq!(loaded.units.scale_to_mm, 1.0);
        assert_eq!(loaded.units.confidence, UnitConfidence::AssumedMillimeters);
    }

    #[test]
    fn unimplemented_reader_returns_malformed() {
        // 3MF is the remaining stub (PK zip magic; content that won't satisfy
        // any implemented reader's parser either, so it routes by extension to
        // the stub arm).
        let res = dispatch_by_extension("3mf", &[0xA5u8; 16]);
        let Err(FormatError::Malformed { reason, .. }) = res else {
            panic!("expected Malformed stub error, got {res:?}");
        };
        assert!(reason.contains("not yet implemented"));
    }

    #[test]
    fn raw_cc_hps_sources_are_parsed() {
        let hps = br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
</HPS>"#;
        let mesh = dispatch_by_extension("hps", hps).expect("raw CC HPS should parse");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.indices(), &[0, 1, 2]);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices()[2].position, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn ce_hps_sources_remain_deferred_until_key_provider_exists() {
        let hps = br"<HPS><Schema>CE</Schema></HPS>";
        let res = dispatch_by_extension("hps", hps);
        assert!(matches!(
            res,
            Err(FormatError::Deferred { format, .. }) if format == "HPS"
        ));

        let zip_hps = zip_with_file("scan/geometry.hps", hps);
        let res = dispatch_by_extension(crate::LEGACY_HPS_EXTENSION, &zip_hps);
        assert!(matches!(
            res,
            Err(FormatError::Deferred { format, .. }) if format == "HPS"
        ));

        let invalid_zip_hps = [0x50, 0x4B, 0x03, 0x04, 0x00, 0x00];
        let res = dispatch_by_extension(crate::LEGACY_HPS_EXTENSION, &invalid_zip_hps);
        assert!(matches!(res, Err(FormatError::Malformed { .. })));
    }

    #[test]
    fn obj_dispatches_and_reads() {
        // A minimal OBJ with one triangle and vertex colors (dental CAD extension).
        let obj = b"v 0 0 0 255 128 0\nv 1 0 0 0 255 0\nv 0 1 0 0 0 255\nf 1 2 3\n";
        let mesh = dispatch_by_extension("obj", obj).expect("OBJ should read");
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.has_vertex_colors());
    }

    #[test]
    fn ply_dispatches_and_reads() {
        // A minimal ASCII PLY with one colored vertex.
        let ply = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 0\nproperty list uchar int vertex_indices\nend_header\n1.0 2.0 3.0 255 128 0\n";
        let mesh = dispatch_by_extension("ply", ply).expect("PLY should read");
        assert_eq!(mesh.vertices().len(), 1);
        assert!(mesh.has_vertex_colors());
    }

    #[test]
    fn mislabeled_extension_falls_back_to_magic() {
        // Real-world case from the OccluTrace corpus: a binary STL renamed to
        // `.ply`. The 80-byte header carries an arbitrary ASCII label
        // ("OccluTrace Native binary STL"); the file is binary STL underneath.
        // Magic-first dispatch must route it to the STL reader, not the PLY
        // reader (which would reject it as bad signature).
        let mut bytes = vec![0u8; 84];
        let label = b"OccluTrace Native binary STL";
        bytes[..label.len()].copy_from_slice(label);
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        // One triangle: normal +Z, three vertices.
        let tri: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        for f in tri {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);

        let mesh = dispatch_by_extension("ply", &bytes).expect("magic wins over extension");
        assert_eq!(mesh.triangle_count(), 1, "STL content must parse as STL");
    }

    #[test]
    fn unknown_extension_is_unsupported() {
        let res = dispatch_by_extension("xyz", &[0u8; 4]);
        assert!(matches!(res, Err(FormatError::Unsupported { .. })));
    }

    #[test]
    fn read_file_parses_owned_bytes() {
        // Write a minimal binary STL and parse the owned snapshot.
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.stl");
        std::fs::write(&path, &bytes).expect("write");
        let mesh = read_file(&path).expect("read_file should parse");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn file_bytes_remain_readable_after_source_is_truncated() {
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.stl");
        std::fs::write(&path, &bytes).expect("write source");

        let snapshot = read_file_bytes(&path).expect("read source");
        std::fs::write(&path, []).expect("truncate source");

        assert_eq!(snapshot.as_slice(), bytes.as_slice());
        assert_eq!(
            snapshot
                .dispatch()
                .expect("parse snapshot")
                .triangle_count(),
            1
        );
    }

    #[test]
    fn read_file_bytes_returns_extension_and_contents() {
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.STL");
        std::fs::write(&path, &bytes).expect("write");

        let file_bytes = read_file_bytes(&path).expect("read file bytes");

        assert_eq!(file_bytes.extension(), "stl");
        assert_eq!(file_bytes.as_slice(), bytes.as_slice());
    }

    #[test]
    fn read_file_bytes_dispatches_with_its_extension() {
        let bytes = one_triangle_binary_stl();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tri.stl");
        std::fs::write(&path, &bytes).expect("write");

        let file_bytes = read_file_bytes(&path).expect("read file bytes");
        let mesh = file_bytes.dispatch().expect("dispatch file bytes");

        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn read_file_missing_extension_is_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("noext");
        std::fs::write(&path, b"x").expect("write");
        assert!(matches!(
            read_file(&path),
            Err(FormatError::Unsupported { .. })
        ));
    }

    #[test]
    fn read_file_bytes_missing_extension_is_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("noext");
        std::fs::write(&path, b"x").expect("write");
        assert!(matches!(
            read_file_bytes(&path),
            Err(FormatError::Unsupported { .. })
        ));
    }

    #[test]
    fn read_files_preserves_input_layer_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first.stl");
        let second = dir.path().join("second.stl");
        std::fs::write(&first, one_triangle_binary_stl_with_x_offset(0.0)).expect("write first");
        std::fs::write(&second, one_triangle_binary_stl_with_x_offset(10.0)).expect("write second");

        let scene = read_files(&[first, second]).expect("read files");

        assert_eq!(scene.meshes().len(), 2);
        assert_eq!(scene.meshes()[0].mesh.bbox_uncached().min.x, 0.0);
        assert_eq!(scene.meshes()[1].mesh.bbox_uncached().min.x, 10.0);
    }

    #[test]
    fn read_files_single_path_returns_one_mesh_scene() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("single.stl");
        std::fs::write(&path, one_triangle_binary_stl()).expect("write");

        let scene = read_files(&[path]).expect("read files");

        assert_eq!(scene.meshes().len(), 1);
        assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
    }
}

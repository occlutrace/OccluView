//! Dispatch a file to its format reader by extension.
//!
//! This is the integration seam used by `occluview-app`, `occluview-cli`, and
//! `occluview-shell`. Each concrete reader is wired in here.

use crate::error::FormatError;
use crate::hps::HpsKeyProvider;
use crate::memory::{check_estimate, estimate_file_peak_bytes};
use crate::probe::FormatKind;
#[cfg(test)]
pub(crate) use crate::read::import_batches;
pub use crate::read::{
    read_file, read_file_bytes, read_file_bytes_with_limit, read_file_loaded_shaded,
    read_file_loaded_with_key_provider, read_file_shaded, read_file_with_key_provider, read_files,
    read_files_with_key_provider, read_files_with_memory_budget, FileBytes,
    IMPORT_BATCH_BUDGET_BYTES, IMPORT_PARALLELISM, MAX_IMPORT_BYTES,
};
use crate::units::{policy_for, UnitInterpretation};

use occluview_core::Mesh;

/// Read `bytes` as the format indicated by `kind`, returning a [`Mesh`].
///
/// # Errors
/// - [`FormatError::Deferred`] for recognized formats this build does not read
///   (3MF, and the JSON `.gltf` form of glTF).
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
/// Only the three formats a scanner writes take the policy; the rest are read
/// one way, because they are rare enough on the thumbnail path that the
/// plumbing would cost more than the milliseconds.
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
    check_estimate(estimate_file_peak_bytes(kind, bytes, 0)?)?;
    dispatch_by_kind_loaded_admitted(kind, bytes, key_provider, shading)
}

pub(crate) fn dispatch_by_kind_loaded_admitted(
    kind: FormatKind,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<LoadedMesh, FormatError> {
    let mesh = match kind {
        FormatKind::Stl => crate::stl::read_admitted(bytes, shading),
        FormatKind::Ply => crate::ply::read_admitted(bytes, shading),
        FormatKind::Obj => crate::obj::read_admitted(bytes, shading),
        // `.gltf` is JSON, and `probe` maps both extensions to this kind, but
        // the GLB reader only accepts the binary container and would answer
        // "not a glTF file: bad signature" for a file that is a glTF. Defer
        // only what looks like JSON, so a truncated or corrupted `.glb` still
        // fails as one.
        FormatKind::Gltf if looks_like_json(bytes) => Err(FormatError::Deferred {
            format: "glTF",
            reason: ".gltf (JSON) is not read; export .glb".to_string(),
        }),
        FormatKind::Gltf => crate::gltf::read_admitted(bytes),
        FormatKind::Off => crate::off::read_admitted(bytes),
        // 3MF is recognized from the ZIP container, but this build has no
        // reader. The operator reads this sentence in the load-failure dialog,
        // so it names the format and an action instead of the crate and the
        // Rust variant that produced it.
        FormatKind::Threemf => Err(FormatError::Deferred {
            format: "3MF",
            reason: "this build has no 3MF reader; export the model as STL, PLY, OBJ, or GLB"
                .to_string(),
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
    dispatch_by_extension_loaded_inner(extension, bytes, key_provider, shading, false)
}

pub(crate) fn dispatch_by_extension_loaded_with_companion_budget(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
) -> Result<LoadedMesh, FormatError> {
    dispatch_by_extension_loaded_inner(extension, bytes, key_provider, shading, true)
}

fn dispatch_by_extension_loaded_inner(
    extension: &str,
    bytes: &[u8],
    key_provider: &dyn HpsKeyProvider,
    shading: crate::MeshShading,
    includes_companions: bool,
) -> Result<LoadedMesh, FormatError> {
    // The BOM is stripped by `probe` (for signature matching) and by each text
    // reader (PLY, ASCII STL), not here. Stripping it in front of the whole
    // format layer would remove three bytes from every container, including a
    // binary STL whose free-form 80-byte header begins with those bytes: the
    // triangle count would then come from the wrong offset and a valid file
    // would be misread. Each layer that interprets text skips the mark itself.
    // Magic-first: if the bytes declare a format, honor it over the extension.
    // `probe` falls back to the extension when the magic is ambiguous (e.g.
    // binary STL with a zero header), so this is safe.
    let kind = probe_kind(extension, bytes)?;
    let file_estimate = estimate_file_peak_bytes(kind, bytes, 0)?;
    let companion_estimate = if includes_companions {
        crate::memory::estimate_companion_peak_bytes(kind, bytes)
    } else {
        0
    };
    check_estimate(file_estimate.saturating_add(companion_estimate))?;
    dispatch_by_kind_loaded_admitted(kind, bytes, key_provider, shading)
}

pub(crate) fn probe_kind(extension: &str, bytes: &[u8]) -> Result<FormatKind, FormatError> {
    match crate::probe::probe(Some(extension), bytes) {
        Ok(kind) => Ok(kind),
        Err(FormatError::Unsupported { .. }) => Err(FormatError::Unsupported {
            extension: extension.to_string(),
        }),
        Err(error) => Err(error),
    }
}

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

/// The bytes of `bytes` without a leading UTF-8 byte-order mark.
///
/// A BOM is metadata, not content: no format here declares it as part of its
/// signature, and a tool that writes one means the file that follows.
fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => rest,
        None => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SCENE_IMPORT_MEMORY_BUDGET_BYTES;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A minimal valid binary STL: 1 triangle in the XY plane, normal +Z.
    fn one_triangle_binary_stl() -> Vec<u8> {
        one_triangle_binary_stl_with_x_offset(0.0)
    }

    fn one_triangle_binary_stl_with_x_offset(x_offset: f32) -> Vec<u8> {
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
        occluview_core::test_support::binary_stl(&[tri])
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
    fn a_zip_without_a_reader_is_deferred_with_a_user_facing_message() {
        // Every ZIP reaches the 3MF arm: `probe` classifies the container by
        // its magic, whatever the file is named. The message is shown in the
        // load-failure dialog, so it must name neither an internal crate nor a
        // Rust variant.
        let zip = zip_with_file("[Content_Types].xml", b"<Types/>");
        let error = dispatch_by_extension("zip", &zip)
            .expect_err("a recognized container with no reader must not read");
        assert!(matches!(error, FormatError::Deferred { .. }));
        let message = error.to_string();
        assert!(
            message.contains("3MF"),
            "the operator message must name the format: {message}"
        );
        assert!(
            !message.contains("occluview"),
            "the operator message leaks an internal crate name: {message}"
        );
        assert!(
            !message.contains("Threemf"),
            "the operator message leaks a Rust variant name: {message}"
        );
    }

    #[test]
    fn a_gltf_json_file_is_deferred_with_a_glb_export_hint() {
        // `.gltf` (JSON) is not among the v1 open extensions because the
        // reader accepts only the GLB container. The refusal still has to tell
        // the operator what to do next.
        let error = dispatch_by_extension("gltf", br#"{"asset":{"version":"2.0"}}"#)
            .expect_err(".gltf (JSON) has no reader");
        assert!(matches!(error, FormatError::Deferred { .. }));
        let message = error.to_string();
        assert!(
            message.contains(".glb"),
            "the operator message must offer the .glb export: {message}"
        );
        assert!(
            !message.contains("occluview"),
            "the operator message leaks an internal crate name: {message}"
        );
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
        let mut header = [0u8; 80];
        let label = b"OccluTrace Native binary STL";
        header[..label.len()].copy_from_slice(label);
        // One triangle: normal +Z, three vertices.
        let tri: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let bytes = occluview_core::test_support::binary_stl_with_header(&header, &[tri]);

        let mesh = dispatch_by_extension("ply", &bytes).expect("magic wins over extension");
        assert_eq!(mesh.triangle_count(), 1, "STL content must parse as STL");
    }

    #[test]
    fn unknown_extension_is_unsupported() {
        let res = dispatch_by_extension("xyz", &[0u8; 4]);
        assert!(matches!(res, Err(FormatError::Unsupported { .. })));
    }

    #[test]
    fn refuses_a_ply_header_that_claims_more_vertices_than_the_scene_budget() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 1000000000\nproperty float x\nproperty float y\nproperty float z\nelement face 0\nproperty list uchar int vertex_indices\nend_header\n";

        let error = dispatch_by_extension("ply", bytes)
            .expect_err("the count must be checked before a vertex buffer is built");

        assert!(matches!(error, FormatError::MemoryBudgetExceeded { .. }));
    }

    #[test]
    fn scene_admission_includes_memory_retained_by_existing_layers() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("tri.stl");
        std::fs::write(&path, one_triangle_binary_stl()).expect("write STL");

        let error = read_files_with_memory_budget(
            std::slice::from_ref(&path),
            &crate::hps::NoHpsKeyProvider,
            SCENE_IMPORT_MEMORY_BUDGET_BYTES - 1,
        )
        .expect_err("the existing scene and source buffer exceed the budget");

        assert!(matches!(error.1, FormatError::MemoryBudgetExceeded { .. }));
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

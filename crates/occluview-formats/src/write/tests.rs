/// A payload whose declared format this build does not know is left alone
/// rather than decoded on a guess.
#[test]
fn a_payload_with_an_unknown_format_is_not_decoded() {
    let header = "ply\nformat ascii 1.0\n\
             comment OccluViewTextureFormat webp\n\
             comment OccluViewTextureBase64 aGVsbG8=\n\
             element vertex 3\n\
             property float x\nproperty float y\nproperty float z\n\
             property float s\nproperty float t\n\
             end_header\n0 0 0 0 1\n1 0 0 1 1\n0 1 0 0 0\n";
    let mesh = crate::ply::read(header.as_bytes()).expect("the mesh still reads");
    assert!(
        mesh.texture().is_none(),
        "an unknown payload is not decoded"
    );
}
use crate::write::{write_mesh, MeshWriteWarning};

/// A textured export follows the scanner convention: colour on the
/// vertices, one file, and no encoded image in the header.
#[test]
fn an_exported_ply_carries_its_colour_on_the_vertices() {
    use occluview_core::{MeshTexture, Vertex};

    let mut mesh = Mesh::new(
        Some("arch".to_string()),
        vec![
            Vertex::at(glam::Vec3::ZERO).with_uv([0.25, 0.25]),
            Vertex::at(glam::Vec3::X).with_uv([0.75, 0.25]),
            Vertex::at(glam::Vec3::Y).with_uv([0.25, 0.75]),
        ],
        vec![0, 1, 2],
    )
    .expect("a triangle mesh");
    mesh.set_texture(MeshTexture::new(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ],
    ));

    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("upper-edited.ply");
    let report = write_mesh_to_new_file(
        &path,
        &mesh,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )
    .expect("write the export");

    assert!(
        !report
            .warnings
            .contains(&MeshWriteWarning::TextureImageNotWritten),
        "the colour was written: {:?}",
        report.warnings
    );
    let entries: Vec<String> = std::fs::read_dir(directory.path())
        .expect("read the folder")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["upper-edited.ply".to_string()],
        "an export must leave exactly one file behind, found {entries:?}"
    );

    let bytes = std::fs::read(&path).expect("the exported ply");
    let header_end = bytes
        .windows(b"end_header\n".len())
        .position(|window| window == b"end_header\n")
        .expect("end header");
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(
        !header.contains("TextureFile"),
        "no image sits beside this file, so nothing may name one:\n{header}"
    );
    assert!(
        !header.contains("OccluViewTexture"),
        "no image may be encoded into the header:\n{header}"
    );
    assert!(
        header.contains(
            "property uchar red\nproperty uchar green\nproperty uchar blue\nproperty uchar alpha\n"
        ),
        "the colour is per vertex:\n{header}"
    );

    let read = crate::ply::read(&bytes).expect("read the export back");
    assert!(read.has_vertex_colors(), "the colour came back");
    assert!(read.texture().is_none(), "no phantom image is attached");
    assert_eq!(read.vertices()[0].color, [255, 0, 0, 255]);
    assert_eq!(read.vertices()[1].color, [0, 255, 0, 255]);
    assert_eq!(read.vertices()[2].color, [0, 0, 255, 255]);
}

/// A PLY an earlier release wrote with an encoded image in its header still
/// opens with that image: dropping the writer must not strand those files.
#[test]
fn a_legacy_embedded_ply_header_still_decodes_to_its_image() {
    // A 1x1 opaque red PNG, the form the legacy writer embedded.
    const RED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    let header = format!(
        "ply\nformat ascii 1.0\n\
             comment OccluViewTextureFormat png\n\
             comment OccluViewTextureWidth 1\n\
             comment OccluViewTextureHeight 1\n\
             comment OccluViewTextureBase64 {RED_PNG}\n\
             element vertex 3\n\
             property float x\nproperty float y\nproperty float z\n\
             property float s\nproperty float t\n\
             end_header\n0 0 0 0 1\n1 0 0 1 1\n0 1 0 0 0\n"
    );
    let mesh = crate::ply::read(header.as_bytes()).expect("the legacy file still reads");
    let texture = mesh.texture().expect("the legacy image decodes");
    assert_eq!((texture.width, texture.height), (1, 1));
    assert_eq!(texture.rgba, vec![255, 0, 0, 255]);
}

use super::*;
use occluview_core::test_support::colored_uv_triangle_mesh;
use occluview_core::{Mesh, Vertex};
use tempfile::NamedTempFile;

#[cfg(unix)]
#[test]
fn overwrite_resolves_eight_links_and_rejects_a_ninth() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().expect("directory");
    let target = directory.path().join("target.obj");
    std::fs::write(&target, b"previous export").expect("target");
    let mut last = target.clone();
    for depth in 1..=9 {
        let link = directory.path().join(format!("link-{depth}.obj"));
        symlink(&last, &link).expect("link");
        if depth <= 8 {
            assert_eq!(
                resolve_overwrite_destination(&link).expect("supported chain"),
                target
            );
        } else {
            assert!(resolve_overwrite_destination(&link).is_err());
        }
        last = link;
    }
}

#[test]
fn overwrite_semantics_truncate_existing_file() {
    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("scan.obj");
    std::fs::write(&destination, b"stale bytes").expect("seed file");

    let report = write_mesh_overwrite(
        &destination,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    )
    .expect("overwrite");

    assert_eq!(report.format, MeshWriteFormat::Obj);
    let bytes = std::fs::read(&destination).expect("read back");
    assert!(!bytes.starts_with(b"stale bytes"));
}

#[test]
fn overwrite_commits_a_complete_file_without_leaving_a_sibling_temp() {
    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("scan.obj");
    std::fs::write(&destination, b"previous export").expect("seed file");

    write_mesh_overwrite(
        &destination,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    )
    .expect("overwrite");

    let bytes = std::fs::read(&destination).expect("read complete export");
    assert!(bytes.starts_with(b"o sample\n"));
    assert!(!bytes.starts_with(b"previous export"));
    assert!(std::fs::read_dir(directory.path())
        .expect("read directory")
        .filter_map(Result::ok)
        .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
}

#[test]
fn overwrite_temp_files_are_unique_siblings_of_the_destination() {
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("scan.ply");
    let (first_path, first_file) = create_export_temp(&destination).expect("first temp");
    let (second_path, second_file) = create_export_temp(&destination).expect("second temp");
    drop(first_file);
    drop(second_file);

    assert_ne!(first_path, second_path);
    assert_eq!(first_path.parent(), destination.parent());
    assert_eq!(second_path.parent(), destination.parent());
    std::fs::remove_file(first_path).expect("remove first temp");
    std::fs::remove_file(second_path).expect("remove second temp");
}

#[test]
fn new_file_publishes_only_the_complete_export() {
    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("scan.obj");

    let report = write_mesh_to_new_file(
        &destination,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    )
    .expect("new export");

    assert_eq!(report.format, MeshWriteFormat::Obj);
    assert!(std::fs::read(&destination)
        .expect("read complete export")
        .starts_with(b"o sample\n"));
    assert!(std::fs::read_dir(directory.path())
        .expect("read directory")
        .filter_map(Result::ok)
        .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
}

#[test]
fn new_file_collision_leaves_the_existing_export_and_no_temp_behind() {
    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("scan.obj");
    let seed = b"operator export already exists";
    std::fs::write(&destination, seed).expect("seed destination");

    let result = write_mesh_to_new_file(
        &destination,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    );

    let error = result.expect_err("create-new export must reject collisions");
    // The batch exporter retries the next numbered name when it sees this
    // kind. On Windows the collision is only detectable at publish time,
    // inside `MoveFileExW`, so the classification has to survive the Win32
    // error conversion there.
    assert!(
        matches!(
            &error,
            FormatError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists
        ),
        "a create-new collision must be classified for retry, got {error:?}"
    );
    assert_eq!(std::fs::read(&destination).expect("read seed"), seed);
    assert!(std::fs::read_dir(directory.path())
        .expect("read directory")
        .filter_map(Result::ok)
        .all(|entry| { !entry.file_name().to_string_lossy().contains(".occluview-") }));
}

/// Case folders are often symlinks into a lab archive. The publish step is
/// a rename, and rename replaces the link itself, so an operator overwriting
/// `CASE/upper.ply` would get a success message while the archive copy kept
/// the previous geometry.
#[cfg(unix)]
#[test]
fn overwriting_a_symlink_updates_its_target_and_keeps_the_link() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temp directory");
    let target = directory.path().join("archive.obj");
    let link = directory.path().join("case.obj");
    std::fs::write(&target, b"previous scan").expect("seed target");
    symlink(&target, &link).expect("create symlink");

    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    write_mesh_overwrite(
        &link,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    )
    .expect("overwrite through the link");

    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link metadata")
            .file_type()
            .is_symlink(),
        "the operator's link must survive the export"
    );
    assert!(
        std::fs::read(&target)
            .expect("read target")
            .starts_with(b"o sample\n"),
        "the file the link points at must receive the new export"
    );
    assert!(
        std::fs::read_dir(directory.path())
            .expect("read directory")
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".occluview-")),
        "a published overwrite leaves no temporary behind"
    );
}

/// Some filesystems cannot hard-link at all. The fallback has to keep the
/// contract that matters there: an existing destination is never replaced,
/// and a collision is still reported as such so the batch exporter can
/// advance to the next numbered name. The fallback never runs on Windows,
/// which has its own no-replace publish path.
#[cfg(not(windows))]
#[test]
fn a_publish_without_link_support_still_never_replaces_a_destination() {
    let directory = tempfile::tempdir().expect("temp directory");
    let temporary = directory.path().join("staged.tmp");
    let destination = directory.path().join("scan.obj");
    std::fs::write(&temporary, b"complete export").expect("stage temporary");

    publish_by_exclusive_copy(&temporary, &destination).expect("first publish");
    assert_eq!(
        std::fs::read(&destination).expect("read export"),
        b"complete export"
    );
    assert!(
        !temporary.exists(),
        "a published export consumes its staged temporary"
    );

    std::fs::write(&temporary, b"second export").expect("stage second temporary");
    let error = publish_by_exclusive_copy(&temporary, &destination)
        .expect_err("an existing destination is never replaced");
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists,
        "a collision must stay classifiable for the batch retry"
    );
    assert_eq!(
        std::fs::read(&destination).expect("read export"),
        b"complete export",
        "the first export survives the second publish"
    );
}

/// The fallback must not swallow a real collision: the batch exporter has
/// to see `AlreadyExists` to advance to the next numbered name, while every
/// other link failure means the filesystem cannot link at all.
#[cfg(not(windows))]
#[test]
fn a_link_collision_stays_classified_and_other_failures_fall_back() {
    assert!(!linkless_publish_required(&std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "a file with that name exists",
    )));
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::Unsupported,
        std::io::ErrorKind::Other,
    ] {
        assert!(
            linkless_publish_required(&std::io::Error::new(kind, "link unavailable")),
            "{kind:?} means the filesystem could not link, so the fallback applies"
        );
    }
}

/// A chain that never reaches a regular file must fail the export instead
/// of renaming onto a link: the rename would replace the link inode and
/// leave the file it pointed at with the previous geometry, which the
/// symlink resolution exists to prevent. The link is left unchanged.
#[cfg(unix)]
#[test]
fn an_unresolvable_link_chain_fails_instead_of_replacing_the_link() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temp directory");
    let first = directory.path().join("a.obj");
    let second = directory.path().join("b.obj");
    symlink(&second, &first).expect("first link");
    symlink(&first, &second).expect("closing the loop");

    let error =
        resolve_overwrite_destination(&first).expect_err("a link loop must not resolve to a file");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let mesh = colored_uv_triangle_mesh(Some("sample")).expect("sample mesh");
    let outcome = write_mesh_overwrite(
        &first,
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    );
    assert!(
        outcome.is_err(),
        "exporting onto an unresolvable link must fail, not report success"
    );
    assert!(
        std::fs::symlink_metadata(&first)
            .expect("link metadata")
            .file_type()
            .is_symlink(),
        "a failed export must leave the link alone"
    );
}

#[test]
fn a_rejected_export_leaves_the_destination_untouched() {
    // A PLY point cloud is a loadable layer, so "export this layer as .stl
    // over an existing scan" is an ordinary action, and the rejection has
    // to land before `File::create` truncates.
    let file = NamedTempFile::new().expect("temp file");
    let seed = b"an existing scan the operator still needs";
    std::fs::write(file.path(), seed).expect("seed destination");

    let cloud = Mesh::point_cloud(
        Some("cloud".to_string()),
        vec![Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0))],
    );
    let result = write_mesh_overwrite(
        file.path(),
        &cloud,
        MeshWriteFormat::StlBinary,
        MeshWriteOptions::default(),
    );

    assert!(result.is_err(), "STL cannot represent a point cloud");
    let after = std::fs::read(file.path()).expect("read back");
    assert_eq!(
        after, seed,
        "a failed export must leave the destination exactly as it found it"
    );
}

#[test]
fn an_empty_mesh_is_rejected_before_touching_the_destination() {
    let file = NamedTempFile::new().expect("temp file");
    let seed = b"previous export";
    std::fs::write(file.path(), seed).expect("seed destination");

    let result = write_mesh_overwrite(
        file.path(),
        &Mesh::empty(),
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    );

    assert!(result.is_err(), "an empty placeholder is not an export");
    assert_eq!(std::fs::read(file.path()).expect("read destination"), seed);
}

#[test]
fn a_non_finite_position_is_rejected_before_touching_the_destination() {
    let file = NamedTempFile::new().expect("temp file");
    let seed = b"previous export";
    std::fs::write(file.path(), seed).expect("seed destination");
    let mesh = Mesh::new(
        Some("bad".to_owned()),
        vec![
            Vertex::at(glam::Vec3::new(f32::NAN, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("shape is valid even though its payload is not");

    let result = write_mesh_overwrite(
        file.path(),
        &mesh,
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    );

    assert!(result.is_err(), "non-finite positions are not exportable");
    assert_eq!(std::fs::read(file.path()).expect("read destination"), seed);
}

#[test]
fn a_point_cloud_still_writes_where_the_format_supports_it() {
    let directory = tempfile::tempdir().expect("temp directory");
    let destination = directory.path().join("cloud.ply");
    let cloud = Mesh::point_cloud(
        Some("cloud".to_string()),
        vec![Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0))],
    );

    let report = write_mesh_overwrite(
        &destination,
        &cloud,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )
    .expect("PLY carries point clouds");

    assert_eq!(report.format, MeshWriteFormat::PlyBinaryLittleEndian);
    assert!(!std::fs::read(&destination).expect("read back").is_empty());
}

#[test]
fn public_sink_entry_point_rejects_an_empty_mesh_before_writing() {
    let mut bytes = Vec::from(b"prefix".as_slice());

    let result = write_mesh(
        &mut bytes,
        &Mesh::empty(),
        MeshWriteFormat::Obj,
        MeshWriteOptions::default(),
    );

    assert!(result.is_err(), "an empty mesh is not an export");
    assert_eq!(bytes, b"prefix");
}

//! A textured scan keeps its texture when it is exported as PLY.
//!
//! PLY has no texture element, so the image travels inside the file as encoded
//! comment lines and nothing is written beside it: one scan in, one scan out.
//! This walks the whole chain — a textured GLB in, a PLY out, the same PLY read
//! back — and compares the pixels, because a texture that arrives re-encoded is
//! a texture that did not arrive.

#![allow(clippy::expect_used, clippy::panic)]

use occluview_core::{Mesh, MeshTexture, Vertex};
use occluview_formats::{
    dispatch_by_extension, write_mesh_to_new_file, write_textured_glb, MeshWriteFormat,
    MeshWriteOptions,
};

fn textured_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        Some("arch".to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)).with_uv([0.0, 1.0]),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)).with_uv([1.0, 1.0]),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)).with_uv([0.0, 0.0]),
        ],
        vec![0, 1, 2],
    )
    .expect("a triangle mesh");
    // Four pixels, one per corner, so a mirrored or reordered image cannot pass
    // by accident.
    mesh.set_texture(MeshTexture::new(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ],
    ));
    mesh
}

#[test]
fn a_textured_glb_exported_as_ply_carries_its_texture() -> Result<(), Box<dyn std::error::Error>> {
    let source = textured_mesh();
    let glb = write_textured_glb(&source)?;
    let imported = dispatch_by_extension("glb", &glb)?;
    assert!(
        imported.texture().is_some(),
        "the GLB round trip must keep the texture before the PLY is involved"
    );
    assert!(imported.has_uvs(), "and its coordinates");

    let directory = std::env::temp_dir().join(format!("occluview-ply-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    let export = directory.join("arch-edited.ply");

    let report = write_mesh_to_new_file(
        &export,
        &imported,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )?;
    assert!(
        !report
            .warnings
            .contains(&occluview_formats::MeshWriteWarning::TextureImageNotWritten),
        "the texture was written: {:?}",
        report.warnings
    );

    // One file, and nothing beside it: the image travels inside.
    let entries: Vec<String> = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["arch-edited.ply".to_string()],
        "an export must leave exactly one file behind, found {entries:?}"
    );
    let bytes = std::fs::read(&export)?;
    let header_end = bytes
        .windows(b"end_header\n".len())
        .position(|window| window == b"end_header\n")
        .expect("a header terminator");
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(
        !header.contains("TextureFile"),
        "nothing is written beside the export, so the header must name nothing:\n{header}"
    );
    assert!(
        header.contains("comment OccluViewTextureBase64 "),
        "the image must be in the header:\n{header}"
    );
    assert!(
        header.contains("property list uchar float texcoord"),
        "the faces must carry texture coordinates, or nothing can sample the image"
    );

    // And the same file, moved on its own, still opens with its texture.
    let reopened = dispatch_by_extension("ply", &bytes)?;
    let texture = reopened.texture().expect("the traveled texture");
    assert_eq!((texture.width, texture.height), (2, 2));
    assert_eq!(
        texture.rgba,
        source.texture().expect("the source texture").rgba,
        "the pixels must survive the whole chain"
    );
    assert!(reopened.has_uvs(), "the coordinates came back with it");
    let uv = reopened.vertices()[1].uv;
    assert!(
        (uv[0] - 1.0).abs() < f32::EPSILON && (uv[1] - 1.0).abs() < f32::EPSILON,
        "the corner that was (1,1) came back as {uv:?}"
    );

    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

#[test]
fn an_untextured_export_writes_one_file_and_reports_no_loss(
) -> Result<(), Box<dyn std::error::Error>> {
    let mesh = Mesh::new(
        Some("plain".to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )?;
    let directory =
        std::env::temp_dir().join(format!("occluview-ply-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    let export = directory.join("plain.ply");

    let report = write_mesh_to_new_file(
        &export,
        &mesh,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )?;

    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let entries: Vec<String> = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["plain.ply".to_string()],
        "a mesh without a texture must not leave anything behind"
    );

    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

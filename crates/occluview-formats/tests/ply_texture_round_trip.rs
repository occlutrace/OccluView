//! A textured scan keeps its colour when it is exported as PLY.
//!
//! PLY has no texture element, so the colour is baked into the vertices as
//! RGBA, and nothing is written beside the file: one scan in, one scan out.
//! This walks the whole chain — a textured GLB in, a PLY out, the same PLY read
//! back — and checks that the colour arrives on the vertices it was sampled for.

#![allow(clippy::expect_used, clippy::panic)]

use occluview_core::{Mesh, MeshTexture, Vertex};
use occluview_formats::{
    dispatch_by_extension, write_mesh_to_new_file, write_textured_glb, MeshWriteFormat,
    MeshWriteOptions,
};

/// A quad whose four UVs land on four different texels, so a mirrored or
/// reordered image cannot pass by accident.
fn textured_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        Some("arch".to_string()),
        vec![
            Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)).with_uv([0.25, 0.25]),
            Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)).with_uv([0.75, 0.25]),
            Vertex::at(glam::Vec3::new(1.0, 1.0, 0.0)).with_uv([0.75, 0.75]),
            Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)).with_uv([0.25, 0.75]),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("a quad mesh");
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
fn a_textured_glb_exported_as_ply_carries_its_colour() -> Result<(), Box<dyn std::error::Error>> {
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
        "the colour was written: {:?}",
        report.warnings
    );

    // One file, and nothing beside it.
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
        !header.contains("OccluViewTexture"),
        "no image may be encoded into the header:\n{header}"
    );
    assert!(
        header.contains(
            "property uchar red\nproperty uchar green\nproperty uchar blue\nproperty uchar alpha\n"
        ),
        "the colour must be per vertex, the way a scanner writes it:\n{header}"
    );

    // And the same file, moved on its own, still opens with its colour.
    let reopened = dispatch_by_extension("ply", &bytes)?;
    assert!(reopened.has_vertex_colors(), "the colour came back");
    assert!(
        reopened.texture().is_none(),
        "no phantom image is attached to the reopened mesh"
    );
    // The image did not travel, so no coordinates point at one. Writing them
    // would make a reader enter an empty texture mode and draw a white shell.
    assert!(
        !header.contains("property float s"),
        "no dangling coordinates are written:\n{header}"
    );
    // Each corner samples the texel the viewer would have shown it.
    assert_eq!(reopened.vertices()[0].color, [255, 0, 0, 255]);
    assert_eq!(reopened.vertices()[1].color, [0, 255, 0, 255]);
    assert_eq!(reopened.vertices()[2].color, [255, 255, 0, 255]);
    assert_eq!(reopened.vertices()[3].color, [0, 0, 255, 255]);

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

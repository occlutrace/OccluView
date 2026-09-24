//! A coloured `.dcm` scan saved as PLY keeps its colour inside the single file.
//!
//! `.dcm` is the legacy extension of the 3Shape HPS container. This walks the
//! whole chain the operator takes — a textured HPS opened as the scan the
//! viewer shows, written out as one `.ply` — and checks that the image is in
//! the file, that nothing was written beside it, and that the pixels come back
//! unchanged. The colour is carried both ways a PLY can carry it: as the
//! embedded atlas, and as per-vertex RGBA when the source has no atlas.
//!
//! The fixture is the same packed-CC shape the CLI round-trip tests use, with a
//! two-triangle quad so the UVs are genuinely per corner.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::panic
)]

use occluview_formats::write::{write_mesh_to_new_file, MeshWriteFormat, MeshWriteOptions};
use occluview_formats::{dispatch_by_extension, MeshWriteWarning};

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn append_packed_uv(bytes: &mut Vec<u8>, u: f32, v: f32) {
    let pack = |component: f32| -> u16 { (component.clamp(0.0, 1.0) * 32767.0).round() as u16 };
    let packed = u32::from(pack(u)) | (u32::from(pack(v)) << 16);
    bytes.extend_from_slice(&packed.to_le_bytes());
}

/// The four pixels of the 2x2 atlas, one per corner, so a reordered image would
/// be caught rather than passing by accident.
const TEXTURE: [u8; 16] = [
    205, 164, 118, 255, 194, 151, 105, 255, 184, 144, 101, 255, 218, 176, 132, 255,
];

/// A 2x2 quad split into two triangles, with a corner UV per vertex.
fn textured_hps() -> Vec<u8> {
    let mut uv_bytes = Vec::new();
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        uv_bytes.push(1);
        append_packed_uv(&mut uv_bytes, u, v);
    }
    let mut vertex_bytes = Vec::new();
    for position in [
        [0.0_f32, 0.0, 0.0],
        [1.0_f32, 0.0, 0.0],
        [1.0_f32, 1.0, 0.0],
        [0.0_f32, 1.0, 0.0],
    ] {
        for component in position {
            vertex_bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    let faces = [4_u8, 0_u8];
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="2" base64_encoded_bytes="{faces_len}">{faces}</Facets>
        <Vertices vertex_count="4" base64_encoded_bytes="{vertices_len}">{vertices}</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
  <TextureData2>
    <PerVertexTextureCoord TextureCoordId="uv0" TextureId="tex0" Base64EncodedBytes="{uv_len}">{uvs}</PerVertexTextureCoord>
    <TextureImages>
      <TextureImage TextureId="tex0" RefTextureCoordId="uv0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="RGBA" Base64EncodedBytes="{texture_len}">{pixels}</TextureImage>
    </TextureImages>
  </TextureData2>
</HPS>"#,
        uv_len = uv_bytes.len(),
        uvs = encode_base64(&uv_bytes),
        faces_len = faces.len(),
        faces = encode_base64(&faces),
        vertices_len = vertex_bytes.len(),
        vertices = encode_base64(&vertex_bytes),
        texture_len = TEXTURE.len(),
        pixels = encode_base64(&TEXTURE),
    )
    .into_bytes()
}

/// The same quad with per-vertex RGB instead of an atlas: a colour scan whose
/// colour lives in the vertex data.
fn vertex_colored_hps() -> Vec<u8> {
    let colors = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
    let mut vertex_bytes = Vec::new();
    for position in [
        [0.0_f32, 0.0, 0.0],
        [1.0_f32, 0.0, 0.0],
        [1.0_f32, 1.0, 0.0],
        [0.0_f32, 1.0, 0.0],
    ] {
        for component in position {
            vertex_bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    let faces = [4_u8, 0_u8];
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="2" base64_encoded_bytes="{faces_len}">{faces}</Facets>
        <Vertices vertex_count="4" base64_encoded_bytes="{vertices_len}">{vertices}</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
  <VertexColorSets>
    <VertexColorSet Base64EncodedBytes="{colors_len}">{colors}</VertexColorSet>
  </VertexColorSets>
</HPS>"#,
        faces_len = faces.len(),
        faces = encode_base64(&faces),
        vertices_len = vertex_bytes.len(),
        vertices = encode_base64(&vertex_bytes),
        colors_len = colors.len(),
        colors = encode_base64(&colors),
    )
    .into_bytes()
}

fn export_directory(name: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("occluview-dcm-ply-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a temporary directory");
    path
}

fn only_entry(directory: &std::path::Path) -> String {
    let entries: Vec<String> = std::fs::read_dir(directory)
        .expect("read the folder")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries.len(), 1, "an export is one file, found {entries:?}");
    entries.into_iter().next().expect("one entry")
}

/// The operator's case: a `.dcm` scan with an atlas, saved as a PLY, carries
/// its colour inside that one file and nothing beside it.
#[test]
fn a_textured_dcm_saved_as_ply_keeps_its_colour_inside_the_file(
) -> Result<(), Box<dyn std::error::Error>> {
    // Opened through the same dispatch the viewer uses, under the legacy
    // extension, so the probe has to recognise the HPS content.
    let mesh = dispatch_by_extension("dcm", &textured_hps())?;
    assert!(
        mesh.texture().is_some(),
        "the .dcm scan must show its colour before the export"
    );
    assert!(mesh.has_uvs(), "and carry the mapping for it");

    let directory = export_directory("textured");
    let export = directory.join("scan-edited.ply");
    let report = write_mesh_to_new_file(
        &export,
        &mesh,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )?;
    assert!(
        !report
            .warnings
            .contains(&MeshWriteWarning::TextureImageNotWritten),
        "the colour was written: {:?}",
        report.warnings
    );

    // Exactly one file, and the image is inside it.
    let name = only_entry(&directory);
    assert_eq!(name, "scan-edited.ply");
    let bytes = std::fs::read(&export)?;
    let header_end = bytes
        .windows(b"end_header\n".len())
        .position(|window| window == b"end_header\n")
        .expect("a header terminator");
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(
        !header.contains("TextureFile"),
        "nothing may be named beside the export:\n{header}"
    );
    assert!(
        header.contains("comment OccluViewTextureFormat png")
            && header.contains("comment OccluViewTextureBase64 "),
        "the atlas must travel in the header:\n{header}"
    );
    assert!(
        header.contains("property list uchar float texcoord"),
        "the faces must carry the coordinates the atlas samples"
    );

    // Moved on its own, with the folder forgotten, the colour is still there.
    let reopened = dispatch_by_extension("ply", &bytes)?;
    let texture = reopened.texture().expect("the travelled atlas");
    assert_eq!((texture.width, texture.height), (2, 2));
    assert_eq!(texture.rgba, TEXTURE, "the pixels must survive unchanged");

    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

/// An HPS scan with no atlas but per-vertex colour keeps that colour through a
/// PLY export, so "saved in colour" does not depend on the image path alone.
#[test]
fn a_vertex_coloured_dcm_saved_as_ply_keeps_its_vertex_colour(
) -> Result<(), Box<dyn std::error::Error>> {
    let mesh = dispatch_by_extension("dcm", &vertex_colored_hps())?;
    assert!(
        mesh.has_vertex_colors(),
        "the .dcm scan must show its vertex colour before the export"
    );

    let directory = export_directory("vertex");
    let export = directory.join("scan-edited.ply");
    write_mesh_to_new_file(
        &export,
        &mesh,
        MeshWriteFormat::PlyBinaryLittleEndian,
        MeshWriteOptions::default(),
    )?;

    only_entry(&directory);
    let bytes = std::fs::read(&export)?;
    let header_end = bytes
        .windows(b"end_header\n".len())
        .position(|window| window == b"end_header\n")
        .expect("a header terminator");
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(
        header.contains("property uchar red")
            && header.contains("property uchar green")
            && header.contains("property uchar blue"),
        "the colour must be written as vertex properties:\n{header}"
    );

    let reopened = dispatch_by_extension("ply", &bytes)?;
    assert!(reopened.has_vertex_colors(), "the colour came back");
    let colors: Vec<[u8; 4]> = reopened.vertices().iter().map(|v| v.color).collect();
    assert_eq!(colors[0], [10, 20, 30, 255]);
    assert_eq!(colors[3], [100, 110, 120, 255]);

    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

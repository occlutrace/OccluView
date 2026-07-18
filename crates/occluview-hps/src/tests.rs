#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::float_cmp
)]

use super::*;
use std::io::{Cursor, Write};

struct StaticProvider(Vec<u8>);

impl HpsKeyProvider for StaticProvider {
    type Error = HpsError;

    fn base_key(&self) -> Result<Option<HpsSecretKey>, Self::Error> {
        HpsSecretKey::from_bytes(self.0.clone()).map(Some)
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn append_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn append_packed_uv(bytes: &mut Vec<u8>, u: f32, v: f32) {
    let pack = |component: f32| -> u16 { (component.clamp(0.0, 1.0) * 32767.0).round() as u16 };
    let packed = u32::from(pack(u)) | (u32::from(pack(v)) << 16);
    bytes.extend_from_slice(&packed.to_le_bytes());
}

fn red_png_bytes() -> Vec<u8> {
    let img = image::RgbaImage::from_raw(2, 2, [255, 0, 0, 255].repeat(4)).expect("image dims");
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

fn small_jpeg_bytes() -> Vec<u8> {
    let image =
        image::RgbImage::from_raw(2, 1, vec![220, 30, 20, 20, 30, 220]).expect("image dimensions");
    let mut buffer = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut buffer, image::ImageFormat::Jpeg)
        .expect("encode jpeg");
    buffer.into_inner()
}

fn sequential_vertex_bytes(vertex_count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertex_count * 12);
    for idx in 0..vertex_count {
        append_f32(&mut bytes, idx as f32);
        append_f32(&mut bytes, 0.0);
        append_f32(&mut bytes, 0.0);
    }
    bytes
}

fn cc_fixture(
    vertex_count: usize,
    face_count: usize,
    face_bytes: &[u8],
    extra_xml: &str,
) -> Vec<u8> {
    schema_fixture("CC", vertex_count, face_count, face_bytes, extra_xml)
}

fn schema_fixture(
    schema: &str,
    vertex_count: usize,
    face_count: usize,
    face_bytes: &[u8],
    extra_xml: &str,
) -> Vec<u8> {
    let vertices = sequential_vertex_bytes(vertex_count);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>{schema}</Schema>
    <Binary_data>
      <{schema} version="1.0">
        <Facets facet_count="{face_count}" base64_encoded_bytes="{face_len}">{faces}</Facets>
        <Vertices vertex_count="{vertex_count}" base64_encoded_bytes="{vertex_len}">{vertices}</Vertices>
      </{schema}>
    </Binary_data>
  </Packed_geometry>
{extra_xml}</HPS>"#,
        face_len = face_bytes.len(),
        faces = encode_base64(face_bytes),
        vertex_len = vertices.len(),
        vertices = encode_base64(&vertices),
    )
    .into_bytes()
}

fn zip_hps_fixture(path: &str, hps: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file(path, options)
            .expect("zip start_file should work");
        archive.write_all(hps).expect("zip write should work");
        archive.finish().expect("zip finish should work");
    }
    cursor.into_inner()
}

fn minimal_ce_hps_fixture() -> &'static [u8] {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CE</Schema>
    <Binary_data>
      <CE version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36" check_value="2130807316">zCbrd0TcI4bOxhSDOGslNswm63dE3COGzsYUgzhrJTbMJut3RNwjhg==</Vertices>
      </CE>
    </Binary_data>
  </Packed_geometry>
  <Properties>
    <Property name="EKID" value="1"/>
  </Properties>
</HPS>"#
}

fn test_key_provider() -> StaticProvider {
    StaticProvider((1_u8..=16).collect())
}

fn indices_from(bytes: &[u8]) -> Vec<u32> {
    read(bytes).expect("HPS should read").indices().to_vec()
}

#[test]
fn parses_minimal_cc_hps_triangle() {
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
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
    let mesh = read(xml).expect("CC HPS should read");
    assert_eq!(mesh.indices().len() / 3, 1);
    assert_eq!(mesh.indices(), &[0, 1, 2]);
    assert_eq!(mesh.positions()[1], [1.0, 0.0, 0.0]);
    assert_eq!(mesh.positions()[2], [0.0, 1.0, 0.0]);
}

#[test]
fn parses_hps_xml_inside_hps_zip_package() {
    let hps = cc_fixture(4, 2, &[4, 0], "");
    let package = zip_hps_fixture("scan/geometry.hps", &hps);

    let mesh = read(&package).expect("HPS package should read nested HPS XML");

    assert_eq!(mesh.indices().len() / 3, 2);
    assert_eq!(mesh.indices(), &[0, 1, 2, 3, 1, 0]);
}

#[test]
fn parses_unencrypted_ca_and_cb_hps_schemas() {
    for schema in ["CA", "CB"] {
        let mesh = read(&schema_fixture(schema, 4, 2, &[4, 0], ""))
            .expect("unencrypted HPS schema should read");
        assert_eq!(mesh.indices().len() / 3, 2, "schema={schema}");
        assert_eq!(mesh.indices(), &[0, 1, 2, 3, 1, 0], "schema={schema}");
    }
}

#[test]
fn ce_schema_is_deferred_until_key_provider_exists() {
    let xml = br"<HPS><Schema>CE</Schema></HPS>";
    assert!(matches!(read(xml), Err(HpsError::KeyMissing)));
}

#[test]
fn medical_dicom_is_not_treated_as_hps() {
    let mut medical = vec![0_u8; 132];
    medical[128..132].copy_from_slice(b"DICM");
    assert!(matches!(read(&medical), Err(HpsError::MedicalDicom)));
}

#[test]
fn parses_minimal_ce_hps_with_key_provider() {
    let mesh = read_with_key_provider(minimal_ce_hps_fixture(), &test_key_provider())
        .expect("CE HPS should decrypt with test key");
    assert_eq!(mesh.indices().len() / 3, 1);
    assert_eq!(mesh.indices(), &[0, 1, 2]);
    assert_eq!(mesh.positions()[1], [1.0, 0.0, 0.0]);
    assert_eq!(mesh.positions()[2], [0.0, 1.0, 0.0]);
}

#[test]
fn rejects_ce_hps_when_key_fails_integrity_check() {
    let bad_provider = StaticProvider((2_u8..=17).collect());
    let err = read_with_key_provider(minimal_ce_hps_fixture(), &bad_provider)
        .expect_err("wrong key must fail integrity check");
    assert!(matches!(
        err,
        ReadError::Parser(HpsError::IntegrityFailure { reason })
            if reason.contains("integrity")
    ));
}

#[test]
fn parses_face_command_stream_opcodes() {
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &[4, 0], "")),
        [0, 1, 2, 3, 1, 0]
    );
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &[4, 1], "")),
        [0, 1, 2, 0, 2, 1]
    );
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &[4, 2], "")),
        [0, 1, 2, 0, 2, 1]
    );
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &[4, 3, 0], "")),
        [0, 1, 2, 3, 2, 1]
    );
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &[4, 9, 0], "")),
        [0, 1, 2, 3, 2, 1]
    );
    assert_eq!(
        indices_from(&cc_fixture(5, 2, &[4, 10, 0], "")),
        [0, 1, 2, 4, 1, 0]
    );

    let mut restart16 = vec![5];
    restart16.extend_from_slice(&2_u16.to_le_bytes());
    restart16.extend_from_slice(&3_u16.to_le_bytes());
    restart16.extend_from_slice(&4_u16.to_le_bytes());
    assert_eq!(indices_from(&cc_fixture(5, 1, &restart16, "")), [2, 3, 4]);

    let mut absolute32 = vec![4, 8];
    absolute32.extend_from_slice(&3_u32.to_le_bytes());
    assert_eq!(
        indices_from(&cc_fixture(4, 2, &absolute32, "")),
        [0, 1, 2, 3, 1, 0]
    );
}

#[test]
fn vertex_color_set_expands_rgb_to_rgba() {
    let rgb = [10, 20, 30, 40, 50, 60, 70, 80, 90];
    let extra = format!(
        r#"  <VertexColorSets>
    <VertexColorSet Base64EncodedBytes="{}">{}</VertexColorSet>
  </VertexColorSets>
"#,
        rgb.len(),
        encode_base64(&rgb)
    );
    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("colored HPS should read");
    let colors = mesh.colors().expect("vertex colors should be present");
    assert_eq!(colors[0], [10, 20, 30, 255]);
    assert_eq!(colors[2], [70, 80, 90, 255]);
}

#[test]
fn default_color_attribute_fills_vertices() {
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1" color="0x336699">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
    </Packed_geometry>
</HPS>"#;
    let mesh = read(xml).expect("default color HPS should read");
    let colors = mesh.colors().expect("default colors should be present");
    assert_eq!(colors[0], [0x33, 0x66, 0x99, 255]);
    assert_eq!(colors[2], [0x33, 0x66, 0x99, 255]);
}

#[test]
fn neutral_hps_default_color_does_not_mark_scan_as_colored() {
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1" color="8421504">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
    </Packed_geometry>
</HPS>"#;
    let mesh = read(xml).expect("neutral default-color HPS should read");
    assert!(
        mesh.colors().is_none(),
        "HPS's uniform neutral gray default color is metadata, not real scan color"
    );
}

#[test]
fn texture_data_splits_corner_uvs_and_attaches_texture() {
    let mut uv_bytes = Vec::new();
    uv_bytes.push(2);
    append_packed_uv(&mut uv_bytes, 0.0, 0.0);
    append_packed_uv(&mut uv_bytes, 0.75, 0.25);
    uv_bytes.push(2);
    append_packed_uv(&mut uv_bytes, 1.0, 0.0);
    append_packed_uv(&mut uv_bytes, 1.0, 1.0);
    uv_bytes.push(1);
    append_packed_uv(&mut uv_bytes, 0.5, 1.0);
    uv_bytes.push(1);
    append_packed_uv(&mut uv_bytes, 0.25, 0.75);

    let png_bytes = red_png_bytes();
    let extra = format!(
        r#"  <TextureData2>
    <PerVertexTextureCoord TextureCoordId="uv0" TextureId="tex0" Base64EncodedBytes="{}">{}</PerVertexTextureCoord>
    <TextureImages>
      <TextureImage TextureId="tex0" RefTextureCoordId="uv0" Width="2" Height="2" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        uv_bytes.len(),
        encode_base64(&uv_bytes),
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(4, 2, &[4, 0], &extra)).expect("textured HPS should read");
    assert_eq!(mesh.indices(), &[0, 1, 2, 3, 4, 5]);
    assert_eq!(mesh.positions().len(), 6);
    let uvs = mesh.uvs().expect("texture coordinates should be present");
    assert_eq!(uvs[0], [0.0, 0.0]);
    assert!((uvs[5][0] - 0.75).abs() < 0.0001);
    assert!((uvs[5][1] - 0.25).abs() < 0.0001);

    let texture = mesh.texture().expect("HPS texture should be attached");
    assert_eq!(texture.width(), 2);
    assert_eq!(texture.height(), 2);
    assert!(texture
        .rgba()
        .chunks_exact(4)
        .all(|pixel| pixel == [255, 0, 0, 255]));
}

#[test]
fn raw_bgra_texture_image_converts_to_rgba() {
    let raw_bgra = [
        12, 34, 200, 255, // R=200, G=34, B=12
        90, 80, 70, 255, // R=70, G=80, B=90
        3, 2, 1, 255, // R=1, G=2, B=3
        30, 20, 10, 128, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="BGRA" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(texture.width(), 2);
    assert_eq!(texture.height(), 2);
    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128,]
    );
}

#[test]
fn raw_rgba_texture_image_keeps_declared_rgba_order() {
    let raw_rgba = [
        200, 34, 12, 255, // R=200, G=34, B=12
        70, 80, 90, 255, // R=70, G=80, B=90
        1, 2, 3, 255, // R=1, G=2, B=3
        10, 20, 30, 128, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="RGBA" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_rgba.len(),
        encode_base64(&raw_rgba)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(texture.rgba(), &raw_rgba);
}

#[test]
fn raw_argb_texture_image_keeps_declared_argb_order() {
    let raw_argb = [
        255, 200, 34, 12, // R=200, G=34, B=12
        255, 70, 80, 90, // R=70, G=80, B=90
        255, 1, 2, 3, // R=1, G=2, B=3
        128, 10, 20, 30, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="ARGB" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_argb.len(),
        encode_base64(&raw_argb)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128]
    );
}

#[test]
fn raw_abgr_texture_image_keeps_declared_abgr_order() {
    let raw_abgr = [
        255, 12, 34, 200, // R=200, G=34, B=12
        255, 90, 80, 70, // R=70, G=80, B=90
        255, 3, 2, 1, // R=1, G=2, B=3
        128, 30, 20, 10, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="ABGR" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_abgr.len(),
        encode_base64(&raw_abgr)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128]
    );
}

#[test]
fn raw_rgb_texture_image_keeps_declared_rgb_order() {
    let raw_rgb = [
        200, 34, 12, // R=200, G=34, B=12
        70, 80, 90, // R=70, G=80, B=90
        1, 2, 3, // R=1, G=2, B=3
        10, 20, 30, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="3" Format="RGB" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_rgb.len(),
        encode_base64(&raw_rgb)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 255]
    );
}

#[test]
fn compressed_texture_uses_decoded_dimensions_before_raw_metadata_limits() {
    let jpeg = small_jpeg_bytes();
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="8192" Height="4096" BytesPerPixel="3" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        jpeg.len(),
        encode_base64(&jpeg)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra))
        .expect("a compressed texture must not be rejected as raw RGBA");
    let texture = mesh
        .texture()
        .expect("compressed HPS texture should be attached");

    assert_eq!((texture.width(), texture.height()), (2, 1));
    assert_eq!(texture.rgba().len(), 2 * 4);
}

// A format-less raw HPS texture MUST decode deterministically as BGRA — HPS
// emits DirectX surfaces (D3DFMT_A8R8G8B8) whose memory byte order is [B,G,R,A].
// This is the owner-verified-correct behavior: a warm-white dental surface
// (physical R>=G>B) is stored with the small blue value in byte 0, and swapping
// R<->B is what keeps enamel warm instead of turning it blue.
#[test]
fn raw_texture_image_without_format_defaults_to_bgra_swap() {
    // Bytes are a warm-white enamel patch stored BGRA: byte0=B(small) .. byte2=R(large).
    let raw_bgra = [
        118, 164, 205, 255, 105, 151, 194, 255, 101, 144, 184, 255, 132, 176, 218, 255,
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    // R<->B swapped: warm-white enamel (R>B), never cool blue.
    assert_eq!(
        texture.rgba(),
        vec![205, 164, 118, 255, 194, 151, 105, 255, 184, 144, 101, 255, 218, 176, 132, 255]
    );
    for pixel in texture.rgba().chunks_exact(4) {
        assert!(
            pixel[0] > pixel[2],
            "format-less HPS enamel must stay warm (R>B), never blue: {pixel:?}"
        );
    }
}

// Regression for the owner bug ("где белое — синим красит"): a texture atlas
// dominated by cool/neutral stone with a minority of warm-white enamel decodes
// deterministically as BGRA, so the enamel stays warm regardless of what the
// rest of the atlas looks like — no per-scan pixel-statistics guessing.
#[test]
fn raw_texture_image_cool_dominant_atlas_keeps_enamel_warm() {
    let mut raw_bgra = Vec::new();
    // Cool-neutral stone (physical R=210,G=214,B=220) stored BGRA -> [220,214,210].
    for _ in 0..13 {
        raw_bgra.extend_from_slice(&[220, 214, 210, 255]);
    }
    // Warm-white enamel (physical R=248,G=244,B=236) stored BGRA -> [236,244,248].
    for _ in 0..3 {
        raw_bgra.extend_from_slice(&[236, 244, 248, 255]);
    }
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    // Enamel pixels are the last three; they must be warm (R>B), not blue.
    let enamel = &texture.rgba()[13 * 4..];
    for pixel in enamel.chunks_exact(4) {
        assert!(
            pixel[0] > pixel[2],
            "warm-white enamel rendered blue under a cool-dominant atlas: {pixel:?}"
        );
    }
    // And the cool stone is faithfully reproduced (R<B), not warm-flipped.
    assert_eq!(&texture.rgba()[0..4], &[210, 214, 220, 255]);
}

// A file that declares the DirectX pixel-format NAME D3DFMT_A8R8G8B8 (0xAARRGGBB)
// stores memory bytes [B,G,R,A]. It must decode as BGRA (swap R<->B), NOT as the
// literal channel order ARGB — the historic ARGB mismap forced blue = alpha byte,
// painting entire scans blue.
#[test]
fn raw_a8r8g8b8_directx_name_decodes_as_bgra() {
    // memory bytes for a warm-white pixel: [B=236, G=244, R=248, A=255]
    let raw = [
        236, 244, 248, 255, 236, 244, 248, 255, 236, 244, 248, 255, 236, 244, 248, 255,
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="A8R8G8B8" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw.len(),
        encode_base64(&raw)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    for pixel in texture.rgba().chunks_exact(4) {
        assert_eq!(
            pixel,
            [248, 244, 236, 255],
            "A8R8G8B8 must decode to warm BGRA"
        );
    }
}

fn solid_rgba_png_bytes(
    width: u32,
    height: u32,
    pixel: [u8; 4],
    transparent_corner: bool,
) -> Vec<u8> {
    let mut data = pixel.repeat((width * height) as usize);
    if transparent_corner {
        data[3] = 0; // First pixel fully transparent: must not skew the sample.
    }
    let img = image::RgbaImage::from_raw(width, height, data).expect("image dims");
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

// Regression for a real owner-reported bug: a 3Shape/HPS dental scan whose
// embedded JPEG texture atlas has its chroma channels swapped AT THE SOURCE
// (standards-compliant decode still comes out blue — there is no container
// pixel-format ambiguity to resolve here, unlike the raw-D3DFMT tests above).
// Calibrated against the real file's measured statistics: swapped mean
// R≈107/B≈150, corrected mean R≈150/B≈107.
#[test]
fn embedded_png_with_swapped_dental_chroma_is_corrected_to_warm() {
    let png_bytes = solid_rgba_png_bytes(4, 4, [107, 117, 150, 255], true);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    let opaque_pixels: Vec<&[u8]> = texture.rgba().chunks_exact(4).skip(1).collect();
    for pixel in opaque_pixels {
        assert_eq!(
            pixel,
            [150, 117, 107, 255],
            "swapped dental chroma must be corrected back to warm (R>B)"
        );
    }
    // The transparent corner pixel must not itself be mangled by the swap
    // guard sampling it should have skipped in the first place.
    assert_eq!(texture.rgba()[3], 0);
}

// A mildly cool tint (a bluish stone shade, or a cool composite light) stays
// well under the implausible-blue margin and must NOT be flipped — the
// physical prior only fires on a whole-texture bias far beyond normal
// material/lighting variation. The 20-value gap clears the near-gray filter
// (so this genuinely exercises the margin comparison, not the gray skip) but
// stays under `red_mean / 4` (150 / 4 = 37).
#[test]
fn embedded_png_with_a_mild_cool_tint_is_left_untouched() {
    let pixel = [150, 160, 170, 255]; // R=150, B=170: a 20-value cool tint.
    let png_bytes = solid_rgba_png_bytes(4, 4, pixel, false);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    for decoded in texture.rgba().chunks_exact(4) {
        assert_eq!(
            decoded, pixel,
            "a mild cool tint must not be treated as a channel-order bug"
        );
    }
}

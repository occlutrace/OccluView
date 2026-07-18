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

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
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

pub(crate) fn append_packed_uv(bytes: &mut Vec<u8>, u: f32, v: f32) {
    let pack = |component: f32| -> u16 { (component.clamp(0.0, 1.0) * 32767.0).round() as u16 };
    let packed = u32::from(pack(u)) | (u32::from(pack(v)) << 16);
    bytes.extend_from_slice(&packed.to_le_bytes());
}

pub(crate) fn red_png_bytes() -> Vec<u8> {
    let img = image::RgbaImage::from_raw(2, 2, [255, 0, 0, 255].repeat(4)).expect("image dims");
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

pub(crate) fn small_jpeg_bytes() -> Vec<u8> {
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

pub(crate) fn cc_fixture(
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

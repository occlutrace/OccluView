//! Black-box contract tests for the public HPS converter binary.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::unwrap_used
)]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

const KEY_ENV_VARS: &[&str] = &[
    "OCCLUVIEW_HPS_ENCRYPTION_KEY",
    "OCCLUVIEW_HPS_KEY",
    "OCCLUTRACE_HPS_ENCRYPTION_KEY",
    "HPS_ENCRYPTION_KEY",
];

const UNTEXTURED_HPS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
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

const ENCRYPTED_HPS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
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
</HPS>"#;

fn converter_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_occluview-hps-export"));
    for variable in KEY_ENV_VARS {
        command.env_remove(variable);
    }
    command
}

fn run_with_stdin(command: &mut Command, stdin: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("converter should start");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(stdin)
        .expect("write converter stdin");
    child.wait_with_output().expect("converter should finish")
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("output should be JSON")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

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
    let texture = [
        205, 164, 118, 255, 194, 151, 105, 255, 184, 144, 101, 255, 218, 176, 132, 255,
    ];
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
        texture_len = texture.len(),
        pixels = encode_base64(&texture),
    )
    .into_bytes()
}

fn assert_manifest_header(manifest: &Value) {
    assert_eq!(manifest["schema_version"], 2);
    assert_eq!(manifest["parser_version"], env!("CARGO_PKG_VERSION"));
}

fn assert_manifest_artifact(
    manifest: &Value,
    field: &str,
    expected_format: &str,
    expected_path: &str,
    artifact: &[u8],
) {
    assert_eq!(manifest[field]["format"], expected_format);
    assert_eq!(manifest[field]["path"], expected_path);
    assert_eq!(manifest[field]["sha256"], sha256_hex(artifact));
}

fn assert_json_error(output: &Output, exit_code: i32, error_code: &str) -> Value {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(output.stdout.is_empty(), "failures must not write stdout");
    let body = parse_json(&output.stderr);
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["error"]["code"], error_code);
    assert_eq!(body["error"]["exit_code"], exit_code);
    body
}

#[test]
fn file_input_writes_fixed_untextured_ply_and_hashed_manifest() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("patient-jane-doe-private-scan.dcm");
    let output_dir = temp.path().join("artifacts");
    fs::write(&input, UNTEXTURED_HPS).expect("write fixture");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert!(output.status.success(), "stderr={:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let artifact = fs::read(output_dir.join("surface.ply")).expect("PLY artifact");
    assert!(artifact.starts_with(b"ply\nformat binary_little_endian 1.0\n"));
    assert!(!output_dir.join("surface.glb").exists());
    let stdout_manifest = parse_json(&output.stdout);
    let file_manifest =
        parse_json(&fs::read(output_dir.join("manifest.json")).expect("manifest artifact"));
    assert_eq!(stdout_manifest, file_manifest);
    assert_manifest_header(&file_manifest);
    assert_manifest_artifact(&file_manifest, "geometry", "ply", "surface.ply", &artifact);
    assert!(file_manifest.get("preview").is_none());
}

#[test]
fn stdin_textured_input_writes_geometry_ply_and_self_contained_glb_preview() {
    let temp = TempDir::new().expect("temp dir");
    let output_dir = temp.path().join("artifacts");
    let output = run_with_stdin(
        converter_command()
            .args(["--input", "-", "--output-dir"])
            .arg(&output_dir),
        &textured_hps(),
    );

    assert!(output.status.success(), "stderr={:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let geometry = fs::read(output_dir.join("surface.ply")).expect("PLY geometry artifact");
    assert!(geometry.starts_with(b"ply\nformat binary_little_endian 1.0\n"));
    let geometry_header = String::from_utf8_lossy(&geometry);
    assert!(geometry_header.contains("element vertex 4\n"));
    assert!(geometry_header.contains("element face 2\n"));
    let preview = fs::read(output_dir.join("surface.glb")).expect("GLB preview artifact");
    assert!(preview.starts_with(b"glTF"));
    let mesh = occluview_formats::gltf::read(&preview).expect("GLB should be self-contained");
    assert_eq!(mesh.vertices().len(), 4);
    assert_eq!(mesh.indices(), &[0, 1, 2, 3, 1, 0]);
    assert!(mesh.texture().is_some());
    let manifest = parse_json(&output.stdout);
    assert_manifest_header(&manifest);
    assert_manifest_artifact(&manifest, "geometry", "ply", "surface.ply", &geometry);
    assert_manifest_artifact(&manifest, "preview", "glb", "surface.glb", &preview);
}

#[test]
fn missing_runtime_key_is_typed_and_does_not_create_artifacts() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("encrypted.hps");
    let output_dir = temp.path().join("artifacts");
    fs::write(&input, ENCRYPTED_HPS).expect("write fixture");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 4, "key_missing");
    assert!(!output_dir.join("surface.ply").exists());
    assert!(!output_dir.join("surface.glb").exists());
    assert!(!output_dir.join("manifest.json").exists());
}

#[test]
fn malformed_input_error_never_echoes_input_or_output_paths_or_payload() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("patient-alice-smith-secret-scan.dcm");
    let output_dir = temp.path().join("patient-alice-smith-secret-output");
    let phi = "patient-alice-smith-secret";
    fs::write(&input, format!("not a HPS package: {phi}")).expect("write fixture");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 5, "bad_signature");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 JSON error");
    assert!(!stderr.contains(phi));
    assert!(!stderr.contains(input.to_string_lossy().as_ref()));
    assert!(!stderr.contains(output_dir.to_string_lossy().as_ref()));
}

#[test]
fn malformed_environment_key_is_typed_and_never_disclosed() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("encrypted.hps");
    let output_dir = temp.path().join("artifacts");
    let secret = "TOP-SECRET-KEY-MATERIAL-THAT-MUST-NEVER-LEAK".repeat(2);
    fs::write(&input, ENCRYPTED_HPS).expect("write fixture");

    let output = converter_command()
        .env("OCCLUVIEW_HPS_ENCRYPTION_KEY", &secret)
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 4, "invalid_key");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&secret));
}

#[test]
fn key_material_is_not_accepted_on_the_command_line_or_echoed() {
    let temp = TempDir::new().expect("temp dir");
    let secret = "argv-secret-key-material";
    let output = converter_command()
        .args(["--hps-key", secret, "--output-dir"])
        .arg(temp.path())
        .output()
        .expect("run converter");

    assert_json_error(&output, 2, "invalid_arguments");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
}

#[test]
fn missing_input_file_error_does_not_echo_the_path() {
    let temp = TempDir::new().expect("temp dir");
    let missing = temp.path().join("patient-bob-private-missing.dcm");
    let output_dir = temp.path().join("artifacts");
    let output = converter_command()
        .args(["--input"])
        .arg(&missing)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 3, "input_read_failed");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("patient-bob-private"));
}

#[test]
fn reserved_output_collision_is_typed_and_does_not_overwrite() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("input.hps");
    let output_dir = temp.path().join("artifacts");
    fs::write(&input, UNTEXTURED_HPS).expect("write fixture");
    fs::create_dir_all(&output_dir).expect("create output dir");
    fs::write(output_dir.join("surface.ply"), b"existing").expect("seed collision");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 6, "output_exists");
    assert_eq!(
        fs::read(output_dir.join("surface.ply")).expect("existing artifact"),
        b"existing"
    );
}

#[test]
fn encrypted_input_uses_runtime_environment_key_without_disclosure() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("encrypted.hps");
    let output_dir = temp.path().join("artifacts");
    let key = (1_u8..=16)
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>()
        .join(",");
    fs::write(&input, ENCRYPTED_HPS).expect("write fixture");

    let output = converter_command()
        .env("OCCLUVIEW_HPS_ENCRYPTION_KEY", &key)
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert!(output.status.success(), "stderr={:?}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(output_dir.join("surface.ply").is_file());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&key));
}

#[test]
fn medical_dicom_is_rejected_with_a_stable_typed_error() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("medical-private.dcm");
    let output_dir = temp.path().join("artifacts");
    let mut medical_dicom = vec![0_u8; 132];
    medical_dicom[128..132].copy_from_slice(b"DICM");
    fs::write(&input, medical_dicom).expect("write fixture");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 5, "medical_dicom_unsupported");
}

#[test]
fn help_and_version_do_not_expose_a_key_argument() {
    let help = converter_command()
        .arg("--help")
        .output()
        .expect("run help");
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).expect("UTF-8 help");
    assert!(help.contains("--input <FILE|-> --output-dir <DIR>"));
    assert!(!help.contains("--hps-key"));
    assert!(!help.contains("--key"));

    let version = converter_command()
        .arg("--version")
        .output()
        .expect("run version");
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    assert_eq!(
        String::from_utf8(version.stdout).expect("UTF-8 version"),
        format!("occluview-hps-export {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unusable_output_directory_error_does_not_echo_the_path() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("input.hps");
    let output_dir = temp.path().join("patient-private-output-file");
    fs::write(&input, UNTEXTURED_HPS).expect("write fixture");
    fs::write(&output_dir, b"not a directory").expect("seed output file");

    let output = converter_command()
        .args(["--input"])
        .arg(&input)
        .args(["--output-dir"])
        .arg(&output_dir)
        .output()
        .expect("run converter");

    assert_json_error(&output, 6, "output_directory_failed");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("patient-private"));
}

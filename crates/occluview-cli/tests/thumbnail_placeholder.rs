//! Integration contract for placeholder output from `occluview-cli thumbnail`.

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

fn unique_tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("occluview-cli-thumb-{nanos}-{name}"))
}

/// A truncated binary STL fixture.
fn corrupt_stl_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; 84];
    bytes[..7].copy_from_slice(b"corrupt");
    bytes[80..84].copy_from_slice(&5_000_000u32.to_le_bytes());
    bytes.extend_from_slice(b"not-a-triangle-soup");
    bytes
}

#[test]
fn thumbnail_of_corrupt_file_exits_zero_and_writes_placeholder_png() {
    let input = unique_tmp("garbage.stl");
    let output = unique_tmp("garbage.png");
    std::fs::write(&input, corrupt_stl_bytes()).expect("write corrupt STL fixture");

    let status = Command::new(env!("CARGO_BIN_EXE_occluview-cli"))
        .args(["thumbnail"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--size", "128"])
        .status()
        .expect("run occluview-cli thumbnail");

    assert!(
        status.success(),
        "thumbnailing a corrupt file must exit 0 so the file manager shows the \
         placeholder instead of a broken-image glyph (got {status:?})"
    );

    let bytes = std::fs::read(&output).expect("placeholder PNG must be written");
    let image = image::load_from_memory(&bytes)
        .expect("output must be a valid PNG")
        .to_rgba8();
    assert_eq!(image.width(), 128);
    assert_eq!(image.height(), 128);
    // The placeholder cube has an opaque body over a transparent background.
    let any_opaque = image.pixels().any(|px| px.0[3] == 255);
    let any_transparent = image.pixels().any(|px| px.0[3] == 0);
    assert!(any_opaque, "placeholder should have an opaque cube body");
    assert!(
        any_transparent,
        "placeholder should keep a transparent background"
    );

    let _ = std::fs::remove_file(input);
    let _ = std::fs::remove_file(output);
}

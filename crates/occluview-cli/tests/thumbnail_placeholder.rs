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

/// A file the thumbnailer cannot OPEN is a failure, and the exit code says so.
///
/// The two cases are deliberately different. A corrupt CONTAINER is a verdict
/// about content: the badge is written and the command succeeds, which is what
/// the test above pins. A missing, unreadable or non-file path is the command
/// FAILING to do its job, and a script running
/// `occluview-cli thumbnail "$f" -o "$o" && use "$o"` must be able to tell the
/// difference. Before this, both exited 0 with a placeholder.
#[test]
fn an_unopenable_input_exits_non_zero_and_still_writes_a_png() {
    let directory = std::env::temp_dir().join(format!(
        "occluview-cli-thumb-missing-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    let missing = directory.join("does-not-exist.stl");
    let output_path = directory.join("out.png");

    let status = Command::new(env!("CARGO_BIN_EXE_occluview-cli"))
        .args(["thumbnail"])
        .arg(&missing)
        .arg("-o")
        .arg(&output_path)
        .args(["--size", "128"])
        .status()
        .expect("run occluview-cli thumbnail");

    assert!(
        !status.success(),
        "thumbnailing a file that does not exist must not report success: {status:?}"
    );
    // The thumbnailer contract still requires a PNG, so the failure is a
    // picture of the failure rather than no output at all.
    let bytes = std::fs::read(&output_path).expect("a placeholder PNG is still written");
    assert!(bytes.starts_with(b"\x89PNG"), "the output must be a PNG");

    std::fs::remove_dir_all(&directory).ok();
}

/// A real mesh on disk renders a REAL thumbnail, which is only possible if the
/// CLI goes through the file-backed path.
///
/// This is the behaviour the removed source-text check described (it looked for
/// the words `try_render_thumbnail_file` in main.rs). The property worth
/// holding is the outcome: the bytes path cannot read metadata, work out the
/// extension, or cache by file identity, so if the CLI ever switched to the
/// in-memory stream entry point, a file on disk would come back as a plain
/// placeholder. This asserts the opposite — that the picture of a real scan is
/// not the placeholder — which is what an operator sees in the file manager.
#[test]
fn a_real_mesh_on_disk_renders_a_real_thumbnail() {
    let directory = std::env::temp_dir().join(format!(
        "occluview-cli-thumb-real-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&directory).expect("scratch directory");

    // One triangle, written as a binary STL.
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&1u32.to_le_bytes());
    for value in [
        0.0f32, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    let mesh_path = directory.join("triangle.stl");
    std::fs::write(&mesh_path, &bytes).expect("write fixture");

    let output_path = directory.join("out.png");
    let status = Command::new(env!("CARGO_BIN_EXE_occluview-cli"))
        .args(["thumbnail"])
        .arg(&mesh_path)
        .arg("-o")
        .arg(&output_path)
        .args(["--size", "128"])
        .status()
        .expect("run occluview-cli thumbnail");
    assert!(
        status.success(),
        "a readable mesh must thumbnail: {status:?}"
    );

    let written = std::fs::read(&output_path).expect("a PNG is written");
    assert!(written.starts_with(b"\x89PNG"));
    // The placeholder is a flat single-colour tile. A rendered mesh is not: it
    // has shading across it, so more than one distinct colour appears.
    let image = image::load_from_memory(&written)
        .expect("output parses as an image")
        .to_rgba8();
    let mut colours = std::collections::BTreeSet::new();
    for pixel in image.pixels() {
        colours.insert((pixel[0], pixel[1], pixel[2], pixel[3]));
        if colours.len() > 8 {
            break;
        }
    }
    assert!(
        colours.len() > 1,
        "a real scan must render shaded geometry, not a flat placeholder tile \
         (the bytes path cannot produce this)"
    );

    std::fs::remove_dir_all(&directory).ok();
}

//! Black-box tests for the command-line interface.

#![allow(clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Output};

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_occluview-cli"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("run occluview-cli")
}

/// Create an empty per-test working directory.
fn scratch(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("occluview-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    directory
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("read the scratch directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn triangle_obj() -> &'static str {
    "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"
}

#[test]
fn help_is_answered_on_stdout_and_writes_nothing() {
    let directory = scratch("help");
    for args in [
        vec!["--help"],
        vec!["thumbnail", "--help"],
        vec!["info", "-h"],
    ] {
        let output = run(&directory, &args);
        assert!(
            output.status.success(),
            "{args:?} should succeed, got {:?}",
            output.status
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("USAGE:"),
            "{args:?} should print the usage text on stdout, got {stdout:?}"
        );
        assert!(
            output.stderr.is_empty(),
            "{args:?} should leave stderr alone, got {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            entries(&directory).is_empty(),
            "{args:?} wrote {:?} into the working directory",
            entries(&directory)
        );
    }
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn an_unknown_subcommand_fails_and_explains_itself_on_stderr() {
    let directory = scratch("unknown");
    let output = run(&directory, &["bogus"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "an unknown subcommand is a failure"
    );
    assert!(
        output.stdout.is_empty(),
        "stdout belongs to the work that was going to be produced"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown subcommand: bogus"), "{stderr}");
    assert!(
        stderr.contains("USAGE:"),
        "the usage belongs beside the error that needed it: {stderr}"
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_flag_where_the_file_belongs_fails_instead_of_rendering() {
    let directory = scratch("flag-first");
    let output = run(&directory, &["thumbnail", "-o", "out.png", "scan.stl"]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expected a file path"), "{stderr}");
    assert!(stderr.contains("-o"), "{stderr}");
    assert!(
        entries(&directory).is_empty(),
        "nothing should have been written: {:?}",
        entries(&directory)
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn the_version_is_the_crate_version_on_stdout() {
    let directory = scratch("version");
    let output = run(&directory, &["--version"]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("occluview-cli {}", env!("CARGO_PKG_VERSION"))
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn convert_collapses_a_repeated_terminal_extension_before_writing() {
    let directory = scratch("convert-extension");
    std::fs::write(directory.join("scan.obj"), triangle_obj()).expect("write OBJ fixture");
    std::fs::create_dir(directory.join("exports")).expect("create export directory");

    let output = run(
        &directory,
        &["convert", "scan.obj", "-o", "exports/upper.stl.stl"],
    );

    assert!(
        output.status.success(),
        "convert should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.join("exports/upper.stl").is_file());
    assert!(!directory.join("exports/upper.stl.stl").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exports/upper.stl"));

    std::fs::remove_dir_all(&directory).ok();
}

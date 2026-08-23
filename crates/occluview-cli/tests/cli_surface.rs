//! What the command line does, run as a command line.
//!
//! The unit tests next to the parser cover which argument means what. These
//! run the built binary, because the things that went wrong here were about
//! the process: a stray file written into the working directory, a help text
//! that could not be piped, an exit code that said success after refusing to
//! do the work.

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

/// A scratch directory of this test's own, emptied first.
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

#[test]
fn help_is_answered_on_stdout_and_writes_nothing() {
    // `--help` used to be answered on stderr, which cannot be piped into a
    // pager or a file without redirecting the error stream. And in the
    // subcommand position it was read as a filename: `thumbnail --help`
    // rendered a placeholder cube into ./--help.png and exited 0.
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
    // `thumbnail -o out.png scan.stl` opened a file called `-o`.
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

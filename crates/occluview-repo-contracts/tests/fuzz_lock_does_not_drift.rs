//! Asserts the fuzz crate resolves the same dependency versions as the workspace.
//!
//! `fuzz/Cargo.lock` is a second dependency graph over the same sources: the
//! fuzz targets compile the workspace crates against their own lockfile. When
//! a package present in both lockfiles resolves to different versions, the
//! fuzzer exercises code the shipped build never runs, so a finding cleared
//! by fuzzing may still ship.

#![allow(clippy::expect_used, clippy::panic)] // a missing lockfile is a test failure

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type LockPackages = BTreeMap<String, BTreeSet<String>>;

/// The `(name, version)` pairs of every `[[package]]` in a lockfile.
fn lock_packages(text: &str) -> LockPackages {
    let mut packages: LockPackages = BTreeMap::new();
    let mut name: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name = ") {
            name = rest
                .strip_prefix('"')
                .and_then(|quoted| quoted.strip_suffix('"'));
        } else if let Some(rest) = trimmed.strip_prefix("version = ") {
            if let (Some(package), Some(version)) = (
                name,
                rest.strip_prefix('"')
                    .and_then(|quoted| quoted.strip_suffix('"')),
            ) {
                packages
                    .entry(package.to_owned())
                    .or_default()
                    .insert(version.to_owned());
            }
            name = None;
        }
    }
    packages
}

fn versions_list(versions: &BTreeSet<String>) -> String {
    versions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn fuzz_lock_does_not_drift() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest is inside the workspace");
    let workspace_text =
        std::fs::read_to_string(root.join("Cargo.lock")).expect("workspace lockfile is readable");
    let fuzz_text =
        std::fs::read_to_string(root.join("fuzz/Cargo.lock")).expect("fuzz lockfile is readable");
    let workspace = lock_packages(&workspace_text);
    let fuzz = lock_packages(&fuzz_text);
    assert!(
        !workspace.is_empty(),
        "the workspace lockfile holds packages"
    );
    assert!(!fuzz.is_empty(), "the fuzz lockfile holds packages");

    let drift: Vec<String> = workspace
        .iter()
        .filter_map(|(name, workspace_versions)| {
            fuzz.get(name).and_then(|fuzz_versions| {
                (!fuzz_versions.is_subset(workspace_versions)).then(|| {
                    format!(
                        "{name}: workspace [{}] vs fuzz [{}]",
                        versions_list(workspace_versions),
                        versions_list(fuzz_versions)
                    )
                })
            })
        })
        .collect();
    assert!(
        drift.is_empty(),
        "every fuzz package version is one the workspace also resolves:\n{}",
        drift.join("\n")
    );
}

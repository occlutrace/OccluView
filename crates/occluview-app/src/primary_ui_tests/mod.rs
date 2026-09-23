// A contract test that cannot find its subject must say so. See
// `repo_source_file` below for why that needs a panic rather than a default.
#![allow(clippy::panic)]

pub(super) use super::*;
use std::path::Path;

mod chrome;
mod documents;
mod loading;
mod platform;
mod presentation_sinks;
mod source_tree;
mod tools;
mod viewport;

/// Every `.rs` file under `directory`, skipping symlinks and any `target`
/// directory so a local build tree cannot pollute a source-tree audit.
pub(super) fn collect_rust_source_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() || path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        if file_type.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        {
            files.push(path);
        }
    }
    Ok(())
}

pub(super) fn main_source() -> &'static str {
    include_str!("../main.rs")
}
/// Canonical home of the application module graph since the library
/// boundary (Stage C): `main.rs` only delegates to the public entry here.
pub(super) fn lib_source() -> &'static str {
    include_str!("../lib.rs")
}

pub(super) fn app_bootstrap_source() -> &'static str {
    include_str!("../app_bootstrap.rs")
}

/// Read a source file this crate makes assertions about.
///
/// The sibling mechanism, `include_str!`, is checked by the compiler: rename
/// the file and the build breaks. This one is not, so it has to break itself.
/// Returning `""` on a missing file turns every assertion about that file into
/// an assertion about the empty string -- silently, with CI green. The negative
/// assertions go first, and those are the ones worth having; a line budget
/// passes in a vacuum too, since `"".lines().count()` is zero.
pub(super) fn repo_source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "contract test source {} is missing: {error}",
            path.display()
        )
    })
}

pub(super) fn app_manifest_source() -> &'static str {
    include_str!("../../Cargo.toml")
}

pub(super) fn ci_workflow_source() -> &'static str {
    include_str!("../../../../.github/workflows/ci.yml")
}

pub(super) fn package_workflow_source() -> &'static str {
    include_str!("../../../../.github/workflows/package-msi.yml")
}

pub(super) fn msi_wxs_source() -> &'static str {
    include_str!("../../../../install/occluview.wxs")
}

pub(super) fn linux_build_deb_source() -> &'static str {
    include_str!("../../../../install/linux/build-deb.sh")
}

pub(super) fn linux_check_deb_source() -> &'static str {
    include_str!("../../../../install/linux/check-deb.sh")
}

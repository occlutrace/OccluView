// A product document that cannot be read must fail its contract check.
#![allow(clippy::panic)]

pub(super) use super::*;
use std::path::Path;

mod chrome;
mod documents;
mod platform;
mod source_tree;
mod viewport;

/// Every `.rs` file under `directory`, skipping symlinks and any `target`
/// directory so a local build tree cannot pollute a source-tree check.
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

/// Read a product document or platform descriptor from the repository root.
pub(super) fn repo_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("contract test input {} is missing: {error}", path.display())
    })
}

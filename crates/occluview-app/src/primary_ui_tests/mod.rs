pub(super) use super::*;
use std::path::Path;

mod chrome;
mod loading;
mod platform;
mod source_tree;
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

pub(super) fn app_module_source() -> &'static str {
    concat!(
        include_str!("../app/mod.rs"),
        "\n",
        include_str!("../app/state.rs")
    )
}

pub(super) fn app_bootstrap_source() -> &'static str {
    include_str!("../app_bootstrap.rs")
}

pub(super) fn app_loading_source() -> &'static str {
    include_str!("../app/app_loading.rs")
}

pub(super) fn app_dialogs_source() -> &'static str {
    include_str!("../app/app_dialogs.rs")
}

pub(super) fn app_render_source() -> &'static str {
    include_str!("../app/app_render.rs")
}

pub(super) fn app_chrome_source() -> &'static str {
    include_str!("../app_chrome.rs")
}

pub(super) fn app_layer_edits_source() -> String {
    [
        include_str!("../app/app_layer_edits/mod.rs"),
        include_str!("../app/app_layer_edits/whole_mesh.rs"),
        include_str!("../app/app_layer_edits/selection_ops.rs"),
        include_str!("../app/app_layer_edits/structural.rs"),
        include_str!("../app/app_layer_edits/undo_redo.rs"),
    ]
    .concat()
}

pub(super) fn app_viewport_source() -> &'static str {
    concat!(
        include_str!("../app/app_viewport.rs"),
        "\n",
        include_str!("../app/app_mesh_editor.rs"),
        "\n",
        include_str!("../app/app_cut_measure.rs"),
        "\n",
        include_str!("../app/app_layer_interaction.rs")
    )
}

pub(super) fn repo_source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(path).unwrap_or_default()
}

pub(super) fn viewer_interaction_source() -> &'static str {
    include_str!("../viewer/interaction.rs")
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

pub(super) fn linux_metainfo_source() -> &'static str {
    include_str!("../../../../install/linux/ai.occlutrace.OccluView.metainfo.xml")
}

pub(super) fn linux_desktop_source() -> &'static str {
    include_str!("../../../../install/linux/ai.occlutrace.OccluView.desktop")
}

pub(super) fn count_occurrences(source: &str, needle: &str) -> usize {
    source.match_indices(needle).count()
}

pub(super) fn function_source<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature);
    assert!(start.is_some(), "missing {signature}");
    let Some(start) = start else {
        return "";
    };
    let body = &source[start + signature.len()..];
    let next_fn = [
        "\n        fn ",
        "\n        pub(super) fn ",
        "\n    fn ",
        "\n    pub(super) fn ",
    ]
    .into_iter()
    .filter_map(|needle| body.find(needle))
    .min()
    .unwrap_or(body.len());
    &source[start..start + signature.len() + next_fn]
}

use super::{AppErrorDialog, Error, PathBuf};

pub(super) fn load_error_dialog(action: &str, error: &Error, paths: &[PathBuf]) -> AppErrorDialog {
    let title = if action == "Add" {
        "Could not add file"
    } else {
        "Could not open file"
    };
    let files = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    AppErrorDialog {
        title: title.to_string(),
        summary: format!("{action} failed: {error:#}"),
        details: format!("{action} failed\n\nFiles:\n{files}\n\nError:\n{error:#}"),
    }
}

use super::{AppErrorAction, AppErrorDialog, Error, PathBuf};

pub(super) fn load_error_dialog(
    locale: &crate::i18n::LocaleManager,
    action: &str,
    error: &Error,
    paths: &[PathBuf],
) -> AppErrorDialog {
    let title = if action == "Add" {
        locale.text("error-add-title")
    } else {
        locale.text("error-open-title")
    };
    let summary = if action == "Add" {
        locale.tr_with(
            "load-action-failed-add",
            &[("detail", &format!("{error:#}"))],
        )
    } else {
        locale.tr_with(
            "load-action-failed-open",
            &[("detail", &format!("{error:#}"))],
        )
    };
    let files = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    AppErrorDialog {
        title,
        summary,
        // Support payload stays verbatim (see `AppErrorDialog.details`).
        details: format!("{action} failed\n\nFiles:\n{files}\n\nError:\n{error:#}"),
        action: AppErrorAction::None,
    }
}

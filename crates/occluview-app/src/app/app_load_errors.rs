use super::{AppErrorAction, AppErrorDialog, Error, PathBuf};

/// The sentence for a file above the import limit, in the operator's language.
///
/// The format error's own text is English and prints raw byte counts, and it is
/// interpolated verbatim into a localized sentence, so a Russian operator reads
/// `file is 2147483648 bytes, larger than the 1073741824 byte limit`. The
/// numbers also belong in gigabytes: the operator's next step depends on how far
/// over the file is, not on its byte count.
#[allow(clippy::cast_precision_loss)]
fn too_large_summary(locale: &crate::i18n::LocaleManager, error: &Error) -> Option<String> {
    let occluview_formats::FormatError::TooLarge { bytes, limit } =
        error.downcast_ref::<occluview_formats::FormatError>()?
    else {
        return None;
    };
    // Tenths of a gigabyte: the operator needs to know how far over the file is,
    // not its byte count, and the limit itself is a whole number of gigabytes.
    let gib = (1_u64 << 30) as f64;
    Some(locale.tr_with(
        "load-file-too-large",
        &[
            ("size", &format!("{:.1}", *bytes as f64 / gib)),
            ("limit", &format!("{}", *limit >> 30)),
        ],
    ))
}

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
    if let Some(summary) = too_large_summary(locale, error) {
        return AppErrorDialog {
            title,
            summary,
            details: format!(
                "{action} failed\n\nFiles:\n{}\n\nError:\n{error:#}",
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            action: AppErrorAction::None,
        };
    }
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

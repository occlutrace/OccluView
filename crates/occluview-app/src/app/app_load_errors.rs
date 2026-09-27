use super::{AppErrorAction, AppErrorDialog, Error, PathBuf};

/// The sentence for a file above the import limit, in the operator's language.
///
/// The format error's own text is English and prints raw byte counts
/// (`file is 2147483648 bytes, larger than the 1073741824 byte limit`), so it
/// cannot be interpolated into a localized sentence. The numbers are stated in
/// gigabytes: the operator's next step depends on how far over the file is, not
/// on its byte count.
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
    let summary = load_failure_summary(locale, action, error);
    if is_special_load_error(error) {
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

pub(super) fn load_failure_summary(
    locale: &crate::i18n::LocaleManager,
    action: &str,
    error: &Error,
) -> String {
    if let Some(summary) = memory_budget_summary(locale, error) {
        return summary;
    }
    if let Some(summary) = too_large_summary(locale, error) {
        return summary;
    }
    if action == "Add" {
        locale.tr_with(
            "load-action-failed-add",
            &[("detail", &format!("{error:#}"))],
        )
    } else {
        locale.tr_with(
            "load-action-failed-open",
            &[("detail", &format!("{error:#}"))],
        )
    }
}

fn is_special_load_error(error: &Error) -> bool {
    error
        .downcast_ref::<occluview_formats::FormatError>()
        .is_some_and(|format_error| {
            matches!(
                format_error,
                occluview_formats::FormatError::TooLarge { .. }
                    | occluview_formats::FormatError::MemoryBudgetExceeded { .. }
            )
        })
}

#[allow(clippy::cast_precision_loss)]
fn memory_budget_summary(locale: &crate::i18n::LocaleManager, error: &Error) -> Option<String> {
    let occluview_formats::FormatError::MemoryBudgetExceeded {
        estimated_bytes,
        limit,
    } = error.downcast_ref::<occluview_formats::FormatError>()?
    else {
        return None;
    };
    let gib = (1_u64 << 30) as f64;
    let size_gib = (*estimated_bytes as f64 / gib * 10.0).ceil() / 10.0;
    Some(locale.tr_with(
        "load-memory-budget-exceeded",
        &[
            ("size", &format!("{size_gib:.1}")),
            ("limit", &format!("{:.1}", *limit as f64 / gib)),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_budget_failure_uses_the_localized_scene_limit_summary() {
        let error = Error::new(occluview_formats::FormatError::MemoryBudgetExceeded {
            estimated_bytes: 5_u64 << 29,
            limit: occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES,
        })
        .context("upper.stl: format reader failed");
        assert!(
            error
                .downcast_ref::<occluview_formats::FormatError>()
                .is_some(),
            "the localized load message must retain the typed format error"
        );
        let locale = crate::i18n::LocaleManager::for_tests();

        let summary = load_failure_summary(&locale, "Add", &error);
        assert!(
            summary.contains("2.5 GB"),
            "estimate missing from {summary}"
        );
        assert!(summary.contains("2.0 GB"), "limit missing from {summary}");
        assert!(
            summary.contains("Close layers"),
            "recovery advice missing from {summary}"
        );

        let just_over_limit = Error::new(occluview_formats::FormatError::MemoryBudgetExceeded {
            estimated_bytes: (2_u64 << 30) + 1,
            limit: occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES,
        });
        assert!(
            load_failure_summary(&locale, "Open", &just_over_limit).contains("2.1 GB"),
            "the rounded estimate must not read as the limit"
        );
    }
}

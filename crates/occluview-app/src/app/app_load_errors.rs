use super::{AppErrorAction, AppErrorDialog, Error, PathBuf};

const BYTES_PER_GIBIBYTE: f64 = 1_073_741_824.0;

#[allow(clippy::cast_precision_loss)]
fn gibibytes(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_GIBIBYTE
}

/// The sentence for a file above the import limit, in the operator's language.
///
/// The format error's own text is English and prints raw byte counts
/// (`file is 2147483648 bytes, larger than the 1073741824 byte limit`), so it
/// cannot be interpolated into a localized sentence. The numbers are stated in
/// gibibytes to match the binary file limit.
#[allow(clippy::cast_precision_loss)]
fn too_large_summary(locale: &crate::i18n::LocaleManager, error: &Error) -> Option<String> {
    let occluview_formats::FormatError::TooLarge { bytes, limit } =
        error.downcast_ref::<occluview_formats::FormatError>()?
    else {
        return None;
    };
    // Tenths of a gibibyte show how far the file exceeds the binary limit.
    Some(locale.tr_with(
        crate::i18n::message_id!("load-file-too-large"),
        &[
            ("size", &locale.number_format().decimal(gibibytes(*bytes), 1)),
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
        locale.text(crate::i18n::message_id!("error-add-title"))
    } else {
        locale.text(crate::i18n::message_id!("error-open-title"))
    };
    let summary = load_failure_summary(locale, action, error);
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
            crate::i18n::message_id!("load-action-failed-add"),
            &[("detail", &format!("{error:#}"))],
        )
    } else {
        locale.tr_with(
            crate::i18n::message_id!("load-action-failed-open"),
            &[("detail", &format!("{error:#}"))],
        )
    }
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
    let size_gib = (gibibytes(*estimated_bytes) * 10.0).ceil() / 10.0;
    Some(locale.tr_with(
        crate::i18n::message_id!("load-memory-budget-exceeded"),
        &[
            ("size", &locale.number_format().decimal(size_gib, 1)),
            ("limit", &locale.number_format().decimal(gibibytes(*limit), 1)),
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
        let summary_without_directional_marks: String = summary
            .chars()
            .filter(|character| !matches!(character, '\u{2068}' | '\u{2069}'))
            .collect();
        assert!(
            summary_without_directional_marks.contains("2.5 GiB"),
            "estimate missing from {summary}"
        );
        assert!(
            summary_without_directional_marks.contains("2.0 GiB"),
            "limit missing from {summary}"
        );
        assert!(
            summary.contains("Close layers"),
            "recovery advice missing from {summary}"
        );

        let just_over_limit = Error::new(occluview_formats::FormatError::MemoryBudgetExceeded {
            estimated_bytes: (2_u64 << 30) + 1,
            limit: occluview_formats::SCENE_IMPORT_MEMORY_BUDGET_BYTES,
        });
        let just_over_limit_summary = load_failure_summary(&locale, "Open", &just_over_limit);
        let just_over_limit_without_directional_marks: String = just_over_limit_summary
            .chars()
            .filter(|character| !matches!(character, '\u{2068}' | '\u{2069}'))
            .collect();
        assert!(
            just_over_limit_without_directional_marks.contains("2.1 GiB"),
            "the rounded estimate must not read as the limit"
        );
    }

    #[test]
    fn file_size_failure_labels_binary_bytes_as_gibibytes() {
        let error = Error::new(occluview_formats::FormatError::TooLarge {
            bytes: 3_u64 << 30,
            limit: 1_u64 << 30,
        });
        let locale = crate::i18n::LocaleManager::for_tests();

        let summary = load_failure_summary(&locale, "Open", &error);
        let summary_without_directional_marks: String = summary
            .chars()
            .filter(|character| !matches!(character, '\u{2068}' | '\u{2069}'))
            .collect();
        assert!(
            summary_without_directional_marks.contains("3.0 GiB"),
            "the measured file size is expressed in GiB: {summary}"
        );
        assert!(
            summary_without_directional_marks.contains("1 GiB"),
            "the file limit is expressed in GiB: {summary}"
        );
    }
}

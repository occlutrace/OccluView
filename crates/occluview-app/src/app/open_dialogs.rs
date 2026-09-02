//! Which modal dialogs are in front of the viewport.

/// Which modal dialogs are up this frame.
///
/// Written out by hand at each of five call sites, the set drifted to three,
/// four and six terms, which is how Escape came to tear down the tool behind
/// an open dialog. Named once, it is enumerable -- and testable without an
/// egui context, which the call sites are not.
// The independent guards remain bools because an error can arrive while the
// unsaved-changes prompt is up. Information surfaces are mutually exclusive,
// so their enum is already reduced to one bool at this boundary.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub(super) struct OpenDialogs {
    pub(super) close_guard: bool,
    pub(super) pending_replace: bool,
    pub(super) error: bool,
    pub(super) settings_popup: bool,
    pub(super) information_dialog: bool,
}

impl OpenDialogs {
    /// True when anything modal is in front of the viewport.
    pub(super) fn any(self) -> bool {
        self.close_guard
            || self.pending_replace
            || self.error
            || self.settings_popup
            || self.information_dialog
    }

    /// Every flag, paired with the name a failing test should print.
    #[cfg(test)]
    fn terms(self) -> [(&'static str, bool); 5] {
        [
            ("close_guard", self.close_guard),
            ("pending_replace", self.pending_replace),
            ("error", self.error),
            ("settings_popup", self.settings_popup),
            ("information_dialog", self.information_dialog),
        ]
    }

    #[cfg(test)]
    fn none_open() -> Self {
        Self {
            close_guard: false,
            pending_replace: false,
            error: false,
            settings_popup: false,
            information_dialog: false,
        }
    }

    #[cfg(test)]
    fn with_term(index: usize) -> Self {
        let mut dialogs = Self::none_open();
        match index {
            0 => dialogs.close_guard = true,
            1 => dialogs.pending_replace = true,
            2 => dialogs.error = true,
            3 => dialogs.settings_popup = true,
            _ => dialogs.information_dialog = true,
        }
        dialogs
    }
}

#[cfg(test)]
mod open_dialogs_tests {
    use super::OpenDialogs;

    #[test]
    fn every_dialog_on_its_own_holds_escape_back() {
        // Dropping any single term is the failure this type exists to prevent:
        // with that dialog up and no other, Escape reached the tool behind it,
        // and for Align Scans that put every scan back where it started.
        for index in 0..OpenDialogs::none_open().terms().len() {
            let dialogs = OpenDialogs::with_term(index);
            let (name, _) = dialogs.terms()[index];
            assert!(
                dialogs.any(),
                "{name} alone must still count as a dialog in front"
            );
        }
    }

    #[test]
    fn nothing_open_lets_escape_through_to_the_tool() {
        // The other half: a predicate that always says "a dialog is open"
        // would pass the test above and make Escape useless everywhere.
        assert!(!OpenDialogs::none_open().any());
    }
}

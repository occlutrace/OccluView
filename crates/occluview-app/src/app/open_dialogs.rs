//! Which modal dialogs are in front of the viewport.

/// Which modal dialogs are up this frame.
///
/// The five terms used to be written out by hand at each of five call sites
/// and had drifted to three, four and six of them, which is how Escape came to
/// tear down the tool behind an open dialog. Naming them once makes the set
/// enumerable -- and testable without an egui context, which the call sites
/// are not.
// Five independent bools rather than a state enum because the dialogs are
// independent: an error can arrive while the unsaved-changes prompt is up,
// and the licences window can sit over either. Collapsing them would lose
// exactly the combinations Escape has to survive.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub(super) struct OpenDialogs {
    pub(super) close_guard: bool,
    pub(super) pending_replace: bool,
    pub(super) error: bool,
    pub(super) about: bool,
    pub(super) third_party: bool,
}

impl OpenDialogs {
    /// True when anything modal is in front of the viewport.
    pub(super) fn any(self) -> bool {
        self.close_guard || self.pending_replace || self.error || self.about || self.third_party
    }

    /// Every flag, paired with the name a failing test should print.
    #[cfg(test)]
    fn terms(self) -> [(&'static str, bool); 5] {
        [
            ("close_guard", self.close_guard),
            ("pending_replace", self.pending_replace),
            ("error", self.error),
            ("about", self.about),
            ("third_party", self.third_party),
        ]
    }

    #[cfg(test)]
    fn none_open() -> Self {
        Self {
            close_guard: false,
            pending_replace: false,
            error: false,
            about: false,
            third_party: false,
        }
    }

    #[cfg(test)]
    fn with_term(index: usize) -> Self {
        let mut dialogs = Self::none_open();
        match index {
            0 => dialogs.close_guard = true,
            1 => dialogs.pending_replace = true,
            2 => dialogs.error = true,
            3 => dialogs.about = true,
            _ => dialogs.third_party = true,
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

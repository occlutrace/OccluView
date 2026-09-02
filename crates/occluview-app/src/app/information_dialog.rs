#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum InformationDialog {
    #[default]
    None,
    About,
    ThirdPartyNotices,
}

impl InformationDialog {
    pub(super) const fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[cfg(test)]
mod tests {
    use super::InformationDialog;

    #[test]
    fn information_dialog_has_exactly_one_active_surface() {
        assert!(!InformationDialog::None.is_open());
        assert!(InformationDialog::About.is_open());
        assert!(InformationDialog::ThirdPartyNotices.is_open());
    }
}

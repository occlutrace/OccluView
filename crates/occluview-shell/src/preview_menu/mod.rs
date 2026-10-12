//! Essential commands for the embedded attachment / Explorer preview.

pub(crate) mod icons;
use icons::PreviewMenuIcon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewMenuCommand {
    Open,
    FitView,
    ToggleWireframe,
}

impl PreviewMenuCommand {
    // Retired ids (Edit, presets and Copy image) are never reused.
    pub(crate) const fn id(self) -> u32 {
        match self {
            Self::Open => 1,
            Self::FitView => 20,
            Self::ToggleWireframe => 30,
        }
    }
    pub(crate) fn from_id(id: u32) -> Option<Self> {
        MENU_COMMANDS
            .iter()
            .copied()
            .find(|command| command.id() == id)
    }
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            Self::Open => "shell-preview-open",
            Self::FitView => "shell-preview-fit-view",
            Self::ToggleWireframe => "shell-preview-wireframe",
        }
    }
    pub(crate) const fn icon(self) -> PreviewMenuIcon {
        match self {
            Self::Open => PreviewMenuIcon::Open,
            Self::FitView => PreviewMenuIcon::FitFrame,
            Self::ToggleWireframe => PreviewMenuIcon::Wireframe,
        }
    }
    pub(crate) const fn is_default(self) -> bool {
        matches!(self, Self::Open)
    }
    pub(crate) const fn is_checkable(self) -> bool {
        matches!(self, Self::ToggleWireframe)
    }
}

pub(crate) const MENU_COMMANDS: [PreviewMenuCommand; 3] = [
    PreviewMenuCommand::Open,
    PreviewMenuCommand::FitView,
    PreviewMenuCommand::ToggleWireframe,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewMenuEntry {
    Separator,
    Command(PreviewMenuCommand),
}

pub(crate) const PREVIEW_MENU_LAYOUT: &[PreviewMenuEntry] = &[
    PreviewMenuEntry::Command(PreviewMenuCommand::Open),
    PreviewMenuEntry::Separator,
    PreviewMenuEntry::Command(PreviewMenuCommand::FitView),
    PreviewMenuEntry::Command(PreviewMenuCommand::ToggleWireframe),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_actions_cannot_be_dispatched() {
        for id in [0, 2, 10, 11, 12, 13, 40] {
            assert_eq!(PreviewMenuCommand::from_id(id), None);
        }
        for command in MENU_COMMANDS {
            assert_eq!(PreviewMenuCommand::from_id(command.id()), Some(command));
        }
    }
}

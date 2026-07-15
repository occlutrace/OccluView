//! Explorer preview-pane context menu: the platform-agnostic model.
//!
//! This describes *what* the right-click menu contains — the commands, their
//! stable ids, labels, icons, and layout — with no Win32 dependency, so the
//! whole inventory is unit tested on any host. The Windows layer
//! (`com::preview::context_menu`) turns this model into a real `HMENU`, tracks
//! it, and dispatches the selected command.

pub(crate) mod dib;
pub(crate) mod icons;

use crate::preview_scene::PreviewViewPreset;
use icons::PreviewMenuIcon;

/// A command a user can pick from the preview context menu.
///
/// Numeric ids are **stable across releases**: `TrackPopupMenuEx` returns the
/// selected item's `wID`, and a future keyboard/automation path may key off it.
/// Never `0` — `TrackPopupMenuEx` returns `0` for "nothing selected".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewMenuCommand {
    /// Open the file in the OccluView desktop app (double-click parity).
    Open,
    /// Open the file in the OccluView desktop app for editing.
    Edit,
    /// Reorient the preview camera to the front view.
    ViewFront,
    /// Reorient the preview camera to the top view.
    ViewTop,
    /// Reorient the preview camera to the side view.
    ViewSide,
    /// Reorient the preview camera to an isometric view.
    ViewIsometric,
    /// Recenter and refit the current view onto the mesh.
    FitView,
    /// Toggle the technical wireframe overlay.
    ToggleWireframe,
    /// Copy the current preview frame to the clipboard as a bitmap.
    CopyImage,
}

impl PreviewMenuCommand {
    /// Stable, nonzero menu command id.
    pub(crate) const fn id(self) -> u32 {
        match self {
            Self::Open => 1,
            Self::Edit => 2,
            Self::ViewFront => 10,
            Self::ViewTop => 11,
            Self::ViewSide => 12,
            Self::ViewIsometric => 13,
            Self::FitView => 20,
            Self::ToggleWireframe => 30,
            Self::CopyImage => 40,
        }
    }

    /// Resolve a `TrackPopupMenuEx` return value back to a command.
    pub(crate) fn from_id(id: u32) -> Option<Self> {
        MENU_COMMANDS.iter().copied().find(|cmd| cmd.id() == id)
    }

    /// The menu item label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Open => "Open in OccluView",
            Self::Edit => "Edit in OccluView",
            Self::ViewFront => "Front",
            Self::ViewTop => "Top",
            Self::ViewSide => "Side",
            Self::ViewIsometric => "Isometric",
            Self::FitView => "Fit view",
            Self::ToggleWireframe => "Wireframe",
            Self::CopyImage => "Copy image",
        }
    }

    /// The icon drawn next to this item.
    pub(crate) const fn icon(self) -> PreviewMenuIcon {
        match self {
            Self::Open => PreviewMenuIcon::Open,
            Self::Edit => PreviewMenuIcon::Edit,
            Self::ViewFront => PreviewMenuIcon::CubeFront,
            Self::ViewTop => PreviewMenuIcon::CubeTop,
            Self::ViewSide => PreviewMenuIcon::CubeSide,
            Self::ViewIsometric => PreviewMenuIcon::CubeIsometric,
            Self::FitView => PreviewMenuIcon::FitFrame,
            Self::ToggleWireframe => PreviewMenuIcon::Wireframe,
            Self::CopyImage => PreviewMenuIcon::CopyImage,
        }
    }

    /// Whether this is the default (bold) item, activated by Enter / plain
    /// click. Only `Open` — the obvious double-click-equivalent action.
    pub(crate) const fn is_default(self) -> bool {
        matches!(self, Self::Open)
    }

    /// Whether this item carries a check mark reflecting live state.
    pub(crate) const fn is_checkable(self) -> bool {
        matches!(self, Self::ToggleWireframe)
    }

    /// The camera preset this command applies, if it is a view command.
    pub(crate) fn view_preset(self) -> Option<PreviewViewPreset> {
        match self {
            Self::ViewFront => Some(PreviewViewPreset::Front),
            Self::ViewTop => Some(PreviewViewPreset::Top),
            Self::ViewSide => Some(PreviewViewPreset::Side),
            Self::ViewIsometric => Some(PreviewViewPreset::Isometric),
            _ => None,
        }
    }
}

/// Every command, in id order. Also the lookup table for [`PreviewMenuCommand::from_id`].
pub(crate) const MENU_COMMANDS: [PreviewMenuCommand; 9] = [
    PreviewMenuCommand::Open,
    PreviewMenuCommand::Edit,
    PreviewMenuCommand::ViewFront,
    PreviewMenuCommand::ViewTop,
    PreviewMenuCommand::ViewSide,
    PreviewMenuCommand::ViewIsometric,
    PreviewMenuCommand::FitView,
    PreviewMenuCommand::ToggleWireframe,
    PreviewMenuCommand::CopyImage,
];

/// One row of the popup menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewMenuEntry {
    /// A horizontal separator line.
    Separator,
    /// A selectable command.
    Command(PreviewMenuCommand),
}

/// The menu, top to bottom. Grouped: launch actions, view presets + fit,
/// wireframe toggle, then copy image.
pub(crate) const PREVIEW_MENU_LAYOUT: &[PreviewMenuEntry] = &[
    PreviewMenuEntry::Command(PreviewMenuCommand::Open),
    PreviewMenuEntry::Command(PreviewMenuCommand::Edit),
    PreviewMenuEntry::Separator,
    PreviewMenuEntry::Command(PreviewMenuCommand::ViewFront),
    PreviewMenuEntry::Command(PreviewMenuCommand::ViewTop),
    PreviewMenuEntry::Command(PreviewMenuCommand::ViewSide),
    PreviewMenuEntry::Command(PreviewMenuCommand::ViewIsometric),
    PreviewMenuEntry::Command(PreviewMenuCommand::FitView),
    PreviewMenuEntry::Separator,
    PreviewMenuEntry::Command(PreviewMenuCommand::ToggleWireframe),
    PreviewMenuEntry::Separator,
    PreviewMenuEntry::Command(PreviewMenuCommand::CopyImage),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique_and_nonzero() {
        let mut ids: Vec<u32> = MENU_COMMANDS.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MENU_COMMANDS.len(), "command ids must be unique");
        assert!(
            MENU_COMMANDS.iter().all(|c| c.id() != 0),
            "0 is reserved for 'no selection'"
        );
    }

    #[test]
    fn from_id_round_trips_every_command() {
        for cmd in MENU_COMMANDS {
            assert_eq!(PreviewMenuCommand::from_id(cmd.id()), Some(cmd));
        }
        assert_eq!(PreviewMenuCommand::from_id(0), None);
        assert_eq!(PreviewMenuCommand::from_id(9999), None);
    }

    #[test]
    fn layout_matches_the_intended_inventory() {
        let commands: Vec<PreviewMenuCommand> = PREVIEW_MENU_LAYOUT
            .iter()
            .filter_map(|entry| match entry {
                PreviewMenuEntry::Command(cmd) => Some(*cmd),
                PreviewMenuEntry::Separator => None,
            })
            .collect();
        assert_eq!(
            commands,
            vec![
                PreviewMenuCommand::Open,
                PreviewMenuCommand::Edit,
                PreviewMenuCommand::ViewFront,
                PreviewMenuCommand::ViewTop,
                PreviewMenuCommand::ViewSide,
                PreviewMenuCommand::ViewIsometric,
                PreviewMenuCommand::FitView,
                PreviewMenuCommand::ToggleWireframe,
                PreviewMenuCommand::CopyImage,
            ]
        );
        // Exactly three separators partition the four groups.
        let separators = PREVIEW_MENU_LAYOUT
            .iter()
            .filter(|e| matches!(e, PreviewMenuEntry::Separator))
            .count();
        assert_eq!(separators, 3, "menu should have three group separators");
    }

    #[test]
    fn exactly_one_default_and_one_checkable_item() {
        assert_eq!(
            MENU_COMMANDS.iter().filter(|c| c.is_default()).count(),
            1,
            "only Open is the default item"
        );
        assert!(PreviewMenuCommand::Open.is_default());
        assert_eq!(
            MENU_COMMANDS.iter().filter(|c| c.is_checkable()).count(),
            1,
            "only Wireframe is checkable"
        );
        assert!(PreviewMenuCommand::ToggleWireframe.is_checkable());
    }

    #[test]
    fn view_commands_map_to_presets_and_others_do_not() {
        assert_eq!(
            PreviewMenuCommand::ViewFront.view_preset(),
            Some(PreviewViewPreset::Front)
        );
        assert_eq!(
            PreviewMenuCommand::ViewTop.view_preset(),
            Some(PreviewViewPreset::Top)
        );
        assert_eq!(
            PreviewMenuCommand::ViewSide.view_preset(),
            Some(PreviewViewPreset::Side)
        );
        assert_eq!(
            PreviewMenuCommand::ViewIsometric.view_preset(),
            Some(PreviewViewPreset::Isometric)
        );
        assert_eq!(PreviewMenuCommand::Open.view_preset(), None);
        assert_eq!(PreviewMenuCommand::CopyImage.view_preset(), None);
    }

    #[test]
    fn every_command_has_a_distinct_label_and_icon() {
        let labels: Vec<&str> = MENU_COMMANDS.iter().map(|c| c.label()).collect();
        for (i, a) in labels.iter().enumerate() {
            for b in &labels[i + 1..] {
                assert_ne!(a, b, "labels must be distinct");
            }
        }
        // Every command has an icon; the four cube variants intentionally share
        // a family but are distinct enum values.
        let icons: Vec<PreviewMenuIcon> = MENU_COMMANDS.iter().map(|c| c.icon()).collect();
        assert_eq!(icons.len(), MENU_COMMANDS.len());
    }
}

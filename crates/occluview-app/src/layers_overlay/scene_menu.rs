//! The scene context menu: what a right-click on empty viewport space offers.
//!
//! Deliberately short. Right-clicking a mesh already opens the layer menu, so
//! this one only carries actions that belong to the whole scene — above all
//! saving it, which is the only way an alignment survives the session. The
//! viewer has no project file.

use crate::icons::AppIcon;
use crate::ui_theme;
use eframe::egui;

use super::menu::menu_item;

/// Fixed menu width, matching the layer menu so the two read as one family.
const MENU_WIDTH: f32 = 244.0;

/// What a right-click on empty space can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneContextAction {
    /// Write every visible layer, in its current pose, as one file.
    SaveScene,
    /// Write every visible layer to its own file in a chosen folder.
    SaveEachLayer,
    /// Return every layer to the identity pose.
    ResetPositions,
    /// Frame the whole scene.
    FitView,
}

/// Render the scene context menu into `ui`.
pub(crate) fn show_scene_context_menu(
    ui: &mut egui::Ui,
    has_layers: bool,
    any_moved: bool,
    request: &mut Option<SceneContextAction>,
) {
    ui.set_min_width(MENU_WIDTH);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    ui.spacing_mut().item_spacing.y = 2.0;

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Scene")
            .color(ui_theme::TEXT_WEAK)
            .size(11.0),
    );
    ui.add_space(2.0);
    ui.separator();

    let entries = [
        (
            AppIcon::Export,
            "Save scene as…",
            has_layers,
            SceneContextAction::SaveScene,
        ),
        (
            AppIcon::Export,
            "Save each layer…",
            has_layers,
            SceneContextAction::SaveEachLayer,
        ),
        (
            AppIcon::FlipNormals,
            "Reset positions",
            any_moved,
            SceneContextAction::ResetPositions,
        ),
        (
            AppIcon::FitView,
            "Fit view",
            has_layers,
            SceneContextAction::FitView,
        ),
    ];

    for (position, (icon, label, enabled, action)) in entries.into_iter().enumerate() {
        // The saving pair and the view pair are different kinds of action.
        if position == 2 {
            ui.separator();
        }
        if menu_item(ui, icon, label, enabled).clicked() {
            *request = Some(action);
            ui.close_menu();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::SceneContextAction;

    /// The scene menu exists to make an alignment survivable: without a save
    /// entry the operator has no way to keep a moved scan, because the viewer
    /// has no project file.
    #[test]
    fn the_scene_menu_offers_saving_before_anything_else() {
        let source = crate::primary_ui_tests::production_source(include_str!("scene_menu.rs"));
        let save = source.find("Save scene as…").expect("a save entry");
        let reset = source.find("Reset positions").expect("a reset entry");
        assert!(save < reset, "saving must come first in the menu");
    }

    #[test]
    fn every_scene_action_is_reachable_from_the_menu() {
        let source = crate::primary_ui_tests::production_source(include_str!("scene_menu.rs"));
        for action in [
            SceneContextAction::SaveScene,
            SceneContextAction::SaveEachLayer,
            SceneContextAction::ResetPositions,
            SceneContextAction::FitView,
        ] {
            let name = format!("SceneContextAction::{action:?}");
            assert!(
                source.contains(name.as_str()),
                "{name} is declared but never offered"
            );
        }
    }
}

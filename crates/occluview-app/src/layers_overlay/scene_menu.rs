//! The scene context menu: what a right-click on empty viewport space offers.
//!
//! Kept short. Right-clicking a mesh already opens the layer menu, so
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
    locale: &crate::i18n::LocaleManager,
) {
    ui.set_min_width(MENU_WIDTH);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    ui.spacing_mut().item_spacing.y = 2.0;

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(locale.tr("scene-menu-title"))
            .color(ui_theme::text_weak())
            .size(11.0),
    );
    ui.add_space(2.0);
    ui.separator();

    // Each entry keeps its English wording beside the catalog key it renders.
    let entries = [
        (
            AppIcon::Export,
            "Save scene as…",
            "scene-menu-save",
            has_layers,
            SceneContextAction::SaveScene,
        ),
        (
            AppIcon::Export,
            "Save each layer…",
            "scene-menu-save-each",
            has_layers,
            SceneContextAction::SaveEachLayer,
        ),
        (
            AppIcon::FlipNormals,
            "Reset positions",
            "scene-menu-reset",
            any_moved,
            SceneContextAction::ResetPositions,
        ),
        (
            AppIcon::FitView,
            "Fit view",
            "scene-menu-fit",
            has_layers,
            SceneContextAction::FitView,
        ),
    ];

    for (position, (icon, _label, key, enabled, action)) in entries.into_iter().enumerate() {
        // The saving pair and the view pair are different kinds of action.
        if position == 2 {
            ui.separator();
        }
        if menu_item(ui, icon, &locale.tr(key), enabled).clicked() {
            *request = Some(action);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
}

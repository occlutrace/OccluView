use crate::icons::AppIcon;
use crate::layer_actions::{LayerContextAction, LayerContextRequest};
use crate::mesh_editor_icons::CELL_ROUNDING;
use crate::ui_theme;
use eframe::egui;
use occluview_core::SceneMeshId;

use super::{LayerOverlaySceneAction, LayerSceneTabs};

/// Fixed context-menu width. It gives the elided file-name title and operator
/// labels room to breathe without turning the menu into a second panel.
const MENU_WIDTH: f32 = 244.0;

/// Everything the layer context menu needs about one layer. Shared by the
/// layers-overlay rows and the viewport right-click menu so both surface the
/// same action set through the same plumbing.
// Five independent display/state flags, not a state machine — see SceneMesh.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub(crate) struct LayerContextMenuTarget {
    /// Display label (file/mesh name) shown as the menu title so the operator
    /// knows which piece the click landed on.
    pub(crate) label: String,
    pub(crate) index: usize,
    pub(crate) layer_id: SceneMeshId,
    pub(crate) visible: bool,
    pub(crate) wireframe: bool,
    pub(crate) face_editable: bool,
    /// Whether the layer has payload that can be written. Point clouds are
    /// exportable as PLY/OBJ even though they are not face-editable.
    pub(crate) can_export: bool,
    /// Whether this layer's scan colors/texture are currently shown (vs the
    /// flat neutral material).
    pub(crate) show_vertex_colors: bool,
    /// Whether an attached texture is currently sampled.
    pub(crate) show_texture: bool,
    /// Whether this layer currently wears occlusal-contact marks.
    pub(crate) contacts: bool,
    /// Whether a contact reading can be opened on this layer at all: it needs a
    /// visible triangle surface and a second visible surface to measure
    /// against. A reading that needs two surfaces must not be an option that
    /// can only explain why it did nothing.
    pub(crate) can_read_contacts: bool,
    /// Whether the layer actually carries vertex colors or a texture — the
    /// toggle is a no-op (and stays disabled) on a plain uncolored scan.
    pub(crate) has_color_data: bool,
    pub(crate) has_texture: bool,
}

/// Attach the layer context menu to a widget response (row controls / row body).
#[allow(clippy::too_many_arguments)]
pub(super) fn attach_layer_context_menu(
    response: egui::Response,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    scene_tabs: Option<&LayerSceneTabs<'_>>,
    scene_action: &mut Option<LayerOverlaySceneAction>,
    locale: &crate::i18n::LocaleManager,
) {
    response.context_menu(|ui| {
        show_layer_context_menu(ui, target, context_request, locale);
        if let Some(scene_tabs) = scene_tabs {
            show_layer_transfer_action(ui, target.layer_id, scene_tabs, scene_action, locale);
        }
    });
}

fn show_layer_transfer_action(
    ui: &mut egui::Ui,
    layer_id: SceneMeshId,
    scene_tabs: &LayerSceneTabs<'_>,
    scene_action: &mut Option<LayerOverlaySceneAction>,
    locale: &crate::i18n::LocaleManager,
) {
    let other_scene = scene_tabs
        .scenes
        .iter()
        .find(|scene| scene.id != scene_tabs.active_scene_id);
    let Some(destination) = other_scene else {
        if !scene_tabs.can_create {
            return;
        }
        ui.separator();
        let label = locale.tr(crate::i18n::message_id!("workspace-layer-move-new-scene"));
        if menu_item(ui, AppIcon::MoveLayer, &label, true).clicked() {
            *scene_action = Some(LayerOverlaySceneAction::TransferToNew { layer_id });
            ui.close();
        }
        return;
    };

    ui.separator();
    let label = locale.tr_with(
        crate::i18n::message_id!("workspace-layer-move-scene"),
        &[("scene", destination.name)],
    );
    if menu_item(ui, AppIcon::MoveLayer, &label, true).clicked() {
        *scene_action = Some(LayerOverlaySceneAction::Transfer {
            layer_id,
            scene_id: destination.id,
        });
        ui.close();
    }
}

/// Render the layer context menu into `ui`. Used by both the row-attached menu
/// and the viewport right-click menu.
pub(crate) fn show_layer_context_menu(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) {
    // Pin the menu width: without this, egui lays the menu out inside
    // whatever sliver of screen is left of the click point, wrapping every
    // label into a letters-tall column at the viewport edge.
    ui.set_min_width(MENU_WIDTH);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    ui.spacing_mut().item_spacing.y = 2.0;
    // Which piece did the click land on? The file name, middle-elided so a long
    // name still fits (and keeps its extension). Essential once Separate/Cut
    // spawn several coincident parts.
    menu_title(ui, &target.label);
    ui.separator();
    show_material_actions(ui, target, context_request, locale);
    ui.separator();
    show_mesh_edit_actions(ui, target, context_request, locale);
    ui.separator();
    show_contact_actions(ui, target, context_request, locale);
    ui.separator();
    show_layer_actions(ui, target, context_request, locale);
}

fn show_material_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) {
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Palette,
            crate::i18n::message_id!("layer-menu-next-tint"),
            true,
            LayerContextAction::NextTint,
        ),
        context_request,
        locale,
    );
    let (colors_key, colors_icon) = if target.show_vertex_colors {
        (
            crate::i18n::message_id!("layer-menu-hide-colors"),
            AppIcon::ScanColors,
        )
    } else {
        (
            crate::i18n::message_id!("layer-menu-show-colors"),
            AppIcon::ScanColorsOff,
        )
    };
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            colors_icon,
            colors_key,
            target.has_color_data,
            LayerContextAction::ToggleShowVertexColors,
        )
        .selected(target.show_vertex_colors),
        context_request,
        locale,
    );
    if target.has_texture {
        let (texture_key, texture_icon) = if target.show_texture {
            (
                crate::i18n::message_id!("layer-menu-disable-texture"),
                AppIcon::Texture,
            )
        } else {
            (
                crate::i18n::message_id!("layer-menu-show-texture"),
                AppIcon::TextureOff,
            )
        };
        layer_menu_button(
            ui,
            target,
            LayerMenuButton::new(
                texture_icon,
                texture_key,
                true,
                LayerContextAction::ToggleShowTexture,
            )
            .selected(target.show_texture),
            context_request,
            locale,
        );
    }
}

fn show_mesh_edit_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) {
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::EditMesh,
            crate::i18n::message_id!("layer-menu-mesh-editing"),
            target.face_editable,
            LayerContextAction::EditMesh,
        ),
        context_request,
        locale,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::BridgeSplit,
            crate::i18n::message_id!("layer-menu-split-bridge"),
            target.visible && target.face_editable,
            LayerContextAction::BridgeSplit,
        ),
        context_request,
        locale,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Repair,
            crate::i18n::message_id!("layer-menu-repair"),
            target.face_editable,
            LayerContextAction::RepairMesh,
        ),
        context_request,
        locale,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::FlipNormals,
            crate::i18n::message_id!("layer-menu-flip-normals"),
            target.face_editable,
            LayerContextAction::InvertNormals,
        ),
        context_request,
        locale,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Export,
            crate::i18n::message_id!("layer-menu-export"),
            target.can_export,
            LayerContextAction::ExportLayer,
        ),
        context_request,
        locale,
    );
}

/// The occlusal contact reading, offered before the destructive entries so a
/// measurement sits with the other read-only actions rather than next to
/// Remove.
fn show_contact_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) {
    if target.contacts {
        layer_menu_button(
            ui,
            target,
            LayerMenuButton::new(
                AppIcon::Contacts,
                crate::i18n::message_id!("layer-menu-hide-contacts"),
                true,
                LayerContextAction::HideContacts,
            )
            .selected(true),
            context_request,
            locale,
        );
        return;
    }
    // The entry is offered but disabled rather than hidden: an operator who
    // right-clicks a lone scan is asking whether contacts exist here at all, and
    // a greyed line that explains itself answers that better than a menu that
    // drops the entry. `menu_item` paints no tooltip, so the reason goes
    // on the row response.
    let response = layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Contacts,
            crate::i18n::message_id!("layer-menu-contacts"),
            target.can_read_contacts,
            LayerContextAction::Contacts,
        ),
        context_request,
        locale,
    );
    if !target.can_read_contacts {
        response
            .on_hover_text(locale.tr(crate::i18n::message_id!("layer-menu-contacts-unavailable")));
    }
}

fn show_layer_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) {
    let wireframe_key = if target.wireframe {
        crate::i18n::message_id!("layer-menu-hide-wireframe")
    } else {
        crate::i18n::message_id!("layer-menu-show-wireframe")
    };
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Wireframe,
            wireframe_key,
            true,
            LayerContextAction::ToggleWireframe,
        )
        .selected(target.wireframe),
        context_request,
        locale,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            AppIcon::Trash,
            crate::i18n::message_id!("layer-menu-remove"),
            true,
            LayerContextAction::Remove,
        ),
        context_request,
        locale,
    );
}

struct LayerMenuButton {
    icon: AppIcon,
    /// Catalog key rendering the localized label.
    key: crate::i18n::MessageId,
    enabled: bool,
    action: LayerContextAction,
    selected: Option<bool>,
}

impl LayerMenuButton {
    const fn new(
        icon: AppIcon,
        key: crate::i18n::MessageId,
        enabled: bool,
        action: LayerContextAction,
    ) -> Self {
        Self {
            icon,
            key,
            enabled,
            action,
            selected: None,
        }
    }

    fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
}

fn layer_menu_button(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    button: LayerMenuButton,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) -> egui::Response {
    let response = menu_item_state(
        ui,
        button.icon,
        &locale.tr(button.key),
        button.enabled,
        button.selected,
    );
    if response.clicked() {
        *context_request = Some(LayerContextRequest {
            index: target.index,
            layer_id: target.layer_id,
            action: button.action,
        });
        ui.close();
    }
    response
}

/// One custom-rendered context-menu row: a vector glyph in a fixed left gutter,
/// then the label, over a rounded hover wash that matches the editor cells.
/// Fully painted (not an `egui::Button`) so every item aligns on the same gutter
/// and carries an icon. Returns the row `Response`.
pub(super) fn menu_item(
    ui: &mut egui::Ui,
    icon: AppIcon,
    label: &str,
    enabled: bool,
) -> egui::Response {
    menu_item_state(ui, icon, label, enabled, None)
}

fn menu_item_state(
    ui: &mut egui::Ui,
    icon: AppIcon,
    label: &str,
    enabled: bool,
    selected: Option<bool>,
) -> egui::Response {
    const ROW_H: f32 = 22.0;
    const PAD_L: f32 = 6.0;
    const GUTTER: f32 = 18.0;
    const ICON: f32 = 15.0;
    const LABEL_GAP: f32 = 8.0;

    let width = ui.available_width();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW_H), sense);

    let hovered = enabled && response.hovered();
    let fg = if !enabled {
        ui.visuals().weak_text_color()
    } else if hovered {
        ui_theme::accent()
    } else {
        ui_theme::text()
    };

    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CELL_ROUNDING, ui_theme::accent().gamma_multiply(0.12));
    }
    let icon_center = egui::pos2(rect.left() + PAD_L + GUTTER * 0.5, rect.center().y);
    let icon_rect = egui::Rect::from_center_size(icon_center, egui::vec2(ICON, ICON));
    crate::icons::paint(painter, icon_rect, icon, fg);
    painter.text(
        egui::pos2(rect.left() + PAD_L + GUTTER + LABEL_GAP, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        fg,
    );
    crate::accessibility::button(&response, label, enabled, selected);
    response
}

/// Draw the menu title: the file name of the clicked layer, middle-elided so a
/// long name fits the menu width while keeping its extension visible.
fn menu_title(ui: &mut egui::Ui, label: &str) {
    let name = menu_title_name(label);
    let font = egui::FontId::proportional(10.5);
    let budget = ui.available_width();
    // Context is a cheap Arc; cloning it lets the measure closure avoid
    // borrowing `ui` while we lay out candidate strings.
    let ctx = ui.ctx().clone();
    let measure = {
        let font = font.clone();
        move |text: &str| {
            ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            })
        }
    };
    let title = elide_middle(name, budget, measure);
    ui.label(egui::RichText::new(title).weak().size(10.5));
}

/// Reduce a label that may be a path to just its file name; leave a plain mesh
/// name untouched. The menu title should name the file the operator clicked,
/// not its whole directory chain.
fn menu_title_name(label: &str) -> &str {
    label
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(label)
}

/// Middle-elide `text` to fit `max_width` (measured by `measure`, in px),
/// keeping a leading prefix and a trailing suffix — so a file extension stays
/// visible — joined by a single '…'. Returns `text` unchanged when it already
/// fits. egui has no middle-ellipsis, so this binary-searches the largest number
/// of original characters that still fit, biasing the extra kept character to
/// the tail so the extension survives.
fn elide_middle(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    const ELLIPSIS: char = '…';
    if measure(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n <= 2 {
        // Nothing meaningful to elide out of the middle.
        return text.to_string();
    }
    let build = |keep: usize| -> String {
        let front = keep / 2;
        let back = keep - front;
        let head: String = chars[..front].iter().collect();
        let tail: String = chars[n - back..].iter().collect();
        format!("{head}{ELLIPSIS}{tail}")
    };
    // Largest count of original chars we can keep (0..=n-1) and still fit. Width
    // grows monotonically with `keep`, so a binary search is exact.
    let mut lo = 0usize;
    let mut hi = n - 1;
    let mut best = 0usize;
    while lo <= hi {
        let mid = lo.midpoint(hi);
        if measure(&build(mid)) <= max_width {
            best = mid;
            lo = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    build(best)
}

#[cfg(test)]
mod tests {
    // The elision tests use a fake monospace measure (chars × px) and assert on a
    // literal file extension; both are intended in test scaffolding.
    #![allow(
        clippy::cast_precision_loss,
        clippy::case_sensitive_file_extension_comparisons
    )]

    use super::{elide_middle, menu_title_name};

    #[test]
    fn menu_title_name_strips_a_path_to_its_file_name() {
        assert_eq!(
            menu_title_name(r"C:\cases\lower_scan.stl"),
            "lower_scan.stl"
        );
        assert_eq!(menu_title_name("/home/clinic/upper.ply"), "upper.ply");
        // A plain mesh name (no separators) is left untouched.
        assert_eq!(menu_title_name("Upper arch"), "Upper arch");
        // A trailing separator falls back to the whole label, not an empty title.
        assert_eq!(menu_title_name("weird/"), "weird/");
    }

    #[test]
    fn elide_middle_leaves_a_short_name_untouched() {
        // Fake monospace measure: 7 px per character.
        let measure = |s: &str| s.chars().count() as f32 * 7.0;
        assert_eq!(elide_middle("lower.stl", 400.0, measure), "lower.stl");
    }

    #[test]
    fn elide_middle_keeps_prefix_and_extension_for_a_long_name() {
        let name = "CROSSLIN-Meir-2026-06-30-final-waxupmodel.stl";
        let measure = |s: &str| s.chars().count() as f32 * 7.0;
        let max = 22.0 * 7.0; // room for roughly 22 characters

        let out = elide_middle(name, max, measure);

        assert!(
            out.contains('…'),
            "a long name should be middle-elided: {out}"
        );
        assert!(
            out.ends_with(".stl"),
            "the file extension must stay visible: {out}"
        );
        let head: String = out.chars().take_while(|&c| c != '…').collect();
        assert!(
            name.starts_with(&head),
            "the elided head must be a real prefix of the name: {out}"
        );
        assert!(
            measure(&out) <= max,
            "the elided title must fit the width budget: {out}"
        );
        assert!(
            out.chars().count() < name.chars().count(),
            "the elided title must be shorter than the original: {out}"
        );
    }

    #[test]
    fn elide_middle_falls_back_to_the_ellipsis_when_nothing_fits() {
        let measure = |s: &str| s.chars().count() as f32 * 7.0;
        assert_eq!(elide_middle("anything.stl", 3.0, measure), "…");
    }
}

use crate::layer_actions::{LayerContextAction, LayerContextRequest};
use crate::mesh_editor_icons::{paint_layer_menu, LayerMenuIcon, CELL_ROUNDING};
use crate::ui_theme;
use eframe::egui;
use occluview_core::SceneMeshId;

/// Fixed context-menu width. Wide enough that the longest label ("Show only
/// this layer") never wraps beside its gutter glyph, and it gives the elided
/// file-name title room to breathe.
const MENU_WIDTH: f32 = 216.0;

/// Everything the layer context menu needs about one layer. Shared verbatim by
/// the layers-overlay rows and the viewport right-click menu so both surface the
/// identical action set through the same plumbing.
// Four independent display/state flags, not a state machine — see SceneMesh.
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
    /// Whether this layer's scan colors/texture are currently shown (vs the
    /// flat neutral material).
    pub(crate) show_vertex_colors: bool,
    /// Whether the layer actually carries vertex colors or a texture — the
    /// toggle is a no-op (and stays disabled) on a plain uncolored scan.
    pub(crate) has_color_data: bool,
}

/// Attach the layer context menu to a widget response (row controls / row body).
pub(super) fn attach_layer_context_menu(
    response: egui::Response,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
) {
    response.context_menu(|ui| {
        show_layer_context_menu(ui, target, context_request);
    });
}

/// Render the layer context menu into `ui`. Used by both the row-attached menu
/// and the viewport right-click menu.
pub(crate) fn show_layer_context_menu(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
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
    show_visibility_actions(ui, target, context_request);
    ui.separator();
    show_material_actions(ui, target, context_request);
    ui.separator();
    show_mesh_edit_actions(ui, target, context_request);
    ui.separator();
    show_layer_actions(ui, target, context_request);
}

fn show_visibility_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
) {
    // The eye tracks the current state: an open eye to Hide, a slashed eye to
    // Show — the same glyph the layer row uses.
    let (visibility_label, eye) = if target.visible {
        ("Hide", LayerMenuIcon::EyeOpen)
    } else {
        ("Show", LayerMenuIcon::EyeSlash)
    };
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            eye,
            visibility_label,
            true,
            LayerContextAction::ToggleVisibility,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Solo,
            "Show only this layer",
            true,
            LayerContextAction::Solo,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::ShowAll,
            "Show all layers",
            true,
            LayerContextAction::ShowAll,
        ),
        context_request,
    );
}

fn show_material_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
) {
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Opacity,
            "Reset opacity",
            true,
            LayerContextAction::ResetOpacity,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Tint,
            "Next tint",
            true,
            LayerContextAction::NextTint,
        ),
        context_request,
    );
    let (colors_label, colors_icon) = if target.show_vertex_colors {
        ("Hide scan colors", LayerMenuIcon::ColorsOn)
    } else {
        ("Show scan colors", LayerMenuIcon::ColorsOff)
    };
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            colors_icon,
            colors_label,
            target.has_color_data,
            LayerContextAction::ToggleShowVertexColors,
        ),
        context_request,
    );
}

fn show_mesh_edit_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
) {
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Pencil,
            "Edit mesh",
            target.face_editable,
            LayerContextAction::EditMesh,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Repair,
            "Split bridge...",
            target.visible && target.face_editable,
            LayerContextAction::BridgeSplit,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Repair,
            "Repair mesh",
            target.face_editable,
            LayerContextAction::RepairMesh,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::FlipNormals,
            "Flip normals",
            target.face_editable,
            LayerContextAction::InvertNormals,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Export,
            "Export layer...",
            target.face_editable,
            LayerContextAction::ExportLayer,
        ),
        context_request,
    );
}

fn show_layer_actions(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    context_request: &mut Option<LayerContextRequest>,
) {
    let wireframe_label = if target.wireframe {
        "Hide wireframe"
    } else {
        "Wireframe overlay"
    };
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Wireframe,
            wireframe_label,
            true,
            LayerContextAction::ToggleWireframe,
        ),
        context_request,
    );
    layer_menu_button(
        ui,
        target,
        LayerMenuButton::new(
            LayerMenuIcon::Trash,
            "Remove layer",
            true,
            LayerContextAction::Remove,
        ),
        context_request,
    );
}

struct LayerMenuButton<'a> {
    icon: LayerMenuIcon,
    label: &'a str,
    enabled: bool,
    action: LayerContextAction,
}

impl<'a> LayerMenuButton<'a> {
    const fn new(
        icon: LayerMenuIcon,
        label: &'a str,
        enabled: bool,
        action: LayerContextAction,
    ) -> Self {
        Self {
            icon,
            label,
            enabled,
            action,
        }
    }
}

fn layer_menu_button(
    ui: &mut egui::Ui,
    target: &LayerContextMenuTarget,
    button: LayerMenuButton<'_>,
    context_request: &mut Option<LayerContextRequest>,
) {
    if menu_item(ui, button.icon, button.label, button.enabled).clicked() {
        *context_request = Some(LayerContextRequest {
            index: target.index,
            layer_id: target.layer_id,
            action: button.action,
        });
        ui.close_menu();
    }
}

/// One custom-rendered context-menu row: a vector glyph in a fixed left gutter,
/// then the label, over a rounded hover wash that matches the editor cells.
/// Fully painted (not an `egui::Button`) so every item aligns on the same gutter
/// and carries an icon. Returns the row `Response`.
fn menu_item(ui: &mut egui::Ui, icon: LayerMenuIcon, label: &str, enabled: bool) -> egui::Response {
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
        ui_theme::ACCENT
    } else {
        ui_theme::TEXT
    };

    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CELL_ROUNDING, ui_theme::ACCENT.gamma_multiply(0.12));
    }
    let icon_center = egui::pos2(rect.left() + PAD_L + GUTTER * 0.5, rect.center().y);
    let icon_rect = egui::Rect::from_center_size(icon_center, egui::vec2(ICON, ICON));
    paint_layer_menu(painter, icon_rect, icon, fg);
    painter.text(
        egui::pos2(rect.left() + PAD_L + GUTTER + LABEL_GAP, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        fg,
    );
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
            ctx.fonts(|fonts| {
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
        let mid = (lo + hi) / 2;
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
    // literal file extension; both are deliberate in test scaffolding.
    #![allow(
        clippy::cast_precision_loss,
        clippy::case_sensitive_file_extension_comparisons
    )]

    use super::{elide_middle, menu_title_name};

    #[test]
    fn every_menu_entry_carries_an_icon_glyph() {
        let source = include_str!("menu.rs").replace("\r\n", "\n");
        let production = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        // Twelve operator actions, each built through `LayerMenuButton::new`.
        let buttons = production.matches("LayerMenuButton::new(").count();
        assert!(
            buttons >= 12,
            "expected the full operator action set; found {buttons} menu buttons"
        );
        // Every button names a glyph (the eye picks open/slashed by state, so a
        // couple of extra glyph mentions are expected — hence >=, not ==).
        let glyphs = production.matches("LayerMenuIcon::").count();
        assert!(
            glyphs >= buttons,
            "every menu entry must carry an icon glyph: {glyphs} glyphs for {buttons} buttons"
        );
        assert!(
            production.contains("fn menu_item("),
            "rows should be custom-rendered with a fixed gutter glyph"
        );
        assert!(
            !production.contains("egui::Button::new("),
            "menu rows are painted glyph rows, not text-only egui buttons"
        );
    }

    #[test]
    fn menu_title_name_strips_a_path_to_its_file_name() {
        assert_eq!(
            menu_title_name(r"C:\cases\lower_scan.stl"),
            "lower_scan.stl"
        );
        assert_eq!(menu_title_name("/home/wow/upper.ply"), "upper.ply");
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

    #[test]
    fn layer_context_menu_keeps_mesh_edit_entry_without_inline_overflow() {
        let source = include_str!("menu.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("Edit mesh"),
            "layer context menu should expose the mesh edit entry point"
        );
        assert!(
            !production_source.contains("\"...\""),
            "layers should use direct controls and context menu actions, not a three-dot overflow"
        );
    }

    #[test]
    fn layer_context_menu_exposes_common_operator_actions() {
        let source = include_str!("menu.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);
        for label in [
            "Hide",
            "Show",
            "Show only this layer",
            "Show all layers",
            "Reset opacity",
            "Next tint",
            "Edit mesh",
            "Split bridge...",
            "Repair mesh",
            "Flip normals",
            "Export layer...",
            "Wireframe overlay",
            "Hide wireframe",
            "Hide scan colors",
            "Show scan colors",
            "Remove layer",
        ] {
            assert!(
                production_source.contains(label),
                "layer context menu should expose operator action: {label}"
            );
        }
    }

    #[test]
    fn destructive_mesh_edit_actions_stay_out_of_layer_context_menu() {
        let source = include_str!("menu.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            !production_source.contains("LayerContextAction::DeleteSelectedFaces")
                && !production_source.contains("LayerContextAction::CropToSelectedFaces")
                && !production_source.contains("LayerContextAction::CutSelectionToNewLayer")
                && !production_source.contains("LayerContextAction::SeparateSelectedComponents"),
            "delete/crop/cut/separate belong to the active mesh editor panel, not the layer context menu"
        );
    }
}

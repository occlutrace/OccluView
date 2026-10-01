mod color;
mod label;
mod layout;
mod menu;
mod row;
mod scene_menu;

use crate::layer_actions::LayerContextRequest;
use crate::ui_theme;
pub(crate) use color::color32_from_tint;
use eframe::egui;
pub(crate) use layout::{
    layer_overlay_desired_height, layer_overlay_rect, LAYER_OVERLAY_BOTTOM_RESERVE_PX,
    LAYER_OVERLAY_TOP_OFFSET_PX,
};
use layout::{LAYER_OVERLAY_CHROME_HEIGHT_PX, LAYER_SCENE_FOOTER_BUTTON_HEIGHT_PX};
use occluview_core::{Scene, SceneMeshId};
use row::{show_layer_row, LayerRowState, LayerRowView};
use std::path::PathBuf;

use label::layer_hover;
pub(crate) use label::{ascii_layer_stem, layer_label};
pub(crate) use menu::{show_layer_context_menu, LayerContextMenuTarget};
pub(crate) use row::LayerRowChange;
pub(crate) use scene_menu::{show_scene_context_menu, SceneContextAction};

const LAYER_LIST_COLLAPSING_ID: &str = "workspace-layers-list-collapse";

/// Geometry currently used by the one shared Layers panel, including its
/// saved collapsed state. Input hit testing must follow the same state as the
/// painted panel so a collapsed header does not reserve invisible row space.
pub(crate) fn current_panel_rect(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    layer_count: usize,
) -> egui::Rect {
    let expanded = egui::collapsing_header::CollapsingState::load_with_default_open(
        ctx,
        egui::Id::new(LAYER_LIST_COLLAPSING_ID),
        true,
    )
    .is_open();
    layout::layer_overlay_rect_for_state(viewport_rect, layer_count, expanded)
}

#[derive(Default)]
pub(crate) struct LayerOverlayChanges {
    pub(crate) context_request: Option<LayerContextRequest>,
    pub(crate) layer_edits: Vec<LayerRowChange>,
    /// Generic layer focus used by workspace commands. This does not start or
    /// retarget a mesh-edit session.
    pub(crate) focused_layer_id: Option<SceneMeshId>,
    pub(crate) scene_action: Option<LayerOverlaySceneAction>,
    pub(crate) drag_started: Option<LayerDragSource>,
    /// Rectangles are returned so the app can route drops onto explicit scene
    /// tabs without making the overlay own workspace mutations.
    pub(crate) scene_tab_rects: Vec<(u64, egui::Rect)>,
    pub(crate) scene_create_rect: Option<egui::Rect>,
}

/// A scene tab supplied by the workspace owner. The overlay only displays the
/// name and opaque ID; it never looks up or mutates a scene.
#[derive(Clone, Copy)]
pub(crate) struct LayerSceneTab<'a> {
    pub(crate) id: u64,
    pub(crate) name: &'a str,
}

/// The optional scene switcher shown at the bottom of the shared Layers panel.
#[derive(Clone, Copy)]
pub(crate) struct LayerSceneTabs<'a> {
    pub(crate) scenes: &'a [LayerSceneTab<'a>],
    pub(crate) active_scene_id: u64,
    pub(crate) can_create: bool,
}

/// Commands raised by the panel and interpreted by the workspace owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerOverlaySceneAction {
    Activate(u64),
    Create,
    TransferToNew {
        layer_id: SceneMeshId,
    },
    Transfer {
        layer_id: SceneMeshId,
        scene_id: u64,
    },
}

/// Source identity for a layer drag. The app tracks the pointer and decides
/// which viewport or tab is the destination. The label and tint are captured
/// here because only the row knows them; the drag preview would otherwise have
/// to rebuild them from the source scene every frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerDragSource {
    pub(crate) scene_id: u64,
    pub(crate) layer_id: SceneMeshId,
    pub(crate) label: String,
    pub(crate) tint: [f32; 4],
}

/// What the layer rows and the viewport menu need to offer the contact reading.
///
/// Passed in rather than read from the scene: whether a reading can be opened on
/// a layer depends on the other layers (it needs a second visible surface), and
/// that rule lives with the contact feature, not with the overlay that draws the
/// rows.
pub(crate) struct LayerContactRows<'a> {
    /// Whether each layer is currently wearing contact marks, one entry per
    /// scene layer.
    ///
    /// A reading paints both arches of its pair, so both rows offer to close it;
    /// a single marked index could name only one of them and would leave the
    /// other offering to open a second reading on the same scans.
    ///
    /// Shorter than the layer list means "not marked".
    pub(crate) marked: &'a [bool],
    /// Whether a contact reading can be opened, one entry per scene layer.
    /// Shorter than the layer list means "not readable".
    pub(crate) readable: &'a [bool],
}

// Explicit viewport, workspace and layer inputs make the overlay's ownership
// boundary visible at its call site.
#[expect(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    scene: &Scene,
    paths: &[PathBuf],
    active_layer_id: Option<SceneMeshId>,
    focused_layer_id: Option<SceneMeshId>,
    scene_tabs: Option<&LayerSceneTabs<'_>>,
    contacts: LayerContactRows<'_>,
    locale: &crate::i18n::LocaleManager,
) -> LayerOverlayChanges {
    let layer_count = scene.meshes().len();
    let mut layer_edits = Vec::new();
    let mut layer_context_request = None;
    let mut focused_layer_change = None;
    let mut scene_action = None;
    let mut drag_started = None;
    let mut scene_tab_rects = Vec::new();
    let mut scene_create_rect = None;

    let collapsing_id = egui::Id::new(LAYER_LIST_COLLAPSING_ID);
    let collapsing_state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        collapsing_id,
        true,
    );
    let expanded = collapsing_state.is_open();
    let overlay_rect = layout::layer_overlay_rect_for_state(viewport_rect, layer_count, expanded);
    ui.scope_builder(egui::UiBuilder::new().max_rect(overlay_rect), |ui| {
        ui_theme::overlay_frame().show(ui, |ui| {
            let overlay_inner_width = overlay_rect.width() - 20.0;
            ui.set_min_width(overlay_inner_width);
            ui.set_max_width(overlay_inner_width);
            let title = locale.tr(crate::i18n::message_id!("layers-title"));
            let (toggle_response, _, _) = collapsing_state
                .show_header(ui, |ui| show_header(ui, layer_count, locale))
                .body_unindented(|ui| {
                    show_header_separator(ui, overlay_inner_width);
                    // The footer stays outside this scroll area even when the
                    // layer list is long. Empty scenes still reserve one row.
                    let rows_height =
                        (overlay_rect.height() - LAYER_OVERLAY_CHROME_HEIGHT_PX).max(0.0);
                    egui::ScrollArea::vertical()
                        .max_height(rows_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (index, entry) in scene.meshes().iter().enumerate() {
                                let label = layer_label(paths, entry, index, locale);
                                let hover = layer_hover(paths, entry, index, locale);
                                let interaction = show_layer_row(
                                    ui,
                                    overlay_inner_width,
                                    LayerRowState {
                                        visible: entry.visible,
                                        opacity: entry.opacity,
                                        tint: entry.tint,
                                        wireframe: entry.wireframe,
                                        face_editable: !entry.mesh.is_point_cloud(),
                                        can_export: !entry.mesh.vertices().is_empty(),
                                        show_vertex_colors: entry.show_vertex_colors,
                                        show_texture: entry.show_texture
                                            && entry.show_vertex_colors,
                                        has_color_data: entry.mesh.carries_color_data(),
                                        has_texture: entry.mesh.texture().is_some(),
                                        contacts: contacts
                                            .marked
                                            .get(index)
                                            .copied()
                                            .unwrap_or(false),
                                        can_read_contacts: contacts
                                            .readable
                                            .get(index)
                                            .copied()
                                            .unwrap_or(false),
                                    },
                                    LayerRowView {
                                        index,
                                        layer_id: entry.id(),
                                        label: &label,
                                        hover: Some(hover.as_str()),
                                        active: active_layer_id == Some(entry.id()),
                                        focused: focused_layer_id == Some(entry.id()),
                                    },
                                    &mut layer_context_request,
                                    scene_tabs,
                                    &mut scene_action,
                                    locale,
                                );
                                if let Some(edit) = interaction.edit {
                                    layer_edits.push(edit);
                                }
                                focused_layer_change =
                                    interaction.focused_layer_id.or(focused_layer_change);
                                if interaction.drag_started {
                                    drag_started = scene_tabs.map(|tabs| LayerDragSource {
                                        scene_id: tabs.active_scene_id,
                                        layer_id: entry.id(),
                                        label: label.clone(),
                                        tint: entry.tint,
                                    });
                                }
                            }
                        });
                });
            crate::accessibility::button(&toggle_response, &title, true, Some(expanded));

            if let Some(scene_tabs) = scene_tabs {
                let footer = show_scene_footer(ui, overlay_inner_width, scene_tabs, locale);
                scene_tab_rects = footer.tab_rects;
                scene_create_rect = footer.create_rect;
                if let Some(action) = footer.action {
                    scene_action = Some(action);
                }
            }
        });
    });

    LayerOverlayChanges {
        context_request: layer_context_request,
        layer_edits,
        focused_layer_id: focused_layer_change,
        scene_action,
        drag_started,
        scene_tab_rects,
        scene_create_rect,
    }
}

struct SceneFooterChanges {
    tab_rects: Vec<(u64, egui::Rect)>,
    create_rect: Option<egui::Rect>,
    action: Option<LayerOverlaySceneAction>,
}

#[allow(clippy::cast_precision_loss)]
fn show_scene_footer(
    ui: &mut egui::Ui,
    inner_width: f32,
    scene_tabs: &LayerSceneTabs<'_>,
    locale: &crate::i18n::LocaleManager,
) -> SceneFooterChanges {
    let mut changes = SceneFooterChanges {
        tab_rects: Vec::with_capacity(scene_tabs.scenes.len()),
        create_rect: None,
        action: None,
    };

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
    let has_create_button = scene_tabs.can_create;
    let plus_width = if has_create_button { 24.0 } else { 0.0 };
    let gaps = if scene_tabs.scenes.is_empty() {
        0.0
    } else {
        4.0 * scene_tabs.scenes.len() as f32
    };
    let tab_width = if scene_tabs.scenes.is_empty() {
        0.0
    } else {
        ((inner_width - plus_width - gaps).max(0.0) / scene_tabs.scenes.len() as f32).max(0.0)
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for tab in scene_tabs.scenes {
            let selected = tab.id == scene_tabs.active_scene_id;
            let button = egui::Button::selectable(
                selected,
                egui::RichText::new(tab.name).size(10.5).color(if selected {
                    ui_theme::text()
                } else {
                    ui_theme::text_weak()
                }),
            )
            .truncate();
            let response = ui.add_sized([tab_width, LAYER_SCENE_FOOTER_BUTTON_HEIGHT_PX], button);
            crate::accessibility::button(&response, tab.name, true, Some(selected));
            let response = response.on_hover_text(tab.name);
            if response.clicked() {
                changes.action = Some(LayerOverlaySceneAction::Activate(tab.id));
            }
            changes.tab_rects.push((tab.id, response.rect));
        }

        if has_create_button {
            let label = locale.tr(crate::i18n::message_id!("workspace-scene-new"));
            let response = ui.add_sized(
                [plus_width, LAYER_SCENE_FOOTER_BUTTON_HEIGHT_PX],
                egui::Button::new(egui::RichText::new("+").size(15.0)),
            );
            crate::accessibility::button(&response, &label, true, None);
            let response = response.on_hover_text(&label);
            if response.clicked() {
                changes.action = Some(LayerOverlaySceneAction::Create);
            }
            changes.create_rect = Some(response.rect);
        }
    });

    // The local UI owns the footer's height; returning exact button rects lets
    // the app resolve drag destinations in the same frame and coordinate space.
    changes
}

fn show_header(ui: &mut egui::Ui, layer_count: usize, locale: &crate::i18n::LocaleManager) {
    let count_text = locale.tr_plural(
        crate::i18n::message_id!("layers-count"),
        &[],
        &[("count", layer_count)],
    );
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(locale.tr(crate::i18n::message_id!("layers-title")))
                .color(ui_theme::text())
                .size(12.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(count_text)
                    .color(ui_theme::text_weak())
                    .size(11.0),
            );
        });
    });
}

fn show_header_separator(ui: &mut egui::Ui, inner_width: f32) {
    ui.add_space(4.0);
    let y = ui.cursor().min.y;
    let left = ui.cursor().min.x;
    ui.painter().hline(
        egui::Rangef::new(left, left + inner_width),
        y,
        egui::Stroke::new(1.0_f32, ui_theme::hairline()),
    );
    ui.add_space(4.0);
}

mod color;
mod label;
mod layout;
mod menu;
mod row;
mod scene_menu;

use crate::layer_actions::LayerContextRequest;
use crate::ui_theme;
use eframe::egui;
pub(crate) use layout::{
    layer_overlay_desired_height, layer_overlay_rect, LAYER_OVERLAY_BOTTOM_RESERVE_PX,
    LAYER_OVERLAY_TOP_OFFSET_PX,
};
use layout::{LAYER_OVERLAY_CHROME_HEIGHT_PX, LAYER_ROW_HEIGHT_PX};
use occluview_core::{Scene, SceneMeshId};
use row::{show_layer_row, LayerRowState, LayerRowView};
use std::path::PathBuf;

use label::layer_hover;
pub(crate) use label::{ascii_layer_stem, layer_label};
pub(crate) use menu::{show_layer_context_menu, LayerContextMenuTarget};
pub(crate) use row::LayerRowChange;
pub(crate) use scene_menu::{show_scene_context_menu, SceneContextAction};

pub(crate) struct LayerOverlayChanges {
    pub(crate) context_request: Option<LayerContextRequest>,
    pub(crate) layer_edits: Vec<LayerRowChange>,
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

// Six inherent inputs (ui/ctx + data + locale); a struct grouping them would
// carry no meaning of its own.
#[expect(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    scene: &Scene,
    paths: &[PathBuf],
    active_layer_id: Option<SceneMeshId>,
    contacts: LayerContactRows<'_>,
    locale: &crate::i18n::LocaleManager,
) -> LayerOverlayChanges {
    let layer_count = scene.meshes().len();
    let mut layer_edits = Vec::new();
    let mut layer_context_request = None;

    let overlay_rect = layer_overlay_rect(viewport_rect, layer_count);
    ui.scope_builder(egui::UiBuilder::new().max_rect(overlay_rect), |ui| {
        ui_theme::overlay_frame().show(ui, |ui| {
            let overlay_inner_width = overlay_rect.width() - 20.0;
            ui.set_min_width(overlay_inner_width);
            ui.set_max_width(overlay_inner_width);
            show_header(ui, overlay_inner_width, layer_count, locale);

            // The row budget mirrors what `layer_overlay_rect` reserved for
            // rows (chrome minus header), so a panel sized for N rows shows
            // exactly N rows with no scrollbar on the boundary.
            let rows_height =
                (overlay_rect.height() - LAYER_OVERLAY_CHROME_HEIGHT_PX).max(LAYER_ROW_HEIGHT_PX);
            egui::ScrollArea::vertical()
                .max_height(rows_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, entry) in scene.meshes().iter().enumerate() {
                        let label = layer_label(paths, entry, index, locale);
                        let hover = layer_hover(paths, entry, index, locale);
                        if let Some(edit) = show_layer_row(
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
                                show_texture: entry.show_texture && entry.show_vertex_colors,
                                has_color_data: entry.mesh.carries_color_data(),
                                has_texture: entry.mesh.texture().is_some(),
                                contacts: contacts.marked.get(index).copied().unwrap_or(false),
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
                            },
                            &mut layer_context_request,
                            locale,
                        ) {
                            layer_edits.push(edit);
                        }
                    }
                });
        });
    });

    LayerOverlayChanges {
        context_request: layer_context_request,
        layer_edits,
    }
}

fn show_header(
    ui: &mut egui::Ui,
    inner_width: f32,
    layer_count: usize,
    locale: &crate::i18n::LocaleManager,
) {
    let count_text = locale.tr_plural("layers-count", &[], &[("count", layer_count)]);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(locale.tr("layers-title"))
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

mod color;
mod label;
mod layout;
mod menu;
mod row;

use crate::layer_actions::LayerContextRequest;
use crate::ui_theme;
use eframe::egui;
pub(crate) use layout::layer_overlay_rect;
use layout::{LAYER_OVERLAY_HEADER_HEIGHT_PX, LAYER_ROW_HEIGHT_PX};
use occluview_core::{Scene, SceneMeshId};
use row::{show_layer_row, LayerRowState, LayerRowView};
use std::path::PathBuf;

use label::layer_hover;
pub(crate) use label::layer_label;
pub(crate) use menu::{show_layer_context_menu, LayerContextMenuTarget};
pub(crate) use row::LayerRowChange;

pub(crate) struct LayerOverlayChanges {
    pub(crate) context_request: Option<LayerContextRequest>,
    pub(crate) layer_edits: Vec<LayerRowChange>,
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    scene: &Scene,
    paths: &[PathBuf],
    active_layer_id: Option<SceneMeshId>,
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
            show_header(ui, overlay_inner_width, layer_count);

            let rows_height =
                (overlay_rect.height() - LAYER_OVERLAY_HEADER_HEIGHT_PX).max(LAYER_ROW_HEIGHT_PX);
            egui::ScrollArea::vertical()
                .max_height(rows_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, entry) in scene.meshes().iter().enumerate() {
                        let label = layer_label(paths, entry, index);
                        let hover = layer_hover(paths, entry, index);
                        if let Some(edit) = show_layer_row(
                            ui,
                            overlay_inner_width,
                            LayerRowState {
                                visible: entry.visible,
                                opacity: entry.opacity,
                                tint: entry.tint,
                                wireframe: entry.wireframe,
                                face_editable: !entry.mesh.is_point_cloud(),
                                show_vertex_colors: entry.show_vertex_colors,
                                has_color_data: entry.mesh.carries_color_data(),
                            },
                            LayerRowView {
                                index,
                                layer_id: entry.id(),
                                label: &label,
                                hover: Some(hover.as_str()),
                                active: active_layer_id == Some(entry.id()),
                            },
                            &mut layer_context_request,
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

/// A quiet title row (name + layer count) closed by a hairline separator.
fn show_header(ui: &mut egui::Ui, inner_width: f32, layer_count: usize) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Layers")
                .color(ui_theme::TEXT)
                .size(12.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(layer_count.to_string())
                    .color(ui_theme::TEXT_WEAK)
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
        egui::Stroke::new(1.0, ui_theme::hairline()),
    );
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    #[test]
    fn layer_overlay_facade_stays_split_by_responsibility() {
        let facade = include_str!("mod.rs").replace("\r\n", "\n");
        let production_source = facade
            .split_once("\nmod tests {")
            .map_or(facade.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("mod layout;")
                && production_source.contains("mod row;")
                && production_source.contains("mod menu;")
                && production_source.contains("mod label;"),
            "layers overlay should stay split by focused responsibility"
        );
        assert!(
            production_source.contains("pub(crate) fn show(")
                && production_source.contains("pub(crate) use label::layer_label;"),
            "facade should preserve the crate API used by app internals"
        );
        assert!(
            production_source.contains("pub(crate) use menu::{show_layer_context_menu"),
            "facade should re-export the shared context menu for the viewport right-click"
        );
        assert!(
            !production_source.contains("fn show_layer_row(")
                && !production_source.contains("fn show_layer_context_menu("),
            "facade should not absorb row or menu implementation"
        );
    }

    #[test]
    fn layer_rows_use_a_shared_overlay_width_instead_of_per_row_available_width() {
        let source = include_str!("mod.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("let overlay_inner_width = overlay_rect.width() - 20.0;"),
            "layers overlay should derive one stable inner width for every row"
        );
        assert!(
            production_source.contains("show_layer_row(")
                && production_source.contains("overlay_inner_width,"),
            "layer rows should receive the shared overlay width explicitly"
        );
        assert!(
            !production_source.contains("let row_width = ui.available_width().max(0.0);"),
            "layer rows must not size themselves from per-row available width"
        );
    }

    #[test]
    fn layer_rows_route_hover_text_through_the_shared_label_helper() {
        let source = include_str!("mod.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("let hover = layer_hover(paths, entry, index);"),
            "layer rows should derive hover text from the shared path/name helper"
        );
        assert!(
            production_source.contains("hover: Some(hover.as_str()),"),
            "layer rows should never hand egui an empty placeholder hover"
        );
    }
}

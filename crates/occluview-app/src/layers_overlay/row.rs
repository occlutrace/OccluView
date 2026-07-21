use super::color::color32_from_linear;
use super::layout::{
    layer_name_width, LAYER_ROW_ACTION_GAP_PX, LAYER_ROW_CONTROL_HEIGHT_PX, LAYER_ROW_EYE_WIDTH_PX,
    LAYER_ROW_GAP_PX, LAYER_ROW_HEIGHT_PX, LAYER_ROW_REMOVE_WIDTH_PX, LAYER_ROW_SLIDER_WIDTH_PX,
    LAYER_ROW_TINT_WIDTH_PX,
};
use super::menu::{attach_layer_context_menu, LayerContextMenuTarget};
use crate::layer_actions::{LayerContextAction, LayerContextRequest, LAYER_TINT_PRESETS};
use crate::ui_theme;
use eframe::egui;
use occluview_core::SceneMeshId;

pub(super) struct LayerRowView<'a> {
    pub(super) index: usize,
    pub(super) layer_id: SceneMeshId,
    pub(super) label: &'a str,
    pub(super) hover: Option<&'a str>,
    /// Whether this layer is the one currently open in the mesh editor.
    pub(super) active: bool,
}

// Four independent display/state flags, not a state machine — see SceneMesh.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub(super) struct LayerRowState {
    pub(super) visible: bool,
    pub(super) opacity: f32,
    pub(super) tint: [f32; 4],
    pub(super) wireframe: bool,
    pub(super) face_editable: bool,
    pub(super) show_vertex_colors: bool,
    pub(super) show_texture: bool,
    pub(super) has_color_data: bool,
    pub(super) has_texture: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct LayerRowChange {
    pub(crate) index: usize,
    pub(crate) visible: bool,
    pub(crate) opacity: f32,
    pub(crate) tint: [f32; 4],
}

#[allow(clippy::too_many_lines)]
pub(super) fn show_layer_row(
    ui: &mut egui::Ui,
    row_width: f32,
    state: LayerRowState,
    view: LayerRowView<'_>,
    context_request: &mut Option<LayerContextRequest>,
) -> Option<LayerRowChange> {
    let mut changed = false;
    let mut visible = state.visible;
    let mut opacity = state.opacity;
    let mut tint = state.tint;

    let row_width = row_width.max(0.0);
    let row_size = egui::vec2(row_width, LAYER_ROW_HEIGHT_PX - 2.0);

    // Paint the hover / active-layer background under the controls first so the
    // controls render on top of it.
    let row_rect = egui::Rect::from_min_size(ui.cursor().min, row_size);
    let hovered = ui.rect_contains_pointer(row_rect);
    if view.active {
        ui.painter()
            .rect_filled(row_rect, 5.0, ui_theme::row_active_fill());
    } else if hovered {
        ui.painter()
            .rect_filled(row_rect, 5.0, ui_theme::row_hover_fill());
    }

    let target = |visible: bool| LayerContextMenuTarget {
        label: view.label.to_string(),
        index: view.index,
        layer_id: view.layer_id,
        visible,
        wireframe: state.wireframe,
        face_editable: state.face_editable,
        show_vertex_colors: state.show_vertex_colors,
        show_texture: state.show_texture && state.show_vertex_colors,
        has_color_data: state.has_color_data,
        has_texture: state.has_texture,
    };

    let row_response = ui
        .allocate_ui_with_layout(
            row_size,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(row_width);
                ui.set_max_width(row_width);
                // Explicit gaps: uniform between columns, a wider one guarding
                // the destructive remove control.
                ui.spacing_mut().item_spacing.x = 0.0;

                // Visibility eye.
                let (eye_rect, eye_response) = ui.allocate_exact_size(
                    egui::vec2(LAYER_ROW_EYE_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX),
                    egui::Sense::click(),
                );
                ui_theme::paint_visibility_eye(ui.painter(), eye_rect, visible);
                let eye_response =
                    eye_response.on_hover_text(if visible { "Hide layer" } else { "Show layer" });
                if eye_response.clicked() {
                    visible = !visible;
                    changed = true;
                }
                attach_layer_context_menu(eye_response, &target(visible), context_request);

                ui.add_space(LAYER_ROW_GAP_PX);

                // Name (fills remaining width, middle-truncates).
                let label_width = layer_name_width(row_width);
                let label = egui::Label::new(
                    egui::RichText::new(view.label)
                        .color(ui_theme::TEXT)
                        .size(11.5),
                )
                .truncate()
                .sense(egui::Sense::click());
                let label_response =
                    ui.add_sized([label_width, LAYER_ROW_CONTROL_HEIGHT_PX], label);
                let label_response = if let Some(hover) = view.hover {
                    label_response.on_hover_text(hover)
                } else {
                    label_response
                };
                attach_layer_context_menu(label_response, &target(visible), context_request);

                ui.add_space(LAYER_ROW_GAP_PX);

                // Opacity scrub.
                let slider_response = ui
                    .add_enabled_ui(visible, |ui| {
                        ui.add_sized(
                            [LAYER_ROW_SLIDER_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX],
                            egui::Slider::new(&mut opacity, 0.1..=1.0)
                                .show_value(false)
                                .step_by(0.01),
                        )
                    })
                    .inner
                    .on_hover_text("Layer opacity");
                changed |= slider_response.changed();
                attach_layer_context_menu(slider_response, &target(visible), context_request);

                ui.add_space(LAYER_ROW_GAP_PX);

                // Tint swatch + palette popup. Right-click coverage comes from
                // the surrounding row body/label, so the swatch stays lean.
                if tint_swatch(ui, &view, visible, &mut tint) {
                    changed = true;
                }

                ui.add_space(LAYER_ROW_ACTION_GAP_PX);

                // Remove.
                let (remove_rect, remove_response) = ui.allocate_exact_size(
                    egui::vec2(LAYER_ROW_REMOVE_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX),
                    egui::Sense::click(),
                );
                paint_remove_glyph(ui.painter(), remove_rect, remove_response.hovered());
                let remove_response = remove_response.on_hover_text("Remove layer");
                if remove_response.clicked() {
                    *context_request = Some(LayerContextRequest {
                        index: view.index,
                        layer_id: view.layer_id,
                        action: LayerContextAction::Remove,
                    });
                }
                attach_layer_context_menu(remove_response, &target(visible), context_request);
            },
        )
        .response;
    attach_layer_context_menu(row_response, &target(visible), context_request);

    changed.then_some(LayerRowChange {
        index: view.index,
        visible,
        opacity,
        tint,
    })
}

/// A color swatch that opens a small named palette popup. Selecting a preset
/// sets the tint directly (a real color choice), rather than blind-cycling.
fn tint_swatch(
    ui: &mut egui::Ui,
    view: &LayerRowView<'_>,
    enabled: bool,
    tint: &mut [f32; 4],
) -> bool {
    let mut changed = false;
    let swatch = egui::Button::new("")
        .fill(color32_from_linear(*tint))
        .stroke(egui::Stroke::new(1.0, ui_theme::panel_stroke()));
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                [LAYER_ROW_TINT_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX],
                swatch,
            )
        })
        .inner
        .on_hover_text("Choose tint");

    let popup_id = ui.make_persistent_id(("layer_tint_palette", view.layer_id));
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(150.0);
            ui.label(
                egui::RichText::new("Tint")
                    .color(ui_theme::TEXT_WEAK)
                    .size(10.5),
            );
            for (color, name) in LAYER_TINT_PRESETS {
                let is_current = tint_eq(color, *tint);
                let entry = ui
                    .horizontal(|ui| {
                        let (swatch_rect, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(swatch_rect, 3.0, color32_from_linear(color));
                        ui.painter().rect_stroke(
                            swatch_rect,
                            3.0,
                            egui::Stroke::new(1.0, ui_theme::hairline()),
                        );
                        ui.selectable_label(is_current, name)
                    })
                    .inner;
                if entry.clicked() {
                    *tint = color;
                    changed = true;
                }
            }
        },
    );
    changed
}

fn tint_eq(lhs: [f32; 4], rhs: [f32; 4]) -> bool {
    lhs.iter()
        .zip(rhs.iter())
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

/// A crisp remove "x", quiet at rest and accented on hover.
fn paint_remove_glyph(painter: &egui::Painter, rect: egui::Rect, hovered: bool) {
    let color = if hovered {
        ui_theme::ACCENT
    } else {
        ui_theme::TEXT_MUTED
    };
    let stroke = egui::Stroke::new(1.4, color);
    let inset = rect.shrink(rect.width() * 0.30);
    painter.line_segment([inset.left_top(), inset.right_bottom()], stroke);
    painter.line_segment([inset.right_top(), inset.left_bottom()], stroke);
}

#[cfg(test)]
mod tests {
    #[test]
    fn layer_row_uses_vector_controls_not_text_toggle() {
        let source = include_str!("row.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\n#[cfg(test)]")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("ui_theme::paint_visibility_eye("),
            "visibility should be a drawn eye toggle, not an On/Off text button"
        );
        assert!(
            !production_source.contains("\"On\"") && !production_source.contains("\"Off\""),
            "the eye replaces the On/Off text toggle"
        );
        assert!(
            production_source.contains("paint_remove_glyph("),
            "remove should be a crisp drawn x, not a cramped text character"
        );
    }

    #[test]
    fn tint_is_a_real_palette_choice_not_blind_cycling() {
        let source = include_str!("row.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\n#[cfg(test)]")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("popup_below_widget")
                && production_source.contains("LAYER_TINT_PRESETS"),
            "the tint swatch should open a named palette popup with the preset colors"
        );
    }

    #[test]
    fn layer_row_controls_share_fixed_height_constant() {
        let source = include_str!("row.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\n#[cfg(test)]")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source
                .matches("LAYER_ROW_CONTROL_HEIGHT_PX")
                .count()
                >= 4,
            "eye, slider, tint swatch, and remove should share one row control height"
        );
    }

    #[test]
    fn layer_row_exposes_context_menu_for_right_click() {
        let source = include_str!("row.rs").replace("\r\n", "\n");
        let production_source = source
            .split_once("\n#[cfg(test)]")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("attach_layer_context_menu(row_response"),
            "right-clicking a layer row should open the shared layer context menu"
        );
    }
}

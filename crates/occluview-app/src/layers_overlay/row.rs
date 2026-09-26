use super::color::color32_from_tint;
use super::layout::{
    layer_name_width, LAYER_ROW_ACTION_GAP_PX, LAYER_ROW_CONTROL_HEIGHT_PX, LAYER_ROW_EYE_WIDTH_PX,
    LAYER_ROW_GAP_PX, LAYER_ROW_HEIGHT_PX, LAYER_ROW_REMOVE_WIDTH_PX, LAYER_ROW_SLIDER_WIDTH_PX,
    LAYER_ROW_TINT_WIDTH_PX,
};
use super::menu::{attach_layer_context_menu, LayerContextMenuTarget};
use crate::layer_actions::{
    tint_matches, LayerContextAction, LayerContextRequest, LAYER_OVERLAY_TINT_PRESETS,
    LAYER_TINT_PRESETS,
};
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

// Five independent display/state flags, not a state machine — see SceneMesh.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub(super) struct LayerRowState {
    pub(super) visible: bool,
    pub(super) opacity: f32,
    pub(super) tint: [f32; 4],
    pub(super) wireframe: bool,
    pub(super) face_editable: bool,
    pub(super) can_export: bool,
    pub(super) show_vertex_colors: bool,
    /// Whether this layer wears occlusal-contact marks right now.
    pub(super) contacts: bool,
    /// Whether a contact reading can be opened on this layer.
    pub(super) can_read_contacts: bool,
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
    /// Whether the tint value comes from a swatch click this frame, rather
    /// than riding along on an opacity drag or a visibility toggle. The
    /// apply side keys its colour overrides on this, so re-picking the
    /// current colour still counts as picking it.
    pub(crate) tint_clicked: bool,
}

#[allow(clippy::too_many_lines)]
// Six inherent inputs (ui/ctx + data + locale); a struct grouping them would
// carry no meaning of its own.
#[expect(clippy::too_many_arguments)]
pub(super) fn show_layer_row(
    ui: &mut egui::Ui,
    row_width: f32,
    state: LayerRowState,
    view: LayerRowView<'_>,
    context_request: &mut Option<LayerContextRequest>,
    locale: &crate::i18n::LocaleManager,
) -> Option<LayerRowChange> {
    let mut changed = false;
    let mut visible = state.visible;
    let mut opacity = state.opacity;
    let mut tint = state.tint;
    let mut tint_clicked = false;

    let row_width = row_width.max(0.0);
    let row_size = egui::vec2(row_width, LAYER_ROW_HEIGHT_PX - 2.0);

    // Paint the hover / active-layer background under the controls first so the
    // controls render on top of it.
    let row_rect = egui::Rect::from_min_size(ui.cursor().min, row_size);
    let hovered = ui.rect_contains_pointer(row_rect);
    if view.active {
        ui.painter()
            .rect_filled(row_rect, 5.0, ui_theme::row_active_fill());
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                row_rect.min,
                egui::pos2(row_rect.min.x + 3.0, row_rect.max.y),
            ),
            1.5,
            ui_theme::accent(),
        );
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
        can_export: state.can_export,
        show_vertex_colors: state.show_vertex_colors,
        show_texture: state.show_texture && state.show_vertex_colors,
        has_color_data: state.has_color_data,
        has_texture: state.has_texture,
        contacts: state.contacts,
        can_read_contacts: state.can_read_contacts,
    };

    // Click-sense catch-all under the controls: a right-click in the gaps
    // between columns still opens the shared layer menu. The controls are
    // registered after it and sit on top, so they keep input priority.
    let row_hit = ui.interact(
        row_rect,
        ui.id().with(("layer-row-background", view.index)),
        egui::Sense::click(),
    );

    ui.allocate_ui_with_layout(
        row_size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(row_width);
            ui.set_max_width(row_width);
            // Explicit gaps: uniform between columns, a wider one guarding
            // the destructive remove control.
            ui.spacing_mut().item_spacing.x = 0.0;

            let (eye_rect, eye_response) = ui.allocate_exact_size(
                egui::vec2(LAYER_ROW_EYE_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX),
                egui::Sense::click(),
            );
            crate::icons::paint(
                ui.painter(),
                eye_rect,
                if visible {
                    crate::icons::AppIcon::Eye
                } else {
                    crate::icons::AppIcon::EyeOff
                },
                if visible {
                    ui_theme::text()
                } else {
                    ui_theme::text_muted()
                },
            );
            let eye_hint = if visible {
                locale.tr("layers-row-hide")
            } else {
                locale.tr("layers-row-show")
            };
            let eye_response = eye_response.on_hover_text(eye_hint);
            if eye_response.clicked() {
                visible = !visible;
                changed = true;
            }
            attach_layer_context_menu(eye_response, &target(visible), context_request, locale);

            ui.add_space(LAYER_ROW_GAP_PX);

            // Name (fills remaining width, middle-truncates).
            let label_width = layer_name_width(row_width);
            let label = egui::Label::new(
                egui::RichText::new(view.label)
                    .color(ui_theme::text())
                    .size(11.5),
            )
            .truncate()
            .sense(egui::Sense::click());
            let label_response = ui.add_sized([label_width, LAYER_ROW_CONTROL_HEIGHT_PX], label);
            let label_response = if let Some(hover) = view.hover {
                label_response.on_hover_text(hover)
            } else {
                label_response
            };
            attach_layer_context_menu(label_response, &target(visible), context_request, locale);

            ui.add_space(LAYER_ROW_GAP_PX);

            // Opacity scrub.
            let slider_response = ui
                .add_enabled_ui(visible, |ui| {
                    // egui's Slider reads spacing.slider_width, even when
                    // add_sized supplies a narrower rectangle. Without this
                    // the row widens the whole Layers frame past its rect.
                    ui.spacing_mut().slider_width = LAYER_ROW_SLIDER_WIDTH_PX;
                    ui.add_sized(
                        [LAYER_ROW_SLIDER_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX],
                        egui::Slider::new(&mut opacity, 0.1..=1.0)
                            .show_value(false)
                            .step_by(0.01),
                    )
                })
                .inner
                .on_hover_text(locale.tr("layers-row-opacity"));
            changed |= slider_response.changed();
            attach_layer_context_menu(slider_response, &target(visible), context_request, locale);

            ui.add_space(LAYER_ROW_GAP_PX);

            // Tint swatch + palette popup. The swatch is a real button, so
            // it eats its own presses: it carries the context menu itself
            // rather than relying on the row around it.
            let (swatch_changed, swatch_response) =
                tint_swatch(ui, &view, visible, &mut tint, locale);
            if swatch_changed {
                changed = true;
                tint_clicked = true;
            }
            attach_layer_context_menu(swatch_response, &target(visible), context_request, locale);

            ui.add_space(LAYER_ROW_ACTION_GAP_PX);

            // Remove.
            let (remove_rect, remove_response) = ui.allocate_exact_size(
                egui::vec2(LAYER_ROW_REMOVE_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX),
                egui::Sense::click(),
            );
            crate::icons::paint(
                ui.painter(),
                remove_rect,
                crate::icons::AppIcon::Close,
                if remove_response.hovered() {
                    ui_theme::accent()
                } else {
                    ui_theme::text_muted()
                },
            );
            let remove_response = remove_response.on_hover_text(locale.tr("layers-row-remove"));
            if remove_response.clicked() {
                *context_request = Some(LayerContextRequest {
                    index: view.index,
                    layer_id: view.layer_id,
                    action: LayerContextAction::Remove,
                });
            }
            attach_layer_context_menu(remove_response, &target(visible), context_request, locale);
        },
    );
    attach_layer_context_menu(row_hit, &target(visible), context_request, locale);

    changed.then_some(LayerRowChange {
        index: view.index,
        visible,
        opacity,
        tint,
        tint_clicked,
    })
}

/// How tall the tint palette popup may get before it scrolls. Enough for the
/// model shades and the first overlay colours at once, so the two groups are
/// visibly two groups without the list reaching the bottom of the window.
const TINT_PALETTE_MAX_HEIGHT_PX: f32 = 300.0;

/// A color swatch that opens a small named palette popup. Selecting a preset
/// sets the tint directly (a real color choice), rather than blind-cycling.
/// Returns whether a preset was picked, plus the swatch response so the caller
/// can attach the shared context menu (the button swallows its own presses).
fn tint_swatch(
    ui: &mut egui::Ui,
    view: &LayerRowView<'_>,
    enabled: bool,
    tint: &mut [f32; 4],
    locale: &crate::i18n::LocaleManager,
) -> (bool, egui::Response) {
    let mut changed = false;
    let swatch = egui::Button::new("")
        .fill(color32_from_tint(*tint))
        .stroke(egui::Stroke::new(1.0_f32, ui_theme::panel_stroke()));
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                [LAYER_ROW_TINT_WIDTH_PX, LAYER_ROW_CONTROL_HEIGHT_PX],
                swatch,
            )
        })
        .inner
        .on_hover_text(locale.tr("tint-choose"));

    let popup_id = ui.make_persistent_id(("layer_tint_palette", view.layer_id));
    egui::Popup::from_toggle_button_response(&response)
        .id(popup_id)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(170.0);
            // Bounded and scrolling: the palette is two groups long, and a
            // popup opening off a layer row near the bottom of the window would
            // otherwise run past the edge and put its last colours out of
            // reach.
            egui::ScrollArea::vertical()
                .max_height(TINT_PALETTE_MAX_HEIGHT_PX)
                .show(ui, |ui| {
                    // Two headed groups rather than one long list: the model shades
                    // are neighbours on one warm band, so two scans wearing any two of
                    // them are still hard to tell apart where they overlap, which is
                    // where an alignment needs them told apart.
                    for (heading, presets) in [
                        ("Model", LAYER_TINT_PRESETS.as_slice()),
                        (
                            "Overlay — two scans at once",
                            LAYER_OVERLAY_TINT_PRESETS.as_slice(),
                        ),
                    ] {
                        ui.label(
                            egui::RichText::new(heading)
                                .color(ui_theme::text_weak())
                                .size(10.5),
                        );
                        for &(color, name) in presets {
                            let is_current = tint_matches(color, *tint);
                            let entry = ui
                                .horizontal(|ui| {
                                    let (swatch_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(16.0, 16.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        swatch_rect,
                                        3.0,
                                        color32_from_tint(color),
                                    );
                                    ui.painter().rect_stroke(
                                        swatch_rect,
                                        3.0,
                                        egui::Stroke::new(1.0_f32, ui_theme::hairline()),
                                        egui::StrokeKind::Middle,
                                    );
                                    ui.selectable_label(is_current, name)
                                })
                                .inner;
                            if entry.clicked() {
                                *tint = color;
                                changed = true;
                            }
                        }
                    }
                });
        });
    (changed, response)
}

// The current-swatch highlight and the apply side's override gate read the
// same bit-for-bit comparison (`layer_actions::tint_matches`); a separate copy
// here could drift and make the popup highlight a colour the apply refuses to
// treat as current.

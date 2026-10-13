//! Stable centered modal surfaces shared by product-information and result
//! dialogs.
//!
//! The egui `Modal` container keeps its full-screen backdrop inside the same
//! measured `Area` as the card. That is a poor fit for content-sized dialogs:
//! the backdrop becomes part of the next sizing pass and can make the card
//! visibly resize forever. This module keeps the backdrop and card separate
//! while preserving `ModalResponse` close and Escape semantics.

use crate::ui::ui_theme;
use eframe::egui;

const BACKDROP_ALPHA: u8 = 88;

pub(crate) fn show_information_modal<T>(
    ctx: &egui::Context,
    id: egui::Id,
    default_size: egui::Vec2,
    close_label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    let viewport = ctx.content_rect();
    let bounds = viewport.shrink(16.0);
    let frame = ui_theme::window_frame(ui_theme::ThemeTokens::from_context(ctx))
        .inner_margin(egui::Margin::same(20));
    let preferred_size = default_size.min(bounds.size());
    let modal_layer = egui::LayerId::new(egui::Order::Foreground, id);
    let is_top_modal = ctx.memory_mut(|memory| {
        memory.set_modal_layer(modal_layer);
        memory.top_modal_layer() == Some(modal_layer)
    });
    let any_popup_open = egui::Popup::is_any_open(ctx);

    // Keep the backdrop out of the card's measured Area. Putting a full-screen
    // child next to content-sized rows makes egui feed the backdrop dimensions
    // back into the card on the next frame.
    let backdrop_id = id.with("backdrop");
    let backdrop_response = egui::Area::new(backdrop_id)
        .order(egui::Order::Foreground)
        .fixed_pos(viewport.min)
        .default_size(viewport.size())
        .constrain_to(viewport)
        .movable(false)
        .interactable(true)
        .sense(egui::Sense::hover())
        .show(ctx, |ui| {
            let mut backdrop = ui.new_child(
                egui::UiBuilder::new()
                    .sense(egui::Sense::click_and_drag())
                    .max_rect(viewport),
            );
            backdrop.set_min_size(viewport.size());
            ui.painter().rect_filled(
                viewport,
                0.0,
                egui::Color32::from_black_alpha(BACKDROP_ALPHA),
            );
            backdrop.response()
        })
        .inner;
    crate::ui::accessibility::button(&backdrop_response, close_label, true, None);

    // Retire an expanded cached card after a layout change. Its width follows
    // the dialog, never a scroll viewport or one of the two scene panes.
    // A naturally tall card must not force an invisible sizing pass forever.
    let needs_sizing_pass = ctx.memory(|memory| {
        memory
            .area_rect(id)
            .is_some_and(|rect| rect.width() > preferred_size.x + 4.0)
    });

    let mut area = egui::Area::new(id)
        .kind(egui::UiKind::Modal)
        .default_size(preferred_size)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .sense(egui::Sense::hover())
        .interactable(true)
        .movable(false)
        .constrain_to(bounds)
        .layout(egui::Layout::top_down(egui::Align::Min));
    if needs_sizing_pass {
        area = area.sizing_pass(true);
    }

    let card = area.show(ctx, |ui| {
        ui.scope_builder(
            egui::UiBuilder::new().sense(egui::Sense::click_and_drag()),
            |ui| {
                frame
                    .show(ui, |ui| {
                        // Dialog content sits on our explicit window surface,
                        // so plain labels must use the matching chrome ink
                        // even when the active button ink is tuned for an
                        // accent fill.
                        ui.visuals_mut().override_text_color =
                            Some(ui_theme::ThemeTokens::from_context(ctx).text());
                        ui.set_width((preferred_size.x - 40.0).max(0.0));
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        ui.style_mut().interaction.selectable_labels = false;
                        ui.style_mut().interaction.multi_widget_text_select = false;
                        add_contents(ui)
                    })
                    .inner
            },
        )
        .inner
    });

    egui::ModalResponse {
        response: card.response,
        backdrop_response,
        inner: card.inner,
        is_top_modal,
        any_popup_open,
    }
}

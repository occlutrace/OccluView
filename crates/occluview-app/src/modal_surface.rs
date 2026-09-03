//! Stable centered modal surfaces shared by product-information and result
//! dialogs.
//!
//! The egui `Modal` container keeps its full-screen backdrop inside the same
//! measured `Area` as the card. That is a poor fit for content-sized dialogs:
//! the backdrop becomes part of the next sizing pass and can make the card
//! visibly resize forever. This module keeps the backdrop and card separate
//! while preserving `ModalResponse` close and Escape semantics.

use crate::ui_theme;
use eframe::egui;

const BACKDROP_ALPHA: u8 = 48;

pub(crate) fn show_information_modal<T>(
    ctx: &egui::Context,
    id: egui::Id,
    default_size: egui::Vec2,
    add_contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    let bounds = ctx.content_rect().shrink(16.0);
    let frame = ui_theme::overlay_frame();
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
        .fixed_pos(bounds.min)
        .default_size(bounds.size())
        .constrain_to(bounds)
        .movable(false)
        .interactable(true)
        .sense(egui::Sense::click_and_drag())
        .show(ctx, |ui| {
            let mut backdrop = ui.new_child(
                egui::UiBuilder::new()
                    .sense(egui::Sense::click_and_drag())
                    .max_rect(bounds),
            );
            backdrop.set_min_size(bounds.size());
            ui.painter()
                .rect_filled(bounds, 0.0, egui::Color32::from_black_alpha(BACKDROP_ALPHA));
            backdrop.response()
        })
        .inner;

    // Only force a sizing pass when the available content rectangle changed
    // and the remembered card no longer fits. Repeating this every frame is
    // precisely the feedback loop the regression test protects against.
    let bounds_key = id.with("information-modal-bounds");
    let bounds_changed = ctx.data(|data| {
        data.get_temp::<egui::Rect>(bounds_key)
            .is_none_or(|previous| previous != bounds)
    });
    ctx.data_mut(|data| data.insert_temp(bounds_key, bounds));
    let needs_sizing_pass = bounds_changed
        && ctx.memory(|memory| {
            memory.area_rect(id).is_some_and(|rect| {
                !bounds.contains_rect(rect)
                    || rect.width() > preferred_size.x * 1.25
                    || rect.height() > preferred_size.y * 1.25
            })
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
            |ui| frame.show(ui, add_contents).inner,
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

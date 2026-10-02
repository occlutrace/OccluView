//! Layer drag: resolving the drop target, previewing it, and queueing the transfer.

use eframe::egui::{pos2, vec2, Rect};

use super::pointer::WorkspaceInputFrame;
use crate::app::workspace::commands::{
    LayerDragPayload, LayerDropTarget, LayerIds, SplitSide, TransferDestination, WorkspaceCommand,
};
use crate::app::{egui, OccluViewApp};

const EDGE_DROP_ZONE_WIDTH: f32 = 56.0;
/// A resolved edge target survives this far past its band, so a hand that
/// drifts a few pixels back inward does not make the highlight flash off.
const EDGE_DROP_ZONE_RELEASE_MARGIN: f32 = 40.0;

impl OccluViewApp {
    /// Resolve where a live layer drag would land. The preview and the release
    /// both call this, so the highlight can never name a destination the drop
    /// does not use.
    fn resolve_layer_drop_target(
        &self,
        ctx: &egui::Context,
        pointer: egui::Pos2,
        frame: &WorkspaceInputFrame<'_>,
        payload: &LayerDragPayload,
    ) -> Option<LayerDropTarget> {
        if let Some((key, rect)) = self
            .workspace
            .scene_tab_rects
            .iter()
            .find(|(key, rect)| *key != payload.source && rect.contains(pointer))
        {
            return Some(LayerDropTarget::SceneTab {
                key: *key,
                rect: *rect,
            });
        }
        // The footer's plus is an explicit create target with the same
        // right-side default as clicking it, and it takes priority over the
        // pane canvas below it.
        if self.workspace.scenes.len() < 2 {
            if let Some(rect) = self
                .workspace
                .scene_create_rect
                .filter(|rect| rect.contains(pointer))
            {
                return Some(LayerDropTarget::NewScene { rect });
            }
        }
        // Pane headers and the Layers panel float above the canvases, so a
        // pointer on them is not over a pane even when it is geometrically
        // inside one. Dropping back onto the source is a cancellation, which
        // is what `None` means here.
        if self.pointer_over_workspace_chrome(pointer, frame.panes)
            || self.pointer_over_layers_panel(ctx, pointer, frame.workspace_rect)
        {
            return None;
        }
        if let Some(pane) = frame
            .panes
            .iter()
            .find(|pane| pane.key != payload.source && pane.canvas.contains(pointer))
        {
            return Some(LayerDropTarget::Pane {
                key: pane.key,
                rect: pane.canvas,
            });
        }
        if self.workspace.scenes.len() >= 2 || !frame.workspace_rect.contains(pointer) {
            return None;
        }
        let band = |side: SplitSide| match side {
            SplitSide::Left => Rect::from_min_max(
                frame.workspace_rect.min,
                pos2(
                    frame.workspace_rect.left() + EDGE_DROP_ZONE_WIDTH,
                    frame.workspace_rect.bottom(),
                ),
            ),
            SplitSide::Right => Rect::from_min_max(
                pos2(
                    frame.workspace_rect.right() - EDGE_DROP_ZONE_WIDTH,
                    frame.workspace_rect.top(),
                ),
                frame.workspace_rect.max,
            ),
        };
        for side in [SplitSide::Left, SplitSide::Right] {
            let rect = band(side);
            if rect.contains(pointer) {
                return Some(LayerDropTarget::Edge { side, rect });
            }
        }
        match (
            self.workspace.layer_drag.is_some(),
            self.workspace.layer_drop_target,
        ) {
            (true, Some(LayerDropTarget::Edge { side, rect }))
                if rect.expand(EDGE_DROP_ZONE_RELEASE_MARGIN).contains(pointer) =>
            {
                Some(LayerDropTarget::Edge { side, rect })
            }
            _ => None,
        }
    }
    pub(super) fn finish_layer_drag(
        &mut self,
        ctx: &egui::Context,
        frame: &WorkspaceInputFrame<'_>,
    ) {
        if self.workspace.layer_drag.is_none() {
            return;
        }
        let (released, held) = ctx.input(|input| {
            (
                input.pointer.button_released(egui::PointerButton::Primary),
                input.pointer.primary_down(),
            )
        });
        if !released {
            // A drag whose primary button is no longer down can never be
            // completed: the window lost focus, another widget consumed the
            // release, or a different button armed the drag. Clear it instead
            // of tracking the pointer until the next primary release.
            if !held {
                self.workspace.layer_drag = None;
            }
            return;
        }
        if frame.modal_open || self.ui.modal_dialog_open() {
            return;
        }
        let Some(pointer) = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())
        }) else {
            // No pointer position on the release frame: keep the drag so the
            // next frame can resolve it. The dead-man above clears it if the
            // button is already up by then.
            return;
        };
        let Some(payload) = self.workspace.layer_drag.clone() else {
            return;
        };
        let target = self.resolve_layer_drop_target(ctx, pointer, frame, &payload);
        // The highlight is not cleared here: the preview keeps painting the last
        // resolved target and lets it fade out, which is what a release should
        // look like. Clearing it here made the mark disappear in one frame.
        self.workspace.layer_drag = None;
        match target {
            Some(LayerDropTarget::SceneTab { key, .. } | LayerDropTarget::Pane { key, .. }) => {
                self.queue_layer_transfer(payload, TransferDestination::Existing(key));
            }
            Some(LayerDropTarget::NewScene { .. }) => {
                self.create_scene_for_layer_transfer(payload, SplitSide::Right);
            }
            Some(LayerDropTarget::Edge { side, .. }) => {
                self.create_scene_for_layer_transfer(payload, side);
            }
            None => {}
        }
    }
    fn create_scene_for_layer_transfer(&mut self, payload: LayerDragPayload, side: SplitSide) {
        let Ok(scene) = self.workspace.ids.allocate_scene() else {
            return;
        };
        let Ok(pane) = self.workspace.ids.allocate_pane_id() else {
            return;
        };
        self.queue_layer_transfer(
            payload,
            TransferDestination::CreateBeside { scene, pane, side },
        );
    }
    fn queue_layer_transfer(
        &mut self,
        payload: LayerDragPayload,
        destination: TransferDestination,
    ) {
        self.workspace
            .commands
            .push_back(WorkspaceCommand::Transfer {
                source: payload.source,
                destination,
                layers: LayerIds::one(payload.layer),
            });
    }
    pub(super) fn show_layer_drag_preview(
        &mut self,
        ui: &mut egui::Ui,
        frame: &WorkspaceInputFrame<'_>,
    ) {
        let ctx = ui.ctx().clone();
        let payload = self.workspace.layer_drag.clone();
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let target = match (&payload, pointer) {
            (Some(payload), Some(pointer)) if !frame.modal_open && !self.ui.modal_dialog_open() => {
                self.resolve_layer_drop_target(&ctx, pointer, frame, payload)
            }
            _ => None,
        };
        // One id drives the fade in and out, so a target that appears and
        // clears within a frame still reads as a transition rather than a
        // flash. The last painted rectangle outlives its target for as long as
        // the fade takes.
        let alpha = ctx.animate_bool_with_time(
            egui::Id::new("layer-drop-highlight"),
            target.is_some(),
            0.14,
        );
        if target.is_some() || alpha > 0.002 {
            ctx.request_repaint();
        }
        let painted = target.or(self.workspace.layer_drop_target);
        self.workspace.layer_drop_target = if target.is_none() && alpha <= 0.002 {
            None
        } else {
            painted
        };
        if let Some(painted) = painted {
            if alpha > 0.002 {
                let accent = crate::ui::ui_theme::accent();
                // A pane is a whole canvas, so tinting it reads as burning the
                // view the operator is about to use. Mark it with its border
                // instead and keep the tint for the narrow bands.
                let (fill_share, stroke_width) = match painted {
                    LayerDropTarget::Pane { .. } => (0.0, 2.0),
                    _ => (0.12, 1.0),
                };
                let rect = painted.rect();
                if fill_share > 0.0 {
                    ui.painter()
                        .rect_filled(rect, 3.0, accent.gamma_multiply(fill_share * alpha));
                }
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    egui::Stroke::new(stroke_width, accent.gamma_multiply(alpha)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let (Some(payload), Some(pointer)) = (payload, pointer) else {
            return;
        };
        if let Some(target) = target {
            self.paint_layer_drop_label(ui.painter(), target.rect(), pointer, frame.workspace_rect);
        }
        ctx.set_cursor_icon(if target.is_some() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        Self::paint_layer_drag_ghost(ui.painter(), pointer, &payload, target.is_some());
    }
    /// Translucent chip under the pointer for the layer being dragged. It dims
    /// when the pointer is not over a drop target, so the outcome of the
    /// release is visible before the button comes up.
    fn paint_layer_drag_ghost(
        painter: &egui::Painter,
        at: egui::Pos2,
        payload: &LayerDragPayload,
        accepted: bool,
    ) {
        let galley = painter.layout_no_wrap(
            payload.label.clone(),
            egui::FontId::proportional(12.0),
            crate::ui::ui_theme::text(),
        );
        let rect = Rect::from_min_size(at + vec2(12.0, 14.0), galley.size() + vec2(22.0, 12.0));
        let tint = crate::layers_overlay::color32_from_tint(payload.tint);
        let (fill, stroke) = if accepted {
            (tint.gamma_multiply(0.85), crate::ui::ui_theme::accent())
        } else {
            (tint.gamma_multiply(0.35), crate::ui::ui_theme::hairline())
        };
        painter.rect_filled(rect, 6.0, fill);
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            rect.min + vec2(11.0, 6.0),
            galley,
            crate::ui::ui_theme::text(),
        );
    }
    fn paint_layer_drop_label(
        &self,
        painter: &egui::Painter,
        target: Rect,
        drop_position: egui::Pos2,
        workspace_rect: Rect,
    ) {
        if target.height() <= 60.0 {
            return;
        }
        let label = self.ui.locale.tr(if self.workspace.scenes.len() < 2 {
            if drop_position.x < workspace_rect.center().x {
                crate::i18n::message_id!("workspace-new-left")
            } else {
                crate::i18n::message_id!("workspace-new-right")
            }
        } else {
            crate::i18n::message_id!("workspace-move-layer")
        });
        let galley = painter.layout_no_wrap(
            label,
            egui::FontId::proportional(14.0),
            crate::ui::ui_theme::text(),
        );
        // A drop band is narrower than this pill, so centring the pill on the
        // band would hang it off the workspace, where the panel clips it. Shift
        // it back inside before painting.
        let wanted = Rect::from_center_size(target.center(), galley.size() + vec2(24.0, 16.0));
        let shift = vec2(
            (workspace_rect.left() - wanted.left()).max(0.0)
                - (wanted.right() - workspace_rect.right()).max(0.0),
            (workspace_rect.top() - wanted.top()).max(0.0)
                - (wanted.bottom() - workspace_rect.bottom()).max(0.0),
        );
        let label_rect = wanted.translate(shift);
        painter.rect_filled(label_rect, 6.0, crate::ui::ui_theme::panel_fill());
        painter.galley(
            label_rect.min + vec2(12.0, 8.0),
            galley,
            crate::ui::ui_theme::text(),
        );
    }
}

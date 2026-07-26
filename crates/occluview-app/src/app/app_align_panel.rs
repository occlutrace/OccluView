//! What the Align Scans windows asked for, and what happens when they ask.
//!
//! Split from `app_align` because it answers a different question: that module
//! routes viewport clicks and worker jobs, this one owns the two windows — the
//! panel and the Brush tool — and the actions they return.

use eframe::egui;

use super::OccluViewApp;
use crate::align_worker::AlignWorker;

impl OccluViewApp {
    /// A stationary right-click takes the last point back.
    ///
    /// The operator asked for this by name: placing the first half of a pair
    /// and then having to reach for a button to undo it is a trip away from
    /// the geometry they are looking at. A right-click that has nothing to take
    /// back is left alone, so the scene menu still opens on empty space.
    pub(super) fn handle_align_undo_click(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        let (pressed, down, motion) = ctx.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Secondary),
                input.pointer.button_down(egui::PointerButton::Secondary),
                input.pointer.motion().unwrap_or(input.pointer.delta()),
            )
        });
        // Tracked here as well as in the camera path, because a frame this
        // method consumes never reaches the camera path at all.
        if pressed {
            self.viewport_secondary_gesture_moved_since_press = false;
        }
        if down && motion.length_sq() > f32::EPSILON {
            self.viewport_secondary_gesture_moved_since_press = true;
        }
        if !response.secondary_clicked() || self.viewport_secondary_gesture_moved_since_press {
            return false;
        }
        if !self.take_align_arrow_back() {
            return false;
        }
        ctx.request_repaint();
        true
    }

    /// Draw the panel and the Brush tool window, then run what they asked for.
    pub(super) fn show_align_panel(&mut self, ctx: &egui::Context, viewport_rect: egui::Rect) {
        let busy = self.align_worker.as_ref().is_some_and(AlignWorker::is_busy);
        let mut settings = self.align_settings;
        let mut constraint = self.align_constraint;
        let mut brush = self.align_brush;
        let mut tab = self.align_tab;
        let mut excluding = brush.is_armed();
        let was_excluding = excluding;
        let mut drop_pending = false;
        let moved = self.align_session_moved();
        let action = crate::align_panel::show(
            ctx,
            viewport_rect,
            crate::align_panel::AlignPanelView {
                tool: &self.align,
                settings: &mut settings,
                status: self.align_status.as_deref(),
                stats: self.align_stats,
                busy,
                moved,
                can_undo: self.edit_mode.undo_layer_id().is_some(),
                can_redo: self.edit_mode.redo_layer_id().is_some(),
                constraint: &mut constraint,
                excluding: &mut excluding,
                drop_pending: &mut drop_pending,
                tab: &mut tab,
            },
        );

        let mut mask_command = None;
        if excluding {
            match crate::align_panel_brush::show(
                ctx,
                viewport_rect,
                &mut brush,
                self.align_marked_fraction(),
                !busy,
            ) {
                Some(crate::align_panel_brush::BrushPanelAction::Mask(command)) => {
                    mask_command = Some(command);
                }
                Some(crate::align_panel_brush::BrushPanelAction::Close) => excluding = false,
                None => {}
            }
        }
        brush.set_armed(excluding);

        if drop_pending {
            self.align.back();
            self.align_status = Some("Half-placed arrow dropped".into());
        }
        self.align_settings = settings;
        self.align_constraint = constraint;
        self.align_brush = brush;
        self.align_tab = tab;
        // Opening and closing the brush changes what is on the surface: the
        // markings go up, and the scan's own colours come back.
        if was_excluding != excluding {
            self.refresh_align_region_preview();
        }
        if let Some(command) = mask_command {
            self.apply_align_mask_command(command);
        }

        match action {
            Some(crate::align_panel::AlignPanelAction::Align) => self.run_align_fit(),
            Some(crate::align_panel::AlignPanelAction::Refine) => self.run_align_refine(),
            Some(crate::align_panel::AlignPanelAction::Measure) => self.run_align_measure(),
            Some(crate::align_panel::AlignPanelAction::HideMap) => self.clear_deviation_overlay(),
            Some(crate::align_panel::AlignPanelAction::Back) => {
                self.take_align_arrow_back();
            }
            Some(crate::align_panel::AlignPanelAction::Undo) => {
                self.apply_history_navigation_now(false, ctx);
                self.invalidate_deviation_map("Stepped back");
            }
            Some(crate::align_panel::AlignPanelAction::Redo) => {
                self.apply_history_navigation_now(true, ctx);
                self.invalidate_deviation_map("Stepped forward");
            }
            Some(crate::align_panel::AlignPanelAction::Cancel) => self.cancel_align_session(ctx),
            Some(crate::align_panel::AlignPanelAction::Done) => self.finish_align_session(ctx),
            None => {}
        }
    }

    /// exocad's "Back": drop the half-placed point, else the last whole arrow.
    fn take_align_arrow_back(&mut self) -> bool {
        if !self.align.back() {
            return false;
        }
        self.align_rejected.clear();
        self.align_status = Some(match self.align.pairs().len() {
            0 if self.align.pending().is_none() => {
                "Click alternating points at the same positions on the two meshes".to_owned()
            }
            remaining => format!("Arrow removed — {remaining} left"),
        });
        true
    }
}

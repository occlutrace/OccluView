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
        let busy = self.align.worker.as_ref().is_some_and(AlignWorker::is_busy);
        let mut settings = self.align.settings;
        let mut constraint = self.align.constraint;
        let mut brush = self.align.brush;
        let mut tab = self.align.tab;
        let mut excluding = brush.is_armed();
        let was_excluding = excluding;
        let mut drop_pending = false;
        let moved = self.align_session_moved();
        let action = crate::align_panel::show(
            ctx,
            viewport_rect,
            crate::align_panel::AlignPanelView {
                tool: &self.align.tool,
                settings: &mut settings,
                status: self.align.status.as_deref(),
                stats: self.align.stats,
                roles: self.align_roles(),
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
            self.align.tool.back();
            self.align.status = Some("Half-placed arrow dropped".into());
        }
        self.align.settings = settings;
        self.align.constraint = constraint;
        self.align.brush = brush;
        let tab_changed = self.align.tab != tab;
        self.align.tab = tab;
        // Opening and closing the brush changes what is on the surface: the
        // markings go up, and the scan's own colours come back.
        if was_excluding != excluding {
            self.refresh_align_region_preview();
        }
        if tab_changed {
            self.settle_align_tab_change();
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
            Some(crate::align_panel::AlignPanelAction::SwapRoles) => self.swap_align_roles(),
            Some(crate::align_panel::AlignPanelAction::Clear) => self.clear_align_pair(),
            // The invalidation lives inside the navigation itself, so the
            // Ctrl+Z shortcut gets it too.
            Some(crate::align_panel::AlignPanelAction::Undo) => {
                self.apply_history_navigation_now(false, ctx);
            }
            Some(crate::align_panel::AlignPanelAction::Redo) => {
                self.apply_history_navigation_now(true, ctx);
            }
            Some(crate::align_panel::AlignPanelAction::Cancel) => self.cancel_align_session(ctx),
            Some(crate::align_panel::AlignPanelAction::Done) => self.finish_align_session(ctx),
            None => {}
        }
    }

    /// Which scan the fit will move, named the way the operator named the files.
    fn align_roles(&self) -> Option<crate::align_panel_roles::AlignRoles> {
        Some(crate::align_panel_roles::AlignRoles {
            moving: self.layer_display_name(self.align.tool.moving_layer()?)?,
            fixed: self.layer_display_name(self.align.tool.fixed_layer()?)?,
            implied: self.align.tool.roles_are_implied(),
        })
    }

    /// Turn the pair around, and take everything that described the old
    /// direction down with it.
    fn swap_align_roles(&mut self) {
        if !self.align.tool.swap_roles() {
            return;
        }
        // The markings belong to surfaces, not to roles.
        self.align.markings.swap_sides();
        // A map is a measurement of one scan against the other, in that order.
        self.forget_align_fit("Pair turned around");
        let named = self.align_roles().map_or_else(
            || "Pair turned around".to_owned(),
            |roles| format!("{} moves now, {} stays put", roles.moving, roles.fixed),
        );
        self.align.status = Some(named);
    }

    /// Drop the pair so a different two scans can be picked, without closing the
    /// tool and without moving anything back.
    fn clear_align_pair(&mut self) {
        self.align.tool.clear();
        self.clear_align_mask();
        self.forget_align_fit("Pair cleared");
        self.align.status = Some("Click a point on the scan that should move".into());
    }

    /// The operator's dental CAD "Back": drop the half-placed point, else the
    /// last whole arrow.
    fn take_align_arrow_back(&mut self) -> bool {
        if !self.align.tool.back() {
            return false;
        }
        self.align.rejected.clear();
        self.align.status = Some(match self.align.tool.pairs().len() {
            0 if self.align.tool.pending().is_none() => {
                "Click alternating points at the same positions on the two meshes".to_owned()
            }
            remaining => format!("Arrow removed — {remaining} left"),
        });
        true
    }
}

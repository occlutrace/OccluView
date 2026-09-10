//! What the Align Scans windows asked for, and what happens when they ask.
//!
//! Split from `app_align` because it answers a different question: that module
//! routes viewport clicks and worker jobs, this one owns the two windows — the
//! panel and the Brush tool — and the actions they return.

use eframe::egui;

use super::OccluViewApp;
use crate::align_worker::{matching_inputs_changed, AlignWorker};

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
            self.ui.viewport_secondary_gesture_moved_since_press = false;
        }
        if down && motion.length_sq() > f32::EPSILON {
            self.ui.viewport_secondary_gesture_moved_since_press = true;
        }
        if !response.secondary_clicked() || self.ui.viewport_secondary_gesture_moved_since_press {
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
        let busy = self
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::is_busy);
        let previous_settings = self.tools.align.settings;
        let mut settings = previous_settings;
        let mut constraint = self.tools.align.constraint;
        let mut brush = self.tools.align.brush;
        let mut tab = self.tools.align.tab;
        let mut excluding = brush.is_armed();
        let was_excluding = excluding;
        let mut drop_pending = false;
        let moved = self.align_session_moved();
        let action = crate::align_panel::show(
            ctx,
            viewport_rect,
            crate::align_panel::AlignPanelView {
                tool: &self.tools.align.tool,
                settings: &mut settings,
                status: self.tools.align.status.as_deref(),
                refined_match_ready: self.tools.align.refined_match_ready,
                roles: self.align_roles(),
                busy,
                moved,
                can_undo: self.document.edit_mode.undo_layer_id().is_some(),
                can_redo: self.document.edit_mode.redo_layer_id().is_some(),
                constraint: &mut constraint,
                excluding: &mut excluding,
                drop_pending: &mut drop_pending,
                tab: &mut tab,
            },
            &self.ui.locale,
        );

        let mut mask_command = None;
        if excluding {
            match crate::align_panel_brush::show(
                ctx,
                viewport_rect,
                &mut brush,
                !busy,
                &self.ui.locale,
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
            self.tools.align.tool.back();
            self.tools.align.status = Some(self.ui.locale.tr("align-status-half-dropped"));
        }
        self.tools.align.settings = settings;
        self.tools.align.constraint = constraint;
        self.tools.align.brush = brush;
        if matching_inputs_changed(previous_settings, settings)
            && self.tools.align.refined_match_ready
        {
            self.forget_align_fit(&self.ui.locale.tr("align-status-settings-changed"));
        }
        let tab_changed = self.tools.align.tab != tab;
        self.tools.align.tab = tab;
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
            Some(crate::align_panel::AlignPanelAction::HideMap) => {
                self.tools.align.settings.show_deviation = false;
                self.abandon_align_jobs();
                self.clear_deviation_overlay();
            }
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
            moving: self.layer_display_name(self.tools.align.tool.moving_layer()?)?,
            fixed: self.layer_display_name(self.tools.align.tool.fixed_layer()?)?,
            implied: self.tools.align.tool.roles_are_implied(),
        })
    }

    /// Turn the pair around, and take everything that described the old
    /// direction down with it.
    fn swap_align_roles(&mut self) {
        if !self.tools.align.tool.swap_roles() {
            return;
        }
        // The markings belong to surfaces, not to roles.
        self.tools.align.markings.swap_sides();
        // A map is a measurement of one scan against the other, in that order.
        self.forget_align_fit(&self.ui.locale.tr("align-status-turned"));
        let named = self.align_roles().map_or_else(
            || self.ui.locale.tr("align-status-turned"),
            |roles| {
                self.ui.locale.tr_with(
                    "align-roles-swapped",
                    &[
                        ("moving", roles.moving.as_str()),
                        ("fixed", roles.fixed.as_str()),
                    ],
                )
            },
        );
        self.tools.align.status = Some(named);
    }

    /// Drop the pair so a different two scans can be picked, without closing the
    /// tool and without moving anything back.
    fn clear_align_pair(&mut self) {
        self.tools.align.tool.clear();
        self.clear_align_mask();
        self.forget_align_fit(&self.ui.locale.tr("align-status-cleared"));
        self.tools.align.status = Some(self.ui.locale.tr("align-status-click-moving"));
    }

    /// The operator's dental CAD "Back": drop the half-placed point, else the
    /// last whole arrow.
    fn take_align_arrow_back(&mut self) -> bool {
        if !self.tools.align.tool.back() {
            return false;
        }
        self.tools.align.rejected.clear();
        self.tools.align.status = Some(match self.tools.align.tool.pairs().len() {
            0 if self.tools.align.tool.pending().is_none() => {
                self.ui.locale.tr("align-status-click-alternate")
            }
            remaining => self
                .ui
                .locale
                .tr_plural("align-arrow-removed", &[], &[("n", remaining)]),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    /// The source before this module. Keeping this contract on the production
    /// half prevents the test from satisfying itself with its own assertion.
    fn production() -> &'static str {
        let source = crate::primary_ui_tests::production_source(include_str!("app_align_panel.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    #[test]
    fn optimizer_setting_changes_drop_the_refined_authority() {
        let source = production();
        assert!(
            source.contains("matching_inputs_changed"),
            "the panel must compare optimizer inputs after editing them"
        );
        assert!(
            source.contains("self.forget_align_fit"),
            "a changed optimizer input must remove the old refined match"
        );
    }
}

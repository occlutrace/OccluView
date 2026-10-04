//! What the Align Scans windows asked for, and what happens when they ask.
//!
//! `align` routes viewport clicks and worker jobs; this module owns the two
//! windows — the panel and the Brush tool — and the actions they return.

use eframe::egui;

use super::super::SceneContext;
use crate::align::align_panel::{AlignPanelAction, AlignTab};
use crate::align::align_worker::{matching_inputs_changed, AlignWorker};

fn heatmap_is_authorized(tab: AlignTab, fit_ready: bool) -> bool {
    tab == AlignTab::Automatically && fit_ready
}

fn action_after_tab_change(
    action: Option<AlignPanelAction>,
    tab_changed: bool,
) -> Option<AlignPanelAction> {
    (!tab_changed).then_some(action).flatten()
}

fn align_worker_needs_ui_poll(worker: Option<&AlignWorker>) -> bool {
    worker.is_some_and(|worker| worker.is_busy() || worker.has_pending_output())
}

impl SceneContext<'_> {
    /// A stationary right-click takes the last point back.
    ///
    /// Undoing a half-placed pair stays on the geometry the operator is looking
    /// at instead of requiring a trip to a button. A right-click that has
    /// nothing to take back is left alone, so the scene menu still opens on
    /// empty space.
    pub(in crate::app) fn handle_align_undo_click(
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
            self.scene_ui.viewport_secondary_gesture_moved_since_press = false;
        }
        if down && motion.length_sq() > f32::EPSILON {
            self.scene_ui.viewport_secondary_gesture_moved_since_press = true;
        }
        if !response.secondary_clicked()
            || self.scene_ui.viewport_secondary_gesture_moved_since_press
        {
            return false;
        }
        if !self.take_align_arrow_back() {
            return false;
        }
        ctx.request_repaint();
        true
    }

    /// Draw the panel and the Brush tool window, then run what they asked for.
    // Keep panel state capture, window rendering, and action application in one
    // ordered UI transaction; changing that order can submit stale settings.
    #[expect(clippy::too_many_lines)]
    pub(in crate::app) fn show_align_panel(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
    ) {
        let busy = self
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::is_busy);
        if align_worker_needs_ui_poll(self.tools.align.worker.as_ref()) {
            // Completion is drained on this UI path. Keep repainting while the
            // worker is active and if it publishes just after this frame's
            // empty drain, so results do not wait for another operator input.
            ctx.request_repaint();
        }
        let worker_failed = self
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::has_failed);
        let previous_settings = self.tools.align.settings;
        let mut settings = previous_settings;
        let mut constraint = self.tools.align.constraint;
        let mut brush = self.tools.align.brush;
        let mut tab = self.tools.align.tab;
        let mut excluding = brush.is_armed();
        let was_excluding = excluding;
        let mut drop_pending = false;
        let moved = self.align_session_moved();
        let panel_roles = self.align_roles();
        let brush_roles = self.align_roles();
        let action = crate::align::align_panel::show(
            ctx,
            viewport_rect,
            egui::Id::new(self.scene_key),
            crate::align::align_panel::AlignPanelView {
                tool: &self.tools.align.tool,
                layer_count: self
                    .document
                    .scene
                    .as_ref()
                    .map_or(0, |scene| scene.meshes().len()),
                settings: &mut settings,
                status: self.tools.align.status.as_deref(),
                fit_unsure: self.tools.align.fitted.as_ref().is_some_and(|fitted| {
                    matches!(
                        fitted.confidence,
                        occluview_align::Confidence::Ambiguous | occluview_align::Confidence::Weak
                    )
                }),
                fit_ready: heatmap_is_authorized(
                    self.tools.align.tab,
                    self.alignment_measurement_ready(),
                ),
                roles: panel_roles,
                busy,
                worker_failed,
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
            match crate::align::align_panel_brush::show(
                ctx,
                egui::Id::new(self.scene_key),
                crate::align::align_panel_brush::BrushPanelView {
                    viewport_rect,
                    brush: &mut brush,
                    roles: brush_roles.as_ref(),
                    enabled: !busy,
                    locale: &self.ui.locale,
                },
            ) {
                Some(crate::align::align_panel_brush::BrushPanelAction::Mask(command)) => {
                    mask_command = Some(command);
                }
                Some(crate::align::align_panel_brush::BrushPanelAction::Close) => excluding = false,
                None => {}
            }
        }
        brush.set_armed(excluding);

        if drop_pending {
            self.tools.align.tool.back();
            self.tools.align.status = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("align-status-half-dropped")),
            );
        }
        self.tools.align.settings = settings;
        self.tools.align.constraint = constraint;
        self.tools.align.brush = brush;
        if matching_inputs_changed(previous_settings, settings) {
            // A running job holds the settings snapshot it was submitted with,
            // so its generation is dead the moment one of these inputs moves.
            // Abandoning is therefore unconditional: gating it on the refined
            // claim would let an orientation edit land inside a running Best
            // fit, and that job would then arm the claim from inputs the
            // operator has already changed. The user-facing "measure again"
            // notice still waits for a fit that actually existed.
            if self.tools.align.fitted.is_some() {
                self.forget_align_fit(
                    &self
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("align-status-settings-changed")),
                );
            } else {
                self.abandon_align_jobs();
            }
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

        match action_after_tab_change(action, tab_changed) {
            Some(AlignPanelAction::Align) => self.run_align_fit(),
            Some(AlignPanelAction::Refine) => self.run_align_refine(),
            Some(AlignPanelAction::Measure) => self.run_align_measure(),
            Some(AlignPanelAction::HideMap) => {
                self.tools.align.settings.show_deviation = false;
                // Only a measurement can be in flight here: the toggle is
                // disabled while a fit runs. The fit itself stays valid, so
                // the map can be switched back on without matching again.
                self.abandon_align_jobs();
                self.clear_deviation_overlay();
            }
            Some(AlignPanelAction::Back) => {
                self.take_align_arrow_back();
            }
            Some(AlignPanelAction::SwapRoles) => self.swap_align_roles(),
            Some(AlignPanelAction::Clear) => self.clear_align_pair(),
            // The invalidation lives inside the navigation itself, so the
            // Ctrl+Z shortcut gets it too.
            Some(AlignPanelAction::Undo) => {
                self.apply_history_navigation_now(false, ctx);
            }
            Some(AlignPanelAction::Redo) => {
                self.apply_history_navigation_now(true, ctx);
            }
            Some(AlignPanelAction::Cancel) => self.cancel_align_session(ctx),
            Some(AlignPanelAction::Done) => self.finish_align_session(ctx),
            None => {}
        }
    }

    /// Which scan the fit will move, named the way the operator named the files.
    pub(in crate::app) fn align_roles(
        &self,
    ) -> Option<crate::align::align_panel_roles::AlignRoles> {
        Some(crate::align::align_panel_roles::AlignRoles {
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
        self.adopt_swapped_roles(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("align-status-turned")),
        );
        let named = self.align_roles().map_or_else(
            || {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("align-status-turned"))
            },
            |roles| {
                self.ui.locale.tr_with(
                    crate::i18n::message_id!("align-roles-swapped"),
                    &[
                        ("moving", roles.moving.as_str()),
                        ("fixed", roles.fixed.as_str()),
                    ],
                )
            },
        );
        self.tools.align.status = Some(named);
    }

    /// Apply everything a moving-to-fixed swap owes.
    ///
    /// Two callers reach it: the panel button, which is the operator asking for
    /// the swap, and the first point of a pair, which can quietly contradict
    /// the arm-time guess. The markings belong to surfaces rather than roles,
    /// and the Brush selects a physical surface, so both follow the swap. A map
    /// is a measurement of one scan against the other in a particular order, so
    /// it does not survive.
    pub(in crate::app) fn adopt_swapped_roles(&mut self, reason: String) {
        self.tools.align.markings.swap_sides();
        self.tools.align.brush.swap_target();
        self.forget_align_fit(&reason);
    }

    /// Drop the pair so a different two scans can be picked, without closing the
    /// tool and without moving anything back.
    fn clear_align_pair(&mut self) {
        self.tools.align.tool.clear();
        self.clear_align_mask();
        self.tools.align.brush.reset_target();
        self.forget_align_fit(
            &self
                .ui
                .locale
                .tr(crate::i18n::message_id!("align-status-cleared")),
        );
        self.tools.align.status = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("align-status-click-moving")),
        );
    }

    /// The operator's dental CAD "Back": drop the half-placed point, else the
    /// last whole arrow.
    fn take_align_arrow_back(&mut self) -> bool {
        if !self.tools.align.tool.back() {
            return false;
        }
        self.abandon_align_jobs();
        self.tools.align.rejected.clear();
        self.tools.align.status = Some(match self.tools.align.tool.pairs().len() {
            0 if self.tools.align.tool.pending().is_none() => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("align-status-click-alternate")),
            remaining => self.ui.locale.tr_plural(
                crate::i18n::message_id!("align-arrow-removed"),
                &[],
                &[("n", remaining)],
            ),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{action_after_tab_change, align_worker_needs_ui_poll, heatmap_is_authorized};
    use crate::align::align_panel::{AlignPanelAction, AlignTab};
    use crate::align::align_worker::{AlignFailure, AlignOutcome, AlignWorker};
    use eframe::egui;
    use std::sync::mpsc;

    #[test]
    fn manual_tab_never_authorizes_a_heatmap_from_stale_readiness() {
        assert!(!heatmap_is_authorized(AlignTab::Manually, true));
        assert!(heatmap_is_authorized(AlignTab::Automatically, true));
        assert!(!heatmap_is_authorized(AlignTab::Automatically, false));
    }

    #[test]
    fn a_tab_switch_discards_the_action_collected_for_the_previous_tab() {
        assert_eq!(
            action_after_tab_change(Some(AlignPanelAction::Refine), true),
            None
        );
        assert_eq!(
            action_after_tab_change(Some(AlignPanelAction::Measure), true),
            None
        );
        assert_eq!(
            action_after_tab_change(Some(AlignPanelAction::Refine), false),
            Some(AlignPanelAction::Refine)
        );
    }

    #[test]
    fn removing_an_arrow_discards_the_fit_that_used_it() -> anyhow::Result<()> {
        use crate::align::align_tool::AlignPoint;
        use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
        use glam::{Affine3A, DQuat, DVec3, Vec3};
        use occluview_align::Rigid;

        let mut app = test_app("align-back-obsolete-fit");
        let mut scene = named_scene("moving", 0.0);
        let moving = scene.meshes()[0].id();
        let fixed = push_named_layer(&mut scene, "fixed", 5.0);
        app.workspace.scenes[0].document.scene = Some(scene.into());
        let ctx = app.ui.repaint_ctx.clone();
        let mut scene = app
            .active_context()
            .ok_or_else(|| anyhow::anyhow!("live scene"))?;
        scene.tools.align.tool.arm();
        for local in [Vec3::ZERO, Vec3::Y] {
            for layer in [moving, fixed] {
                scene.tools.align.tool.click(AlignPoint {
                    layer,
                    local,
                    normal: Vec3::Z,
                });
            }
        }
        assert!(scene.tools.align.tool.can_align());
        let generation = scene.align_worker_mut().generation();
        scene.tools.align.pending_fit = scene.current_fit_key();
        assert!(scene.take_align_arrow_back());
        assert_eq!(scene.tools.align.tool.pairs().len(), 1);
        scene.align_worker_mut().publish_for_tests(
            generation,
            AlignOutcome::Aligned {
                correction: Rigid::new(DQuat::IDENTITY, DVec3::X),
                rejected: Vec::new(),
            },
        );
        scene.drain_align_worker(&ctx);

        assert_eq!(
            scene
                .document
                .scene
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("scene"))?
                .meshes()[0]
                .transform,
            Affine3A::IDENTITY,
            "a result based on the removed arrow cannot move the scan"
        );
        assert_eq!(scene.document.edit_mode.undo_len(), 0);
        Ok(())
    }

    /// The worker can publish after this frame already drained an empty queue.
    /// Such a result is no longer computationally busy, but it still needs one
    /// UI frame or it will sit unapplied until the operator moves the mouse.
    #[test]
    fn a_pending_completion_requests_the_idle_ui_poll() {
        let worker = AlignWorker::spawn();
        assert!(!align_worker_needs_ui_poll(Some(&worker)));
        let generation = worker.generation();
        worker.publish_for_tests(
            generation,
            AlignOutcome::Failed {
                rejection: AlignFailure::FixedSurfaceMissing,
            },
        );
        assert!(!worker.is_busy(), "the computation itself has completed");

        let ctx = egui::Context::default();
        let (sender, receiver) = mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = sender.send(());
        });
        if align_worker_needs_ui_poll(Some(&worker)) {
            ctx.request_repaint();
        }
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok(),
            "an undrained result must keep the idle egui loop alive once"
        );
    }
}

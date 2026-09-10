//! What comes back from the align worker, and what it is allowed to change.
//!
//! Applies completed alignment jobs and invalidates results whose input scene
//! has changed.

use eframe::egui;
use occluview_align::{FitRejection, Rigid};

use super::OccluViewApp;
use crate::align_worker::{AlignCompletion, AlignFailure, AlignOutcome, AlignWorker};
use crate::edit_mode::EditModeCommand;

impl OccluViewApp {
    /// Drain finished jobs and apply them.
    pub(super) fn drain_align_worker(&mut self, ctx: &egui::Context) {
        let worker_failed = self
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::has_failed);
        if worker_failed {
            // A worker failure is terminal for this session. Do not leave a
            // map claiming that the last pose was refined when no future
            // measurement can validate it. Region markings remain intact:
            // they are operator input, not worker output.
            self.tools.align.refined_match_ready = false;
            self.tools.align.settings.show_deviation = false;
            if self.tools.align.overlay == super::app_align_display::AlignOverlay::Map {
                self.clear_deviation_overlay();
            }
            self.tools.align.status = Some(self.ui.locale.tr("align-status-worker-unavailable"));
            ctx.request_repaint();
            return;
        }
        let Some(worker) = self.tools.align.worker.as_ref() else {
            return;
        };
        let completions: Vec<AlignCompletion> = worker.drain();
        if completions.is_empty() {
            return;
        }
        for completion in completions {
            // Re-read the generation for each completion because applying one
            // result can invalidate the remaining jobs.
            let current = self
                .tools
                .align
                .worker
                .as_ref()
                .map_or(completion.generation, AlignWorker::generation);
            if completion.generation != current {
                continue;
            }
            self.apply_align_outcome(completion, ctx);
        }
        ctx.request_repaint();
    }

    /// Apply one finished job.
    fn apply_align_outcome(&mut self, completion: AlignCompletion, ctx: &egui::Context) {
        match completion.outcome {
            AlignOutcome::Aligned { pose, rejected } => {
                if !self.commit_align_pose(pose) {
                    self.tools.align.status = Some(self.ui.locale.tr("align-status-pose-refused"));
                    return;
                }
                // A point fit changes the pose and invalidates any previous
                // map; refinement performs the next measurement.
                self.forget_align_fit(&self.ui.locale.tr("align-status-aligned-points"));
                self.tools.align.rejected = rejected;
                self.tools.align.status = Some(self.ui.locale.tr("align-status-aligned"));
            }
            AlignOutcome::Refined { pose } => {
                if !self.commit_align_pose(pose) {
                    self.tools.align.status = Some(self.ui.locale.tr("align-status-pose-refused"));
                    return;
                }
                // `commit_align_pose` invalidates derived alignment state while
                // rebuilding the scene. Mark readiness only after that commit,
                // so the map can describe the pose that actually landed.
                self.tools.align.refined_match_ready = true;
                self.tools.align.settings.show_deviation = true;
                self.tools.align.status = Some(self.ui.locale.tr("align-status-refined"));
                self.measure_if_shown();
            }
            AlignOutcome::Measured {
                colors,
                stats,
                seen,
                scale_mm,
            } => {
                self.apply_measured_outcome(colors, stats, seen, scale_mm);
            }
            AlignOutcome::Failed { rejection } => {
                let (key, a, b) = align_failure_parts(rejection);
                self.tools.align.status = Some(
                    self.ui
                        .locale
                        .tr_with(key, &[("a", a.as_str()), ("b", b.as_str())]),
                );
            }
        }
        ctx.request_repaint();
    }

    /// Apply one landed measurement: adopt its display scale, paint the
    /// deviation colours, and report the summary. Split from
    /// `apply_align_outcome` so each outcome arm stays reviewable.
    fn apply_measured_outcome(
        &mut self,
        colors: Vec<[u8; 4]>,
        stats: occluview_align::DeviationStats,
        _seen: Option<occluview_align::Observability>,
        scale_mm: f64,
    ) {
        // A measurement can finish after the operator hides the map or after
        // the pose/mask that authorized it became stale. Generation normally
        // drops that completion, but this boundary also owns the invariant so
        // no late result can resurrect a hidden or unrefined overlay.
        if !self.tools.align.refined_match_ready || !self.tools.align.settings.show_deviation {
            return;
        }
        // The brush owns the per-vertex colour channel while it is open.
        // (The caller repaints after every outcome, so no repaint here.)
        if self.tools.align.brush.is_armed() {
            self.tools.align.status = Some(self.ui.locale.tr("align-status-measure-dropped"));
            return;
        }
        // Keep the legend in sync with the scale used for colouring.
        if self.tools.align.settings.auto_scale {
            self.tools.align.settings.scale_mm = scale_mm;
        }
        // Do not paint a map when no valid summary exists.
        let Some(_summary) = stats.summary else {
            self.tools.align.settings.show_deviation = false;
            self.clear_deviation_overlay();
            self.tools.align.stats = Some(stats);
            self.tools.align.status = Some(self.ui.locale.tr("align-status-no-summary"));
            return;
        };
        self.tools.align.stats = Some(stats);
        self.apply_deviation_colors(colors);
        self.tools.align.status = Some(self.ui.locale.tr("align-status-measured"));
    }

    /// Measure again after a pose change, but only if the map is on screen.
    pub(super) fn measure_if_shown(&mut self) {
        // Not while the brush is open. The markings and the map are both
        // per-vertex colours on the same layer, so a measurement landing here
        // would take the operator's own paint off the surface mid-stroke.
        if self.tools.align.brush.is_armed() {
            return;
        }
        if self.tools.align.refined_match_ready
            && self.tools.align.settings.show_deviation
            && self.tools.align.tool.can_measure()
        {
            self.run_align_measure();
        }
    }

    /// Drop a map that the scan just moved out from under, and abandon whatever
    /// the worker is still computing about the pose that map described.
    ///
    /// Showing a stale map is worse than showing none: the colours describe a
    /// pose that no longer exists. The operator re-measures when they are ready.
    pub(super) fn invalidate_deviation_map(&mut self, reason: &str) {
        // Cancel work based on the previous pose before clearing its map.
        self.abandon_align_jobs();
        // A map and a refined-match claim describe the same pose and surface.
        // Invalidate the claim even when no map is currently visible.
        self.tools.align.refined_match_ready = false;
        self.tools.align.settings.show_deviation = false;
        // Preserve operator markings; only the derived map is stale.
        if self.tools.align.overlay != super::app_align_display::AlignOverlay::Map {
            return;
        }
        self.clear_deviation_overlay();
        self.tools.align.status = Some(
            self.ui
                .locale
                .tr_with("align-status-remeasure", &[("reason", reason)]),
        );
    }

    /// Throw away every alignment job in flight, queued, or already finished and
    /// waiting to be picked up.
    ///
    /// Cheap: it moves a counter and empties two small lists. Nothing here waits
    /// on the worker thread.
    pub(super) fn abandon_align_jobs(&self) {
        if let Some(worker) = self.tools.align.worker.as_ref() {
            worker.bump_generation();
        }
    }

    /// Settle everything a tab switch leaves behind.
    ///
    /// The map and ghosted layer belong to the Automatically tab; returning to
    /// it restores controls only and requires a new Best fit matching result.
    pub(super) fn settle_align_tab_change(&mut self) {
        // Either direction: a gesture belongs to the tab it started on. The drag
        // handler closes one when it finds itself on the wrong tab, but that is a
        // frame later, and one frame is enough for the release to land somewhere
        // that no longer expects it.
        self.finish_align_drag();
        self.tools.align.drag = None;
        let entering_automatic =
            self.tools.align.tab == crate::align_panel::AlignTab::Automatically;
        self.abandon_align_jobs();
        // Manual mode changes the pose without a Best fit result. Both tab
        // directions therefore revoke the old authority: returning to
        // Automatically must start with the map off and require a new refined
        // match; role inference alone is not a measurement.
        self.tools.align.refined_match_ready = false;
        self.tools.align.settings.show_deviation = false;
        if entering_automatic {
            if self.tools.align.overlay == super::app_align_display::AlignOverlay::Map {
                self.clear_deviation_overlay();
                self.tools.align.status = Some(self.ui.locale.tr("align-status-map-elsewhere"));
            }
            return;
        }
        // The arrows go too. A hand nudge moves the scan out from under every
        // point that was placed on it, so they would come back describing a fit
        // that no longer holds — and the operator asked for a clean slate here by
        // name. The pair itself stays: they chose those two scans and did not
        // un-choose them.
        let dropped_arrows = self.tools.align.tool.clear_points();
        if dropped_arrows {
            self.tools.align.rejected.clear();
        }
        if self.tools.align.overlay == super::app_align_display::AlignOverlay::Map {
            self.clear_deviation_overlay();
            self.tools.align.status = Some(self.ui.locale.tr("align-status-map-elsewhere"));
        } else if dropped_arrows {
            self.tools.align.status = Some(self.ui.locale.tr("align-status-arrows-cleared"));
        }
    }

    /// Write a new pose onto the moving layer, as one undo step. Returns whether
    /// the pose actually reached the scene.
    ///
    /// It can fail to: the layer may have left the scene while the job ran, and
    /// another tool may hold the edit state machine. The caller has to know,
    /// because it is about to tell the operator the scan was aligned.
    fn commit_align_pose(&mut self, pose: Rigid) -> bool {
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };
        let Some(moving_id) = self.tools.align.tool.moving_layer() else {
            return false;
        };
        let mut next = scene.as_ref().clone();
        if !next.meshes().iter().any(|entry| entry.id() == moving_id) {
            return false;
        }
        let Some(token) =
            self.document
                .edit_mode
                .begin_scene_edit(&next, moving_id, EditModeCommand::MoveLayer)
        else {
            return false;
        };
        if let Some(entry) = next
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == moving_id)
        {
            entry.transform = pose.to_affine();
        }
        self.document
            .edit_mode
            .finish_scene_edit_success(token, &next);
        self.set_scene(next, false);
        // An aligned scan is unsaved work, exactly as a hand-dragged one is. The
        // viewer has no project file, so the pose IS the work product — and the
        // close guard reads this one flag. Without it the app closed without
        // asking and the whole alignment was gone: the fit the operator had just
        // watched land, and every fit before it.
        self.document.mark_mesh_edits_unsaved(moving_id);
        true
    }

    /// Forget the last fit: its outlier marks, its map, and anything the worker
    /// is still computing about it.
    ///
    /// Called where the pose stops being the one the fit produced — a hand drag,
    /// a step through history. The red "ignored as an outlier" marks index pairs
    /// by position and describe one particular fit, so they cannot outlive it:
    /// left up after a Ctrl+Z they marked pairs as rejected by a fit that had
    /// been undone.
    pub(super) fn forget_align_fit(&mut self, reason: &str) {
        self.tools.align.rejected.clear();
        self.invalidate_deviation_map(reason);
    }
}

/// Catalog coordinates for a typed align failure, resolved here at the
/// presentation boundary. English values mirror the former inline sentences.
fn align_failure_parts(failure: AlignFailure) -> (&'static str, String, String) {
    match failure {
        AlignFailure::FixedSurfaceMissing => {
            ("align-fail-no-surface-fixed", String::new(), String::new())
        }
        AlignFailure::MovingSurfaceMissing => {
            ("align-fail-no-surface-moving", String::new(), String::new())
        }
        AlignFailure::MeasurementDropped => ("align-fail-recolor", String::new(), String::new()),
        AlignFailure::Fit(rejection) => fit_rejection_parts(rejection),
    }
}

fn fit_rejection_parts(rejection: FitRejection) -> (&'static str, String, String) {
    let key = match rejection {
        FitRejection::TooFewPairs { .. } => "align-reject-toofew",
        FitRejection::Unpaired { .. } => "align-reject-unpaired",
        FitRejection::Degenerate { .. } => "align-reject-degenerate-plain",
        FitRejection::UnitMismatch { .. } => "align-reject-unit",
        FitRejection::Apart { .. } => "align-reject-apart",
        FitRejection::Runaway { .. } => "align-reject-runaway",
        FitRejection::NoImprovement => "align-reject-no-improvement",
        FitRejection::NonFinite => "align-reject-nonfinite",
    };
    (key, String::new(), String::new())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    /// Source before the test module.
    fn production() -> &'static str {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_align_results.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// Typed failures map to their catalog keys at the presentation boundary.
    #[test]
    fn typed_failures_resolve_to_their_catalog_keys() {
        use super::align_failure_parts;
        use crate::align_worker::AlignFailure;
        use occluview_align::FitRejection;

        let cases = [
            (
                AlignFailure::FixedSurfaceMissing,
                "align-fail-no-surface-fixed",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::MovingSurfaceMissing,
                "align-fail-no-surface-moving",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::MeasurementDropped,
                "align-fail-recolor",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::TooFewPairs { have: 2, need: 3 }),
                "align-reject-toofew",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Unpaired {
                    moving: 4,
                    fixed: 5,
                }),
                "align-reject-unpaired",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Degenerate {
                    weak_axes: [false; 3],
                }),
                "align-reject-degenerate-plain",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Degenerate {
                    weak_axes: [true, false, true],
                }),
                "align-reject-degenerate-plain",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::UnitMismatch { ratio: 2.5 }),
                "align-reject-unit",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Apart {
                    separation: 12.0,
                    allowed: 3.0,
                }),
                "align-reject-apart",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Runaway {
                    moved_by: 20.0,
                    allowed: 5.0,
                }),
                "align-reject-runaway",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::NoImprovement),
                "align-reject-no-improvement",
                String::new(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::NonFinite),
                "align-reject-nonfinite",
                String::new(),
                String::new(),
            ),
        ];
        for (failure, key, a, b) in cases {
            assert_eq!(align_failure_parts(failure), (key, a, b));
        }
    }

    /// The worker stays presentation-free: no catalog key literals.
    #[test]
    fn worker_source_has_no_catalog_key_literals() {
        let source = crate::primary_ui_tests::production_source(include_str!("../align_worker.rs"));
        for literal in ["align-fail-", "align-reject-"] {
            assert!(
                !source.contains(literal),
                "align_worker.rs must not name catalog keys: found {literal}"
            );
        }
    }

    /// A committed pose must enter undo history and mark the layer unsaved.
    #[test]
    fn a_committed_pose_is_both_undoable_and_unsaved_work() {
        let commit = production()
            .split_once("fn commit_align_pose(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            commit.contains("begin_scene_edit(&next, moving_id, EditModeCommand::MoveLayer)")
                && commit.contains("finish_scene_edit_success(token, &next)"),
            "a fit that cannot be undone is not an edit, it is an accident"
        );
        assert!(
            commit.contains("self.document.mark_mesh_edits_unsaved(moving_id)"),
            "an aligned scan that the close guard cannot see is an alignment the \
             operator loses without being asked"
        );
    }

    /// Generation checks run for each completion in the batch.
    #[test]
    fn a_result_the_operator_has_overtaken_is_never_applied() {
        let drain = production()
            .split_once("fn drain_align_worker(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            drain.contains("for completion in completions"),
            "the drain loop moved; this contract no longer reads it"
        );
        assert!(
            drain.contains("AlignWorker::generation")
                && drain.contains("if completion.generation != current"),
            "the check has to sit inside the loop, per completion"
        );
    }

    /// A measurement without a summary must not paint the scan.
    #[test]
    fn a_measurement_with_no_summary_is_not_painted_on_the_scan() {
        let measured = production()
            .split_once("fn apply_measured_outcome(")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let guard = measured
            .split_once("let Some(_summary) = stats.summary else {")
            .map(|(_, rest)| rest);
        let (refusal, remainder) = guard
            .and_then(|rest| rest.split_once("};"))
            .unwrap_or_default();
        assert!(
            !refusal.is_empty(),
            "the no-summary arm is gone; a scan can be painted flat grey again"
        );
        assert!(
            refusal.contains("self.clear_deviation_overlay()") && refusal.contains("return"),
            "a measurement that said nothing has to take the old map down and stop"
        );
        assert!(
            !refusal.contains("apply_deviation_colors"),
            "nothing is painted for a measurement that did not happen"
        );
        assert!(
            refusal.contains("self.tools.align.settings.show_deviation = false"),
            "an empty measurement must not leave the heatmap toggle claiming a map is visible"
        );
        assert!(
            remainder.contains("self.apply_deviation_colors(colors)"),
            "the summary path still paints"
        );
    }

    /// Every path that makes a pose stale abandons the work in flight about it.
    #[test]
    fn dropping_a_stale_map_also_drops_the_work_behind_it() {
        let invalidate = production()
            .split_once("fn invalidate_deviation_map(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        let before_early_return = invalidate
            .split_once("if self.tools.align.overlay !=")
            .map_or("", |(before, _)| before);
        assert!(
            !before_early_return.is_empty(),
            "the overlay guard moved out of invalidate_deviation_map"
        );
        assert!(
            before_early_return.contains("self.abandon_align_jobs()"),
            "the jobs have to go whether or not a map was on screen: a refine \
             landing late commits a pose"
        );
    }

    /// Naming two roles is enough for the low-level compare-files path, but it
    /// is not proof that Align Meshes has completed Best fit matching.
    #[test]
    fn measurement_requires_a_landed_refined_match() {
        let source = production();
        let measure = source
            .split_once("pub(super) fn measure_if_shown(")
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            measure.contains("refined_match_ready"),
            "measurement must be gated by a successful Best fit result"
        );
        let refine = source
            .split_once("AlignOutcome::Refined")
            .and_then(|(_, rest)| rest.split_once("AlignOutcome::Measured"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            refine.contains("self.tools.align.refined_match_ready = true")
                && refine.contains("self.tools.align.settings.show_deviation = true"),
            "only a committed refined result may arm the heatmap"
        );
    }

    /// Switching back to the automatic tab restores controls only. It must not
    /// submit a measurement merely because the arm-time role guess survived.
    #[test]
    fn returning_to_automatic_does_not_measure_implicitly() {
        let source = production();
        let settle = source
            .split_once("pub(super) fn settle_align_tab_change(")
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            !settle.contains("self.measure_if_shown()"),
            "tab restoration must not launch a heatmap job"
        );
        assert!(
            settle.contains("self.tools.align.refined_match_ready = false")
                && settle.contains("self.tools.align.settings.show_deviation = false"),
            "manual mode must revoke the measurement authorization"
        );
        assert!(
            !settle
                .contains("if self.tools.align.tab == crate::align_panel::AlignTab::Automatically")
                && settle.contains("let entering_automatic"),
            "returning to Automatically must revoke readiness instead of taking an early return"
        );
    }

    /// A completion that was already in flight must not resurrect a map the
    /// operator hid, or a fit that the session invalidated in the meantime.
    #[test]
    fn late_measurement_cannot_reopen_hidden_or_unrefined_map() {
        let source = production();
        let measured = source
            .split_once("fn apply_measured_outcome(")
            .and_then(|(_, rest)| rest.split_once("pub(super) fn measure_if_shown("))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            measured.contains("refined_match_ready") && measured.contains("show_deviation"),
            "a late measurement must be rejected after the map is hidden or its fit is stale"
        );
    }
}

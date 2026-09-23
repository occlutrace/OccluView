//! What comes back from the align worker, and what it is allowed to change.
//!
//! Applies completed alignment jobs and invalidates results whose input scene
//! has changed.

use eframe::egui;
use occluview_align::{FitRejection, Rigid};
use occluview_core::SceneMeshId;

use super::app_align_display::AlignOverlay;
use super::OccluViewApp;
use crate::align_worker::{AlignCompletion, AlignFailure, AlignOutcome, AlignWorker};
use crate::edit_mode::EditModeCommand;

fn change_affects_pair(
    moving: Option<SceneMeshId>,
    fixed: Option<SceneMeshId>,
    changed_layers: &[SceneMeshId],
) -> bool {
    [moving, fixed]
        .into_iter()
        .flatten()
        .any(|layer| changed_layers.contains(&layer))
}

impl OccluViewApp {
    /// Invalidate a fit when one of the two selected surfaces changes
    /// visibility. A material update is cheap, but it changes the set of
    /// surfaces the operator can see and therefore the meaning of a later
    /// measurement. Every visibility owner calls this same boundary.
    pub(super) fn invalidate_alignment_for_visibility_changes(
        &mut self,
        changed_layers: &[SceneMeshId],
    ) {
        if change_affects_pair(
            self.tools.align.tool.moving_layer(),
            self.tools.align.tool.fixed_layer(),
            changed_layers,
        ) {
            let reason = self.ui.locale.tr("align-status-visibility-changed");
            self.invalidate_deviation_map(&reason);
        }
    }

    /// Invalidate a fit when one of the two selected surfaces is replaced.
    ///
    /// The structural mesh edits (repair, close holes, crop, cut, separate, a
    /// bridge-split commit, a cancelled mesh-edit session) all rebuild a scene
    /// draft and pass through `set_scene`, which already forgets the fit. The
    /// live sculpt commit is the one path that swaps the layer's mesh in place
    /// instead, so it has to ask here. The map and the refined claim both
    /// describe the surface that was measured; once that surface is a different
    /// mesh, neither is true any more. Both roles count, because the deviation
    /// is a property of the pair.
    pub(super) fn invalidate_alignment_for_geometry_changes(
        &mut self,
        changed_layers: &[SceneMeshId],
    ) {
        if change_affects_pair(
            self.tools.align.tool.moving_layer(),
            self.tools.align.tool.fixed_layer(),
            changed_layers,
        ) {
            let reason = self.ui.locale.tr("align-status-scan-changed");
            self.forget_align_fit(&reason);
        }
    }

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
            if self.tools.align.overlay == AlignOverlay::Map {
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
                // An open contact reading owns the per-surface colouring, and
                // the two overlays describe different measurements. The reading
                // clears the deviation map when it opens ("Contact and deviation
                // overlays are mutually exclusive"), but nothing reciprocated:
                // arming Align never closed the reading and this arm re-armed
                // the map unconditionally after `commit_align_pose` had turned it
                // off. One layer then wore `contact_map = 1` AND `measured_map =
                // 1` at once, both panels claimed their own map was live, and the
                // shader mixed the contact ramp into a colour taken from the
                // deviation ramp, so the colours belonged to neither reading.
                if self.tools.contacts.is_open() {
                    let ctx = self.ui.repaint_ctx.clone();
                    self.close_contacts(&ctx);
                    self.tools.align.status = Some(self.ui.locale.tr("align-status-refined"));
                    self.measure_if_shown();
                    return;
                }
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
                if matches!(rejection, AlignFailure::MeasurementUnobservable) {
                    // The old map, if any, belongs to a measurement whose
                    // geometry did not expose enough rigid motion. It must not
                    // remain visible while the concise refusal is shown.
                    self.tools.align.settings.show_deviation = false;
                    self.clear_deviation_overlay();
                }
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
        seen: Option<occluview_align::Observability>,
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
        // Keep this presentation-side guard even though the worker rejects the
        // same state. It protects the invariant if a future worker path forgets
        // to use `paint`, and it removes any previous map before returning.
        if seen.is_none() {
            self.tools.align.settings.show_deviation = false;
            self.clear_deviation_overlay();
            self.tools.align.status = Some(self.ui.locale.tr("align-fail-unobservable"));
            return;
        }
        self.tools.align.stats = Some(stats);
        if !self.apply_deviation_colors(colors) {
            // A generation mismatch should normally discard this completion,
            // but the layer can still disappear between the worker snapshot
            // and this UI poll. Never leave a refined/visible claim behind a
            // measurement that could not be attached to the current mesh.
            self.tools.align.refined_match_ready = false;
            self.tools.align.settings.show_deviation = false;
            self.clear_deviation_overlay();
            self.tools.align.status = Some(self.ui.locale.tr("align-status-measure-unavailable"));
            return;
        }
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
        if self.tools.align.overlay != AlignOverlay::Map {
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
        self.abandon_align_drag();
        let entering_automatic =
            self.tools.align.tab == crate::align_panel::AlignTab::Automatically;
        self.abandon_align_jobs();
        // Manual mode changes the pose without a Best fit result. Both tab
        // directions therefore revoke the old authority: returning to
        // Automatically must start with the map off and require a new refined
        // match; role inference alone is not a measurement.
        self.tools.align.refined_match_ready = false;
        self.tools.align.settings.show_deviation = false;
        // Do not key cleanup only off the enum: a partial update can leave the
        // colour cache or the faded companion alive after the enum has already
        // been reset. Every derived visual belongs to the old tab/session.
        let had_derived_overlay = self.tools.align.overlay != AlignOverlay::Nothing
            || self.align_overlay_is_up()
            || !self.tools.align.ghosted.is_empty();
        if had_derived_overlay {
            self.clear_deviation_overlay();
        }
        if entering_automatic {
            if had_derived_overlay {
                self.tools.align.status = Some(self.ui.locale.tr("align-status-map-elsewhere"));
            }
            return;
        }
        // A hand nudge invalidates all points tied to the previous fit. Keep the
        // selected pair, but clear its derived points.
        let dropped_arrows = self.tools.align.tool.clear_points();
        if dropped_arrows {
            self.tools.align.rejected.clear();
        }
        if had_derived_overlay {
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
        AlignFailure::MeasurementUnobservable => {
            ("align-fail-unobservable", String::new(), String::new())
        }
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
        FitRejection::Ambiguous => "align-reject-ambiguous",
        FitRejection::NonFinite => "align-reject-nonfinite",
    };
    (key, String::new(), String::new())
}

impl OccluViewApp {
    /// The Align worker, replacing one that has died.
    ///
    /// `AlignWorker::submit` refuses every job once the thread has failed, and
    /// an align worker can fail on a panic inside the refinement. Nothing
    /// replaced it, so Align stayed dead for the rest of the session: the tool
    /// armed, the operator pressed Best fit, and no job ever ran again. The
    /// contact worker already respawns this way; this is the same rule.
    pub(super) fn align_worker_mut(&mut self) -> &mut AlignWorker {
        if self
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::has_failed)
        {
            // Dropping it stops the thread and clears its queue.
            self.tools.align.worker = None;
        }
        self.tools
            .align
            .worker
            .get_or_insert_with(AlignWorker::spawn)
    }
}

#[cfg(test)]
#[path = "app_align_authority_tests.rs"]
mod authority_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::Rigid;

    /// Source before the test module.
    /// Typed failures map to their catalog keys at the presentation boundary.
    ///
    /// The keys are resolved at render time from a variable, so the scanner
    /// that checks literal `.tr("key")` call sites cannot see them, and the
    /// pair is what has to be checked: the helper's answer against the key this
    /// table names, and that key against the catalog the renderer resolves
    /// through. Without the second half, renaming a key in the catalogs and in
    /// both of those places reaches the operator as a `⟦key⟧` marker.
    /// Source before the test module.
    /// Typed failures map to their catalog keys at the presentation boundary.
    ///
    /// The keys are resolved at render time from a variable, so the scanner
    /// that checks literal `.tr("key")` call sites cannot see them, and the
    /// pair is what has to be checked: the helper's answer against the key this
    /// table names, and that key against the catalog the renderer resolves
    /// through. Without the second half, renaming a key in the catalogs and in
    /// both of those places reaches the operator as a `⟦key⟧` marker.
    #[test]
    fn typed_failures_resolve_to_their_catalog_keys() {
        use super::align_failure_parts;
        use crate::align_worker::AlignFailure;
        use occluview_align::FitRejection;

        let cases = [
            (
                AlignFailure::FixedSurfaceMissing,
                "align-fail-no-surface-fixed",
            ),
            (
                AlignFailure::MovingSurfaceMissing,
                "align-fail-no-surface-moving",
            ),
            (AlignFailure::MeasurementDropped, "align-fail-recolor"),
            (
                AlignFailure::MeasurementUnobservable,
                "align-fail-unobservable",
            ),
            (
                AlignFailure::Fit(FitRejection::TooFewPairs { have: 2, need: 3 }),
                "align-reject-toofew",
            ),
            (
                AlignFailure::Fit(FitRejection::Unpaired {
                    moving: 4,
                    fixed: 5,
                }),
                "align-reject-unpaired",
            ),
            (
                AlignFailure::Fit(FitRejection::Degenerate {
                    weak_axes: [false; 3],
                }),
                "align-reject-degenerate-plain",
            ),
            (
                AlignFailure::Fit(FitRejection::Degenerate {
                    weak_axes: [true, false, true],
                }),
                "align-reject-degenerate-plain",
            ),
            (
                AlignFailure::Fit(FitRejection::UnitMismatch { ratio: 2.5 }),
                "align-reject-unit",
            ),
            (
                AlignFailure::Fit(FitRejection::Apart {
                    separation: 12.0,
                    allowed: 3.0,
                }),
                "align-reject-apart",
            ),
            (
                AlignFailure::Fit(FitRejection::Runaway {
                    moved_by: 20.0,
                    allowed: 5.0,
                }),
                "align-reject-runaway",
            ),
            (
                AlignFailure::Fit(FitRejection::NoImprovement),
                "align-reject-no-improvement",
            ),
            (
                AlignFailure::Fit(FitRejection::Ambiguous),
                "align-reject-ambiguous",
            ),
            (
                AlignFailure::Fit(FitRejection::NonFinite),
                "align-reject-nonfinite",
            ),
        ];
        let embedded = crate::i18n::catalog::embedded_en_keys();
        for (failure, key) in cases {
            assert_eq!(
                align_failure_parts(failure),
                (key, String::new(), String::new()),
                "the typed failure must resolve to its own catalog key"
            );
            assert!(
                embedded.contains(key),
                "the typed failure key {key} must exist in the English catalog"
            );
        }
    }

    /// The worker stays presentation-free: no catalog key literals.
    /// A committed pose must be visible, undoable, and marked unsaved.
    /// The worker stays presentation-free: no catalog key literals.
    /// A committed pose must be visible, undoable, and marked unsaved.
    #[test]
    fn a_committed_pose_is_applied_undoable_and_unsaved_work() {
        use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
        use glam::Affine3A;

        let mut app = test_app("commit-align-pose");
        let mut scene = named_scene("lower", 0.0);
        let moving_id = push_named_layer(&mut scene, "upper", 5.0);
        app.document.scene = Some(std::sync::Arc::new(scene));
        app.tools.align.tool.arm();
        app.tools.align.tool.imply_pair(&[moving_id, moving_id]);

        let pose = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::new(1.5, -2.0, 0.25));
        assert!(app.commit_align_pose(pose), "a fit on a live scene commits");

        let moved = app.document.scene.as_ref().expect("scene").meshes()[1].transform;
        assert_eq!(moved, pose.to_affine(), "the pose reaches the live scene");
        assert!(
            app.document.has_unsaved_mesh_edits(),
            "the close guard must see the alignment, or it is lost without asking"
        );

        app.apply_history_navigation_now(false, &egui::Context::default());
        assert_eq!(
            app.document.scene.as_ref().expect("scene").meshes()[1].transform,
            Affine3A::IDENTITY,
            "Ctrl+Z returns the scan to where it was"
        );
    }

    /// Switching back to the automatic tab restores controls only. It must not
    /// submit a measurement merely because the arm-time role guess survived.
    #[test]
    fn change_invalidation_is_scoped_to_the_selected_pair() {
        use super::change_affects_pair;
        use occluview_core::{Mesh, SceneMesh};

        let moving = SceneMesh::new(Mesh::empty()).id();
        let fixed = SceneMesh::new(Mesh::empty()).id();
        let unrelated = SceneMesh::new(Mesh::empty()).id();

        assert!(change_affects_pair(Some(moving), Some(fixed), &[moving]));
        assert!(change_affects_pair(Some(moving), Some(fixed), &[fixed]));
        assert!(!change_affects_pair(
            Some(moving),
            Some(fixed),
            &[unrelated]
        ));
        assert!(!change_affects_pair(None, None, &[]));
    }
}

//! What comes back from the align worker, and what it is allowed to change.
//!
//! Applies completed alignment jobs and invalidates results whose input scene
//! has changed.

use eframe::egui;
use occluview_align::{FitRejection, Rigid};

use super::OccluViewApp;
use crate::align_worker::{AlignCompletion, AlignFailure, AlignOutcome, AlignWorker};
use crate::edit_mode::EditModeCommand;

/// What the operator is told when a finished fit could not be written.
///
/// The scan or edit state may change while the worker runs.
/// English source wording of the refused-pose status (pinned by the lock
/// test below); rendering resolves `align-status-pose-refused`.
#[allow(dead_code)]
const POSE_REFUSED: &str = "The fit finished, but the scan it was for is no longer available";

impl OccluViewApp {
    /// Drain finished jobs and apply them.
    pub(super) fn drain_align_worker(&mut self, ctx: &egui::Context) {
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
            AlignOutcome::Aligned {
                pose,
                rms,
                rejected,
            } => {
                if !self.commit_align_pose(pose) {
                    self.tools.align.status = Some(self.ui.locale.tr("align-status-pose-refused"));
                    return;
                }
                // A point fit changes the pose and invalidates any previous
                // map; refinement performs the next measurement.
                self.forget_align_fit(&self.ui.locale.tr("align-status-aligned-points"));
                self.tools.align.rejected = rejected;
                let dropped = if self.tools.align.rejected.is_empty() {
                    String::new()
                } else {
                    let names: Vec<String> = self
                        .tools
                        .align
                        .rejected
                        .iter()
                        .map(|index| (index + 1).to_string())
                        .collect();
                    self.ui
                        .locale
                        .tr_with("align-status-outlier", &[("pairs", &names.join(", "))])
                };
                self.tools.align.status = Some(self.ui.locale.tr_with(
                    "align-status-aligned",
                    &[("rms", &format!("{rms:.3}")), ("dropped", &dropped)],
                ));
            }
            AlignOutcome::Refined { pose, report } => {
                if !self.commit_align_pose(pose) {
                    self.tools.align.status = Some(self.ui.locale.tr("align-status-pose-refused"));
                    return;
                }
                let weak = weak_axis_note(
                    report.weak_trans_axes,
                    report.weak_rot_axes,
                    &self.ui.locale,
                );
                // Excluded regions are omitted from both the sample and coverage
                // counts, so report the measured region explicitly.
                let measured_over = if self.tools.align.markings.any() {
                    self.ui.locale.tr("align-status-surface-unmarked")
                } else {
                    self.ui.locale.tr("align-status-surface")
                };
                // Distinguish convergence from reaching the iteration limit.
                let settled = if report.converged {
                    String::new()
                } else {
                    self.ui.locale.tr("align-status-settled-limit")
                };
                self.tools.align.status = Some(self.ui.locale.tr_with(
                    "align-status-refined",
                    &[
                        ("rms", &format!("{:.3}", report.rms)),
                        ("pct", &format!("{:.0}", report.coverage * 100.0)),
                        ("surface", &measured_over),
                        ("settled", &settled),
                        ("weak", &weak),
                    ],
                ));
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
        seen: Option<occluview_align::Observability>,
        scale_mm: f64,
    ) {
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
        let Some(summary) = stats.summary else {
            self.clear_deviation_overlay();
            self.tools.align.stats = Some(stats);
            self.tools.align.status = Some(self.ui.locale.tr_with(
                "align-status-no-summary",
                &[
                    (
                        "reach",
                        &format!("{:.1}", self.tools.align.settings.influence_radius_mm),
                    ),
                    ("measured", &format!("{}", stats.measured)),
                    (
                        "total",
                        &format!(
                            "{}",
                            stats.measured.saturating_add(stats.unmeasured.total())
                        ),
                    ),
                ],
            ));
            return;
        };
        self.tools.align.stats = Some(stats);
        self.apply_deviation_colors(colors);
        self.tools.align.status = Some(self.ui.locale.tr_plural(
            "align-status-measured",
            &[
                ("pct", &format!("{:.0}", summary.within_tolerance * 100.0)),
                (
                    "tol",
                    &format!("{:.2}", self.tools.align.settings.tolerance_mm),
                ),
                (
                    "blind",
                    &blind_note(seen.as_ref(), summary.rms, &self.ui.locale),
                ),
            ],
            &[(
                "n",
                usize::try_from(stats.unmeasured.total()).unwrap_or(usize::MAX),
            )],
        ));
    }

    /// Measure again after a pose change, but only if the map is on screen.
    pub(super) fn measure_if_shown(&mut self) {
        // Not while the brush is open. The markings and the map are both
        // per-vertex colours on the same layer, so a measurement landing here
        // would take the operator's own paint off the surface mid-stroke.
        if self.tools.align.brush.is_armed() {
            return;
        }
        if self.tools.align.settings.show_deviation && self.tools.align.tool.can_measure() {
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
    /// The map and ghosted layer belong to the Automatically tab; re-measure
    /// when returning to it.
    pub(super) fn settle_align_tab_change(&mut self) {
        // Either direction: a gesture belongs to the tab it started on. The drag
        // handler closes one when it finds itself on the wrong tab, but that is a
        // frame later, and one frame is enough for the release to land somewhere
        // that no longer expects it.
        self.finish_align_drag();
        self.tools.align.drag = None;
        if self.tools.align.tab == crate::align_panel::AlignTab::Automatically {
            self.measure_if_shown();
            return;
        }
        self.abandon_align_jobs();
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
    match rejection {
        FitRejection::TooFewPairs { have, need } => {
            ("align-reject-toofew", have.to_string(), need.to_string())
        }
        FitRejection::Unpaired { moving, fixed } => (
            "align-reject-unpaired",
            moving.to_string(),
            fixed.to_string(),
        ),
        FitRejection::Degenerate { weak_axes } => {
            let named = axis_names(weak_axes);
            if named.is_empty() {
                (
                    "align-reject-degenerate-plain",
                    String::new(),
                    String::new(),
                )
            } else {
                ("align-reject-degenerate-line", named, String::new())
            }
        }
        FitRejection::UnitMismatch { ratio } => {
            ("align-reject-unit", format!("{ratio:.1}"), String::new())
        }
        FitRejection::Apart {
            separation,
            allowed,
        } => (
            "align-reject-apart",
            format!("{separation:.0}"),
            format!("{allowed:.0}"),
        ),
        FitRejection::Runaway { moved_by, allowed } => (
            "align-reject-runaway",
            format!("{moved_by:.0}"),
            format!("{allowed:.0}"),
        ),
        FitRejection::NonFinite => ("align-reject-nonfinite", String::new(), String::new()),
    }
}

/// World-axis names for a degeneracy report. Locale-neutral identifiers;
/// the catalog sentence places them.
fn axis_names(weak: [bool; 3]) -> String {
    ["X", "Y", "Z"]
        .into_iter()
        .zip(weak)
        .filter_map(|(name, flagged)| flagged.then_some(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the deviation map could not have seen, in a sentence.
///
/// Nearest-surface distance is a lower bound when motion is tangential. The
/// observability estimate converts the reported RMS into a possible hidden
/// displacement.
fn blind_note(
    seen: Option<&occluview_align::Observability>,
    rms_mm: f64,
    locale: &crate::i18n::LocaleManager,
) -> String {
    /// Below this the correction is not worth a sentence.
    const WORTH_SAYING: f64 = 1.15;

    let Some(seen) = seen else {
        return String::new();
    };
    if seen.has_blind_direction() {
        return locale.tr("align-status-blind-free");
    }
    let hidden = seen.hidden_displacement_mm(rms_mm);
    if !hidden.is_finite() || hidden < rms_mm * WORTH_SAYING {
        return String::new();
    }
    locale.tr_with(
        "align-status-blind-hidden",
        &[("mm", &format!("{hidden:.2}"))],
    )
}

/// Name the directions a refine could not determine, so the panel never shows
/// a confident number for a fit that is free to slide.
fn weak_axis_note(
    translation: [bool; 3],
    rotation: [bool; 3],
    locale: &crate::i18n::LocaleManager,
) -> String {
    let sliding = axis_names(translation);
    let spinning = axis_names(rotation);
    match (sliding.is_empty(), spinning.is_empty()) {
        (true, true) => String::new(),
        (false, true) => locale.tr_with("align-status-weak-slide", &[("axes", &sliding)]),
        (true, false) => locale.tr_with("align-status-weak-turn", &[("axes", &spinning)]),
        (false, false) => locale.tr_with(
            "align-status-weak-both",
            &[("sliding", &sliding), ("spinning", &spinning)],
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::weak_axis_note;

    /// Source before the test module.
    fn production() -> &'static str {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_align_results.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    fn english() -> crate::i18n::LocaleManager {
        crate::i18n::LocaleManager::for_tests()
    }

    /// The refused-pose wording renders from the catalog, pinned to source.
    #[test]
    fn refused_pose_status_matches_source_wording() {
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        assert_eq!(
            catalog.text("align-status-pose-refused").as_deref(),
            Some(super::POSE_REFUSED)
        );
    }

    /// A determined fit needs no warning.
    #[test]
    fn a_fit_that_is_pinned_down_gets_no_warning() {
        assert_eq!(
            weak_axis_note([false; 3], [false; 3], &english()),
            String::new()
        );
    }

    /// Undetermined directions are named in the status text.
    #[test]
    fn an_undetermined_direction_is_named_by_its_axis() {
        // Axis names interpolate with Fluent bidi isolation marks by design.
        let sliding = weak_axis_note([false, true, false], [false; 3], &english());
        assert!(sliding.contains("slide along"), "got {sliding}");
        assert!(sliding.contains('Y'), "got {sliding}");
        assert!(
            !sliding.contains("turn"),
            "nothing turns here, got {sliding}"
        );

        let spinning = weak_axis_note([false; 3], [true, false, false], &english());
        assert!(spinning.contains("turn about"), "got {spinning}");
        assert!(spinning.contains('X'), "got {spinning}");
        assert!(
            !spinning.contains("slide"),
            "nothing slides here, got {spinning}"
        );

        let both = weak_axis_note([true, false, true], [false, true, false], &english());
        assert!(both.contains("slide along"), "got {both}");
        assert!(both.contains("X, Z"), "got {both}");
        assert!(both.contains("turn about"), "got {both}");
        assert!(both.contains('Y'), "got {both}");
    }

    /// Typed failures map to their catalog keys at the presentation boundary.
    #[test]
    fn typed_failures_resolve_to_their_catalog_keys() {
        use super::{align_failure_parts, axis_names};
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
                "2".to_string(),
                "3".to_string(),
            ),
            (
                AlignFailure::Fit(FitRejection::Unpaired {
                    moving: 4,
                    fixed: 5,
                }),
                "align-reject-unpaired",
                "4".to_string(),
                "5".to_string(),
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
                "align-reject-degenerate-line",
                "X, Z".to_string(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::UnitMismatch { ratio: 2.5 }),
                "align-reject-unit",
                "2.5".to_string(),
                String::new(),
            ),
            (
                AlignFailure::Fit(FitRejection::Apart {
                    separation: 12.0,
                    allowed: 3.0,
                }),
                "align-reject-apart",
                "12".to_string(),
                "3".to_string(),
            ),
            (
                AlignFailure::Fit(FitRejection::Runaway {
                    moved_by: 20.0,
                    allowed: 5.0,
                }),
                "align-reject-runaway",
                "20".to_string(),
                "5".to_string(),
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
        assert_eq!(axis_names([false; 3]), "");
        assert_eq!(axis_names([true, false, true]), "X, Z");
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
            .split_once("let Some(summary) = stats.summary else {")
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
}

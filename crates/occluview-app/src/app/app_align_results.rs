//! What comes back from the align worker, and what it is allowed to change.
//!
//! Split from `app_align` because it answers a different question: that module
//! routes clicks and hands the worker geometry, this one owns the other end —
//! the finished pose, the finished map, and every case where a result is no
//! longer about the scan in front of the operator.
//!
//! That last part is why the split is worth having. A result can outlive its
//! own premise: the scan moves by hand, history steps back, the pair turns
//! around, the layer leaves the scene. A refine result **commits a pose**, so
//! one landing late is not a stale colour — it is the operator's own work being
//! overwritten.

use eframe::egui;
use occluview_align::Rigid;

use super::OccluViewApp;
use crate::align_worker::{AlignCompletion, AlignOutcome, AlignWorker};
use crate::edit_mode::EditModeCommand;

/// What the operator is told when a finished fit could not be written.
///
/// It happens: the scan can leave the scene while the job runs, and another tool
/// can hold the edit state machine. The status line used to report the fit as
/// landed either way, so the operator read a millimetre figure for a scan that
/// had not moved.
const POSE_REFUSED: &str = "The fit finished, but the scan it was for is no longer available";

impl OccluViewApp {
    /// Drain finished jobs and apply them.
    pub(super) fn drain_align_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.align_worker.as_ref() else {
            return;
        };
        let completions: Vec<AlignCompletion> = worker.drain();
        if completions.is_empty() {
            return;
        }
        for completion in completions {
            // Re-read the generation per completion, not once for the batch:
            // applying one can abandon the rest. A point fit drops the map and
            // everything the worker was still computing about the pose that map
            // described, so a measurement of that pose arriving in the same
            // batch is no longer a reading about this scan.
            let current = self
                .align_worker
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
                    self.align_status = Some(POSE_REFUSED.into());
                    return;
                }
                // Deliberately no measurement here. The point fit only gets
                // the scan close; measuring it would put a map on screen that
                // the very next step invalidates.
                // The scan just moved, so a map drawn before this describes a
                // pose that no longer exists — and the viewport would happily
                // keep re-pushing it. Ahead of adopting this fit's own outlier
                // marks, because that is what clears the previous fit's.
                self.forget_align_fit("Aligned on points");
                self.align_rejected = rejected;
                let dropped = if self.align_rejected.is_empty() {
                    String::new()
                } else {
                    let names: Vec<String> = self
                        .align_rejected
                        .iter()
                        .map(|index| (index + 1).to_string())
                        .collect();
                    format!(", pair {} ignored as an outlier", names.join(" and "))
                };
                self.align_status = Some(format!(
                    "Aligned — {rms:.3} mm on the points{dropped}. Refine to seat it."
                ));
            }
            AlignOutcome::Refined { pose, report } => {
                if !self.commit_align_pose(pose) {
                    self.align_status = Some(POSE_REFUSED.into());
                    return;
                }
                let weak = weak_axis_note(report.weak_trans_axes, report.weak_rot_axes);
                // "of the surface" is not true when a region is painted out: the
                // marked vertices are dropped before the fit samples anything, so
                // they leave the numerator AND the denominator and the coverage can
                // read a hundred per cent over half a scan. Named for what was
                // actually measured instead.
                let measured_over = if self.align_markings.any() {
                    "the unmarked surface"
                } else {
                    "the surface"
                };
                // A run that hit the iteration ceiling is where the solver gave
                // up, not where the surfaces settled. Said out loud, because the
                // two used to read identically and the second is worth another
                // pass at a wider reach.
                let settled = if report.converged {
                    ""
                } else {
                    ", stopped at the iteration limit"
                };
                self.align_status = Some(format!(
                    "Refined — {:.3} mm over {:.0}% of {measured_over}{settled}{weak}",
                    report.rms,
                    report.coverage * 100.0
                ));
                self.measure_if_shown();
            }
            AlignOutcome::Measured {
                colors,
                stats,
                seen,
                scale_mm,
            } => {
                // The brush owns the per-vertex colour channel while it is
                // open, and this measurement was submitted before it opened.
                // Applying it would repaint the moving scan with map colours
                // while the fixed scan still showed the markings — one tool
                // claiming the legend, two surfaces disagreeing, and the
                // operator reading it as "it marks on one mesh and not the
                // other".
                if self.align_brush.is_armed() {
                    // And say so. The status still read "Measuring…" from the
                    // submit, and nothing replaced it until the brush closed — so
                    // the panel claimed a measurement was running that had already
                    // finished and been thrown away.
                    self.align_status =
                        Some("Measurement dropped — the marking brush owns the colours".into());
                    ctx.request_repaint();
                    return;
                }
                // Auto-scale chose the range the colours were painted at. The
                // legend has to adopt it or it would describe a different one.
                if self.align_settings.auto_scale {
                    self.align_settings.scale_mm = scale_mm;
                }
                // No summary means too little surface reached the other scan to
                // characterise. Saying "0% within 0.20 mm" there would be a
                // reading about a measurement that did not happen — and PAINTING
                // it is worse: every unmeasured vertex is grey, so a scan the
                // operator had just moved out of reach came back flat grey and
                // read as broken. A measurement that did not happen is not
                // painted at all.
                let Some(summary) = stats.summary else {
                    self.clear_deviation_overlay();
                    self.align_stats = Some(stats);
                    self.align_status = Some(format!(
                        "Nothing to measure at {:.1} mm reach — {} of {} vertices found the other scan. \
                         Move the scans closer, or widen the reach under More settings.",
                        self.align_settings.influence_radius_mm,
                        stats.measured,
                        stats.measured.saturating_add(stats.unmeasured.total())
                    ));
                    ctx.request_repaint();
                    return;
                };
                self.align_stats = Some(stats);
                self.apply_deviation_colors(colors);
                self.align_status = Some(format!(
                    "{:.0}% within {:.2} mm, {} vertices had nothing to measure against{}",
                    summary.within_tolerance * 100.0,
                    self.align_settings.tolerance_mm,
                    stats.unmeasured.total(),
                    blind_note(seen.as_ref(), summary.rms)
                ));
            }
            AlignOutcome::Failed { message } => {
                self.align_status = Some(message);
            }
        }
        ctx.request_repaint();
    }

    /// Measure again after a pose change, but only if the map is on screen.
    pub(super) fn measure_if_shown(&mut self) {
        // Not while the brush is open. The markings and the map are both
        // per-vertex colours on the same layer, so a measurement landing here
        // would take the operator's own paint off the surface mid-stroke.
        if self.align_brush.is_armed() {
            return;
        }
        if self.align_settings.show_deviation && self.align.can_measure() {
            self.run_align_measure();
        }
    }

    /// Drop a map that the scan just moved out from under, and abandon whatever
    /// the worker is still computing about the pose that map described.
    ///
    /// Showing a stale map is worse than showing none: the colours describe a
    /// pose that no longer exists. The operator re-measures when they are ready.
    pub(super) fn invalidate_deviation_map(&mut self, reason: &str) {
        // The in-flight work goes first, and it goes whether or not a map was on
        // screen. Every caller here is a moment where the pose the worker was
        // handed stopped being the pose the scan is in — a hand drag, a Ctrl+Z,
        // a turned-around pair. A **refine** result commits a pose, so one
        // landing after a hand drag put the scan back where the operator had
        // just taken it from, as a fresh history step, with nothing to say why.
        self.abandon_align_jobs();
        // Only a MAP goes stale on screen. The region tint is the operator's own
        // paint, and dropping it here would erase the brush stroke that called
        // this.
        if self.align_overlay != super::app_align_display::AlignOverlay::Map {
            return;
        }
        self.clear_deviation_overlay();
        self.align_status = Some(format!("{reason} — run Best fit matching to measure again"));
    }

    /// Throw away every alignment job in flight, queued, or already finished and
    /// waiting to be picked up.
    ///
    /// Cheap: it moves a counter and empties two small lists. Nothing here waits
    /// on the worker thread.
    pub(super) fn abandon_align_jobs(&self) {
        if let Some(worker) = self.align_worker.as_ref() {
            worker.bump_generation();
        }
    }

    /// Settle everything a tab switch leaves behind.
    ///
    /// The distance map and the faded other scan belong to the Automatically
    /// tab: that is where the legend, the range chips and the numbers live. They
    /// used to survive into Manually, where the operator got a coloured arch
    /// with nothing on screen naming the colours — and then it vanished the
    /// moment they touched the mesh. Coming back re-measures, so the tab reads
    /// the same on the way in as it did on the way out.
    pub(super) fn settle_align_tab_change(&mut self) {
        // Either direction: a gesture belongs to the tab it started on. The drag
        // handler closes one when it finds itself on the wrong tab, but that is a
        // frame later, and one frame is enough for the release to land somewhere
        // that no longer expects it.
        self.finish_align_drag();
        self.align_drag = None;
        if self.align_tab == crate::align_panel::AlignTab::Automatically {
            self.measure_if_shown();
            return;
        }
        self.abandon_align_jobs();
        // The arrows go too. A hand nudge moves the scan out from under every
        // point that was placed on it, so they would come back describing a fit
        // that no longer holds — and the operator asked for a clean slate here by
        // name. The pair itself stays: they chose those two scans and did not
        // un-choose them.
        let dropped_arrows = self.align.clear_points();
        if dropped_arrows {
            self.align_rejected.clear();
        }
        if self.align_overlay == super::app_align_display::AlignOverlay::Map {
            self.clear_deviation_overlay();
            self.align_status =
                Some("Distance map is on the Automatically tab — it comes back there".into());
        } else if dropped_arrows {
            self.align_status = Some("Arrows cleared — moving by hand from here".into());
        }
    }

    /// Write a new pose onto the moving layer, as one undo step. Returns whether
    /// the pose actually reached the scene.
    ///
    /// It can fail to: the layer may have left the scene while the job ran, and
    /// another tool may hold the edit state machine. The caller has to know,
    /// because it is about to tell the operator the scan was aligned.
    fn commit_align_pose(&mut self, pose: Rigid) -> bool {
        let Some(scene) = self.scene.clone() else {
            return false;
        };
        let Some(moving_id) = self.align.moving_layer() else {
            return false;
        };
        let mut next = scene.as_ref().clone();
        if !next.meshes().iter().any(|entry| entry.id() == moving_id) {
            return false;
        }
        let Some(token) =
            self.edit_mode
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
        self.edit_mode.finish_scene_edit_success(token, &next);
        self.set_scene(next, false);
        // An aligned scan is unsaved work, exactly as a hand-dragged one is. The
        // viewer has no project file, so the pose IS the work product — and the
        // close guard reads this one flag. Without it the app closed without
        // asking and the whole alignment was gone: the fit the operator had just
        // watched land, and every fit before it.
        self.mark_mesh_edits_unsaved(moving_id);
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
        self.align_rejected.clear();
        self.invalidate_deviation_map(reason);
    }
}

/// What the deviation map could not have seen, in a sentence.
///
/// The map measures the distance from each moving vertex to the nearest point
/// on the fixed surface. Slide the two surfaces past each other along a
/// direction the geometry is smooth in and that nearest point slides with them,
/// so a real displacement reads as a fraction of itself — on a full arch, about
/// half. This turns the reported RMS back into the displacement that could be
/// behind it, which is the number a clinician thinks they are already reading.
fn blind_note(seen: Option<&occluview_align::Observability>, rms_mm: f64) -> String {
    /// Below this the correction is not worth a sentence.
    const WORTH_SAYING: f64 = 1.15;

    let Some(seen) = seen else {
        return String::new();
    };
    if seen.has_blind_direction() {
        return " — these surfaces can slide freely, so a displacement of any size \
                could be hiding behind this"
            .into();
    }
    let hidden = seen.hidden_displacement_mm(rms_mm);
    if !hidden.is_finite() || hidden < rms_mm * WORTH_SAYING {
        return String::new();
    }
    format!(" — a rigid mismatch of up to {hidden:.2} mm could read as this")
}

/// Name the directions a refine could not determine, so the panel never shows
/// a confident number for a fit that is free to slide.
fn weak_axis_note(translation: [bool; 3], rotation: [bool; 3]) -> String {
    let sliding = crate::align_worker::axis_names(translation);
    let spinning = crate::align_worker::axis_names(rotation);
    match (sliding.is_empty(), spinning.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" — the fit can still slide along {sliding}"),
        (true, false) => format!(" — the fit can still turn about {spinning}"),
        (false, false) => {
            format!(" — the fit can still slide along {sliding} and turn about {spinning}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::weak_axis_note;

    /// The production half of this file. A source-contract test that scanned its
    /// own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_align_results.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// A well-determined fit says nothing. Anything else here would be noise on
    /// the one line the operator actually reads.
    #[test]
    fn a_fit_that_is_pinned_down_gets_no_warning() {
        assert_eq!(weak_axis_note([false; 3], [false; 3]), String::new());
    }

    /// A scan free to slide or turn is a scan whose millimetre figure means less
    /// than it looks like it means, so the direction is named.
    #[test]
    fn an_undetermined_direction_is_named_by_its_axis() {
        let sliding = weak_axis_note([false, true, false], [false; 3]);
        assert!(sliding.contains("slide along Y"), "got {sliding}");
        assert!(
            !sliding.contains("turn"),
            "nothing turns here, got {sliding}"
        );

        let spinning = weak_axis_note([false; 3], [true, false, false]);
        assert!(spinning.contains("turn about X"), "got {spinning}");
        assert!(
            !spinning.contains("slide"),
            "nothing slides here, got {spinning}"
        );

        let both = weak_axis_note([true, false, true], [false, true, false]);
        assert!(both.contains("slide along X, Z"), "got {both}");
        assert!(both.contains("turn about Y"), "got {both}");
    }

    /// A pose change has to reach the same history Ctrl+Z reads, **and** it has
    /// to raise the unsaved flag the close guard reads. It did the first and not
    /// the second, so an automatic alignment was lost on close with no prompt —
    /// the hand-drag path had been fixed for exactly this and the fit path was
    /// missed.
    ///
    /// A source contract, and named as one: `commit_align_pose` needs a scene, a
    /// live edit state machine and a real layer, none of which a test can hand
    /// it while `OccluViewApp` cannot be constructed.
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
            commit.contains("self.mark_mesh_edits_unsaved(moving_id)"),
            "an aligned scan that the close guard cannot see is an alignment the \
             operator loses without being asked"
        );
    }

    /// The generation is re-read for every completion, not once for the batch.
    /// Applying one result can abandon the rest — a point fit does exactly that
    /// — and a refine result COMMITS a pose, so one applied late overwrites the
    /// operator's own hand movement with a fresh history step.
    ///
    /// A source contract for the same reason as above.
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

    /// A measurement that measured nothing is never painted.
    ///
    /// Every unmeasured vertex is grey, so a scan the operator had just moved out
    /// of reach came back FLAT grey — one of the two scans looking switched off,
    /// with no explanation. Grey over part of a scan is a reading (there is no
    /// tooth opposite that one); grey over all of it is not a reading at all.
    ///
    /// A source contract: the arm needs a worker result, a scene and a live GPU
    /// layer, none of which a test can hand it while `OccluViewApp` cannot be
    /// constructed. The absence of a summary is itself covered by real tests in
    /// `occluview-align`.
    #[test]
    fn a_measurement_with_no_summary_is_not_painted_on_the_scan() {
        let measured = production()
            .split_once("AlignOutcome::Measured {")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("AlignOutcome::Failed"))
            .map(|(body, _)| body)
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
            .split_once("if self.align_overlay !=")
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

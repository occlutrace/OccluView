//! Tests for the align authority boundary: which result may change the scene,
//! and which claim stops being true when the geometry under it changes.
//!
//! A `#[path]` child module of `align/results.rs`, so it reaches the
//! module's private apply path.
#![allow(clippy::expect_used, clippy::float_cmp, clippy::unwrap_used)]

use super::*;
use crate::align::align_worker::{AlignOutcome, AlignWorker};
use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
use crate::app::OccluViewApp;
use glam::Vec3;
use occluview_align::{DeviationStats, DeviationSummary, Observability, Unmeasured};
use std::sync::Arc;
use std::time::Duration;

/// A live scene with an armed tool, a named pair, and a landed refined match.
///
/// This is the state every one of these behaviours is about: two scans the
/// operator has paired, a fit that has actually run, and a map on screen.
fn app_with_a_landed_fit(name: &str) -> (OccluViewApp, SceneMeshId, SceneMeshId) {
    let mut app = test_app(name);
    let mut scene = named_scene("lower", 0.0);
    let fixed_id = scene.meshes()[0].id();
    let moving_id = push_named_layer(&mut scene, "upper", 5.0);
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
    app.workspace.scenes[0].tools.align.tool.arm();
    app.workspace.scenes[0]
        .tools
        .align
        .tool
        .imply_pair(&[moving_id, fixed_id]);
    app.workspace.scenes[0].tools.align.refined_match_ready = true;
    app.workspace.scenes[0].tools.align.settings.show_deviation = true;
    app.active_context()
        .expect("live test scene")
        .align_worker_mut();
    (app, moving_id, fixed_id)
}

fn wait_for_align_output(app: &OccluViewApp, timeout: Duration) -> bool {
    let worker = app.workspace.scenes[0]
        .tools
        .align
        .worker
        .as_ref()
        .expect("worker");
    if worker.has_pending_output() {
        return true;
    }

    let (repaint_tx, repaint_rx) = std::sync::mpsc::channel();
    app.ui
        .repaint_ctx
        .set_request_repaint_callback(move |_| {
            let _ = repaint_tx.send(());
        });
    if worker.has_pending_output() {
        return true;
    }

    repaint_rx.recv_timeout(timeout).is_ok() && worker.has_pending_output()
}

/// Manual pose changes do not invalidate local point correspondences. Repeated
/// tab trips must leave the coarse fit available.
#[test]
fn manual_pose_changes_preserve_point_pairs_across_repeated_tab_trips() {
    let (mut app, moving_id, fixed_id) = app_with_a_landed_fit("align-manual-pairs");
    for (moving, fixed) in [
        (Vec3::new(0.1, 0.2, 0.0), Vec3::new(5.1, 0.2, 0.0)),
        (Vec3::new(0.2, 0.8, 0.0), Vec3::new(5.2, 0.8, 0.0)),
    ] {
        app.workspace.scenes[0]
            .tools
            .align
            .tool
            .click(crate::align::align_tool::AlignPoint {
                layer: moving_id,
                local: moving,
                normal: Vec3::Z,
            });
        app.workspace.scenes[0]
            .tools
            .align
            .tool
            .click(crate::align::align_tool::AlignPoint {
                layer: fixed_id,
                local: fixed,
                normal: Vec3::Z,
            });
    }
    assert_eq!(app.workspace.scenes[0].tools.align.tool.pairs().len(), 2);
    let pairs = app.workspace.scenes[0].tools.align.tool.pairs().to_vec();

    app.workspace.scenes[0].tools.align.tab = crate::align::align_panel::AlignTab::Manually;
    app.active_context()
        .expect("live test scene")
        .settle_align_tab_change();
    let mut scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("scene")
        .as_ref()
        .clone();
    let moving = scene
        .meshes_mut()
        .iter_mut()
        .find(|entry| entry.id() == moving_id)
        .expect("moving scan");
    moving.transform =
        glam::Affine3A::from_translation(Vec3::new(0.0, 0.3, -0.2)) * moving.transform;
    app.workspace.scenes[0].document.scene = Some(Arc::new(scene));

    for tab in [
        crate::align::align_panel::AlignTab::Automatically,
        crate::align::align_panel::AlignTab::Manually,
        crate::align::align_panel::AlignTab::Automatically,
    ] {
        app.workspace.scenes[0].tools.align.tab = tab;
        app.active_context()
            .expect("live test scene")
            .settle_align_tab_change();
        assert_eq!(
            app.workspace.scenes[0].tools.align.tool.pairs(),
            pairs,
            "manual tab trips must preserve local correspondences"
        );
        assert!(app.workspace.scenes[0].tools.align.tool.can_align());
    }
}

/// A worker failure drops its stale state while preserving the points and
/// leaving the automatic fit action eligible once the replacement is idle.
#[test]
fn a_failed_align_worker_recovers_without_disabling_the_coarse_fit() {
    let (mut app, moving_id, fixed_id) = app_with_a_landed_fit("align-failed-worker-recovery");
    for (moving, fixed) in [
        (Vec3::new(0.1, 0.2, 0.0), Vec3::new(5.1, 0.2, 0.0)),
        (Vec3::new(0.2, 0.8, 0.0), Vec3::new(5.2, 0.8, 0.0)),
    ] {
        app.workspace.scenes[0]
            .tools
            .align
            .tool
            .click(crate::align::align_tool::AlignPoint {
                layer: moving_id,
                local: moving,
                normal: Vec3::Z,
            });
        app.workspace.scenes[0]
            .tools
            .align
            .tool
            .click(crate::align::align_tool::AlignPoint {
                layer: fixed_id,
                local: fixed,
                normal: Vec3::Z,
            });
    }
    assert!(app.workspace.scenes[0].tools.align.tool.can_align());

    app.active_context()
        .expect("live test scene")
        .align_worker_mut()
        .poison_queue_for_tests();
    assert!(
        app.workspace.scenes[0]
            .tools
            .align
            .worker
            .as_ref()
            .is_some_and(AlignWorker::has_failed),
        "the failure path must actually be reached"
    );

    app.active_context()
        .expect("live test scene")
        .drain_align_worker(&egui::Context::default());
    assert!(app.workspace.scenes[0].tools.align.worker.is_none());
    assert!(app.workspace.scenes[0].tools.align.tool.can_align());
    let replacement_is_healthy_and_idle = {
        let mut scene = app.active_context().expect("live test scene");
        let worker = scene.align_worker_mut();
        !worker.has_failed() && !worker.is_busy()
    };
    assert!(replacement_is_healthy_and_idle);
    assert!(
        app.workspace.scenes[0].tools.align.tool.can_align(),
        "a fresh idle worker leaves the fit gate open"
    );
}

/// The colours a map of the moving layer would carry: one per vertex.
fn map_colors(app: &OccluViewApp, moving_id: SceneMeshId) -> Vec<[u8; 4]> {
    let scene = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .expect("a scene");
    let count = scene
        .meshes()
        .iter()
        .find(|entry| entry.id() == moving_id)
        .expect("the moving layer")
        .mesh
        .vertices()
        .len();
    vec![[40, 90, 160, 255]; count]
}

fn a_summary() -> DeviationStats {
    DeviationStats {
        measured: 64,
        unmeasured: Unmeasured::default(),
        summary: Some(DeviationSummary {
            within_tolerance: 0.9,
            mean_abs: 0.02,
            rms: 0.03,
            median: 0.01,
            p95: 0.05,
            max_abs: 0.08,
        }),
    }
}

// The Option is production's: `apply_measured_outcome` takes
// `Option<Observability>`, and a `None` case is covered separately below.
#[allow(clippy::unnecessary_wraps)]
fn a_measurement_that_can_be_seen() -> Option<Observability> {
    Some(Observability {
        sensitivity: [1.0; 6],
        blind_rotation: glam::DVec3::ZERO,
        blind_translation: glam::DVec3::ZERO,
        pivot: glam::DVec3::ZERO,
        samples: 64,
    })
}

/// A structural change to a paired scan revokes the whole fit, not just the
/// map: the outlier marks index the pairs of one particular fit and mean
/// nothing once the surface they were measured on is gone.
#[test]
fn a_geometry_change_forgets_the_whole_fit() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-geometry-change");
    app.workspace.scenes[0].tools.align.rejected = vec![0, 1];
    let colors = map_colors(&app, moving_id);
    assert!(
        app.active_context()
            .expect("live test scene")
            .apply_deviation_colors(colors),
        "the map is up"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Map
    );
    assert!(app
        .active_context()
        .expect("live test scene")
        .align_overlay_is_up());

    app.active_context()
        .expect("live test scene")
        .invalidate_alignment_for_geometry_changes(&[moving_id]);

    assert!(
        app.workspace.scenes[0].tools.align.rejected.is_empty(),
        "marks that name the pairs of a fit that no longer describes this surface \
         must not survive it"
    );
    assert!(
        !app.workspace.scenes[0].tools.align.refined_match_ready,
        "a fit measured against the old surface is not a refined match for this one"
    );
    assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing,
        "the colours describe the old surface"
    );
    assert!(!app
        .active_context()
        .expect("live test scene")
        .align_overlay_is_up());
    let reason = app
        .ui
        .locale
        .tr(crate::i18n::message_id!("align-status-scan-changed"));
    assert_eq!(
        app.workspace.scenes[0].tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .tr_with(
                    crate::i18n::message_id!("align-status-remeasure"),
                    &[("reason", &reason)]
                )
                .as_str()
        ),
        "the operator is told to measure again, and why"
    );
}

/// A result the operator has overtaken is never applied.
///
/// Two completions of the same generation can be in hand at once. Applying the
/// first one commits a pose and invalidates the fit, which moves the generation
/// — so the second belongs to a state that no longer exists. Without the
/// per-completion re-read it lands on top of the first as a fresh history step,
/// and the scan jumps back to a pose the operator never asked for.
#[test]
fn a_result_the_operator_has_overtaken_is_never_applied() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-overtaken-result");
    let first = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::new(1.0, 0.0, 0.0));
    let second = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::new(0.0, 9.0, 0.0));

    let mut scene = app.active_context().expect("live test scene");
    let worker = scene.align_worker_mut();
    let generation = worker.generation();
    worker.publish_for_tests(
        generation,
        AlignOutcome::Aligned {
            pose: first,
            rejected: Vec::new(),
        },
    );
    worker.publish_for_tests(
        generation,
        AlignOutcome::Aligned {
            pose: second,
            rejected: Vec::new(),
        },
    );

    app.active_context()
        .expect("live test scene")
        .drain_align_worker(&egui::Context::default());

    assert_eq!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[1]
            .transform,
        first.to_affine(),
        "the pose that landed is the first one, not the one published behind it"
    );
    assert_eq!(
        app.workspace.scenes[0].document.edit_mode.undo_len(),
        1,
        "the overtaken result must not land as a second history step"
    );
    assert_ne!(
        app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("scene")
            .meshes()[1]
            .transform,
        second.to_affine()
    );
    let _ = moving_id;
}

/// A measurement that finishes after the operator has hidden the map, or after
/// the pose that authorized it stopped being a refined match, must not reopen
/// the map on the scan. The colours would describe a pose the operator has
/// already left.
#[test]
fn late_measurement_cannot_reopen_hidden_or_unrefined_map() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-late-measurement");
    let colors = map_colors(&app, moving_id);

    // The map was hidden while the job was in flight.
    app.workspace.scenes[0].tools.align.settings.show_deviation = false;
    app.active_context()
        .expect("live test scene")
        .apply_measured_outcome(
            colors.clone(),
            a_summary(),
            a_measurement_that_can_be_seen(),
            0.2,
        );
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing
    );
    assert!(
        !app.active_context()
            .expect("live test scene")
            .align_overlay_is_up(),
        "a hidden map stays hidden"
    );
    assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert!(
        app.workspace.scenes[0].tools.align.status.is_none(),
        "and no reading is reported for a map the operator did not ask to see"
    );

    // The refined claim went away under the running job.
    app.workspace.scenes[0].tools.align.settings.show_deviation = true;
    app.workspace.scenes[0].tools.align.refined_match_ready = false;
    app.active_context()
        .expect("live test scene")
        .apply_measured_outcome(colors, a_summary(), a_measurement_that_can_be_seen(), 0.2);
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing
    );
    assert!(!app
        .active_context()
        .expect("live test scene")
        .align_overlay_is_up());
    assert!(
        app.workspace.scenes[0].tools.align.status.is_none(),
        "a measurement with no landed refined match behind it reports nothing"
    );

    // Positive control: the same call paints once the map is authorized, so the
    // two refusals above cannot pass on a call that never paints at all.
    app.workspace.scenes[0].tools.align.refined_match_ready = true;
    let colors = map_colors(&app, moving_id);
    app.active_context()
        .expect("live test scene")
        .apply_measured_outcome(colors, a_summary(), a_measurement_that_can_be_seen(), 0.2);
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Map
    );
    assert_eq!(
        app.workspace.scenes[0].tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("align-status-measured"))
                .as_str()
        )
    );
}

/// A measurement that produced no numbers is not painted. The colours would be
/// the only thing on screen, and they would say nothing.
#[test]
fn a_measurement_with_no_summary_is_not_painted_on_the_scan() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-no-summary");
    let stats = DeviationStats {
        measured: 4,
        unmeasured: Unmeasured {
            out_of_reach: 2,
            ..Unmeasured::default()
        },
        summary: None,
    };

    let colors = map_colors(&app, moving_id);
    app.active_context()
        .expect("live test scene")
        .apply_measured_outcome(colors, stats, a_measurement_that_can_be_seen(), 0.2);

    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing,
        "a map with no summary behind it must not reach the scan"
    );
    assert!(!app
        .active_context()
        .expect("live test scene")
        .align_overlay_is_up());
    assert!(
        !app.workspace.scenes[0].tools.align.settings.show_deviation,
        "the toggle must not claim a map is up"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.align.stats,
        Some(stats),
        "the counts that explain the refusal are still reported"
    );
    assert_eq!(
        app.workspace.scenes[0].tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("align-status-no-summary"))
                .as_str()
        )
    );
}

/// Dropping a stale map also drops the work behind it. The job in flight is
/// about the pose the map described; letting it land re-creates the reading
/// that was just discarded.
#[test]
fn dropping_a_stale_map_also_drops_the_work_behind_it() {
    let (mut app, _moving_id, _fixed_id) = app_with_a_landed_fit("align-stale-map-drops-work");
    let generation = app
        .active_context()
        .expect("live test scene")
        .align_worker_mut()
        .generation();
    app.active_context()
        .expect("live test scene")
        .run_align_measure();

    // A published result requests a repaint, and the worker keeps it queued
    // until the scene drains it.
    assert!(
        wait_for_align_output(&app, Duration::from_secs(30)),
        "the measurement never ran"
    );

    let reason = app
        .ui
        .locale
        .tr(crate::i18n::message_id!("align-status-scan-changed"));
    app.active_context()
        .expect("live test scene")
        .invalidate_deviation_map(&reason);

    let worker = app.workspace.scenes[0]
        .tools
        .align
        .worker
        .as_ref()
        .expect("worker");
    assert!(
        worker.generation() > generation,
        "the abandoned job's generation has to move"
    );
    assert!(
        worker.drain().is_empty(),
        "a completion for the pose the map described must not survive the drop"
    );

    // Positive control: the worker still takes work and still publishes, so the
    // empty drain above is not an empty worker.
    app.workspace.scenes[0].tools.align.refined_match_ready = true;
    app.workspace.scenes[0].tools.align.settings.show_deviation = true;
    app.active_context()
        .expect("live test scene")
        .run_align_measure();
    assert!(
        wait_for_align_output(&app, Duration::from_secs(30)),
        "the second measurement never ran"
    );
    assert!(
        !app.workspace.scenes[0]
            .tools
            .align
            .worker
            .as_ref()
            .expect("worker")
            .drain()
            .is_empty(),
        "a measurement of the current pose does come back"
    );
}

/// A measurement needs a landed refined match. Naming two scans is not one, and
/// submitting anyway would colour the scan from a fit that never ran.
#[test]
fn measurement_requires_a_landed_refined_match() {
    let (mut app, _moving_id, _fixed_id) = app_with_a_landed_fit("align-measure-authority");

    app.workspace.scenes[0].tools.align.refined_match_ready = false;
    app.active_context()
        .expect("live test scene")
        .run_align_measure();
    assert!(
        app.workspace.scenes[0].tools.align.status.is_none(),
        "with no landed refined match there is nothing to measure and nothing to say"
    );
    assert!(
        !app.workspace.scenes[0]
            .tools
            .align
            .worker
            .as_ref()
            .expect("worker")
            .is_busy(),
        "and no job may be submitted"
    );

    app.workspace.scenes[0].tools.align.refined_match_ready = true;
    app.active_context()
        .expect("live test scene")
        .run_align_measure();
    assert_eq!(
        app.workspace.scenes[0].tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("align-job-measure"))
                .as_str()
        ),
        "the same call does submit once the refined match has landed"
    );
}

/// A click that contradicts the arm-time role guess turns the pair around, and
/// a map measured in the other direction is no longer a map of this pair. The
/// app-level swap path has to revoke the fit, not merely relabel the roles.
#[test]
fn a_click_that_turns_the_pair_around_invalidates_the_fit() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-swap-invalidates");
    let colors = map_colors(&app, moving_id);
    assert!(
        app.active_context()
            .expect("live test scene")
            .apply_deviation_colors(colors),
        "the map is up"
    );
    app.workspace.scenes[0].tools.align.rejected = vec![0];
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Map
    );

    let status = app
        .ui
        .locale
        .tr(crate::i18n::message_id!("align-status-turned"));
    app.active_context()
        .expect("live test scene")
        .adopt_swapped_roles(status);

    assert!(
        !app.workspace.scenes[0].tools.align.refined_match_ready,
        "a fit measured one way round is not a fit for the pair the other way round"
    );
    assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing,
        "the directional colours belong to the old order"
    );
    assert!(
        app.workspace.scenes[0].tools.align.rejected.is_empty(),
        "outlier marks name pairs of the fit that just stopped describing this pair"
    );
}

/// An optimizer setting changes the surface the fit is solved against, so the
/// landed refined match stops being authoritative and its map must come down.
/// The predicate is covered elsewhere; this is the app-side drop it drives.
#[test]
fn optimizer_setting_changes_drop_the_refined_authority() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-optimizer-authority");
    let colors = map_colors(&app, moving_id);
    assert!(
        app.active_context()
            .expect("live test scene")
            .apply_deviation_colors(colors),
        "the map is up"
    );

    let reason = app
        .ui
        .locale
        .tr(crate::i18n::message_id!("align-status-scan-changed"));
    app.active_context()
        .expect("live test scene")
        .forget_align_fit(&reason);

    assert!(
        !app.workspace.scenes[0].tools.align.refined_match_ready,
        "an optimizer change revokes the refined match"
    );
    assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert_eq!(
        app.workspace.scenes[0].tools.align.overlay,
        AlignOverlay::Nothing
    );
    assert!(
        app.workspace.scenes[0].tools.align.status.is_some(),
        "the operator is told why the reading went away"
    );
}

/// A setting change abandons the running fit immediately. The job may still be
/// mid-flight in the worker; waiting for its claim would let a result measured
/// against the old setting land on the new one.
#[test]
fn a_settings_change_abandons_a_running_fit_without_waiting_for_a_claim() {
    let (mut app, _moving_id, _fixed_id) = app_with_a_landed_fit("align-abandon-running");
    let before = app.workspace.scenes[0]
        .tools
        .align
        .worker
        .as_ref()
        .expect("worker")
        .generation();

    app.active_context()
        .expect("live test scene")
        .abandon_align_jobs();

    let after = app.workspace.scenes[0]
        .tools
        .align
        .worker
        .as_ref()
        .expect("worker")
        .generation();
    assert!(
        after > before,
        "abandoning must move the generation at once, with no wait on the worker"
    );
}

/// Returning to the automatic tab restores controls only. A measurement must
/// not be submitted merely because the operator came back to the tab that can
/// show one: the pose that was measured is gone and a new fit has to run first.
#[test]
fn returning_to_automatic_does_not_measure_implicitly() {
    let (mut app, moving_id, _fixed_id) = app_with_a_landed_fit("align-return-automatic");
    let colors = map_colors(&app, moving_id);
    assert!(
        app.active_context()
            .expect("live test scene")
            .apply_deviation_colors(colors),
        "a map is up to be dropped"
    );
    app.workspace.scenes[0].tools.align.tab = crate::align::align_panel::AlignTab::Automatically;

    app.active_context()
        .expect("live test scene")
        .settle_align_tab_change();

    assert!(
        !app.workspace.scenes[0].tools.align.refined_match_ready,
        "the old refined match does not survive the tab change"
    );
    assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    assert_ne!(
        app.workspace.scenes[0].tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .text(crate::i18n::message_id!("align-job-measure"))
                .as_str()
        ),
        "returning to the tab must not submit a measurement"
    );
    assert!(
        !app.workspace.scenes[0]
            .tools
            .align
            .worker
            .as_ref()
            .expect("worker")
            .is_busy(),
        "and nothing is left running as if a measurement had been asked for"
    );
}

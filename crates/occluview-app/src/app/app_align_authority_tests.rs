//! Tests for the align authority boundary: which result may change the scene,
//! and which claim stops being true when the geometry under it changes.
//!
//! A `#[path]` child module of `app_align_results.rs`, so it reaches the
//! module's private apply path.
#![allow(clippy::expect_used, clippy::float_cmp, clippy::unwrap_used)]

use super::*;
use crate::align_worker::AlignOutcome;
use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
use occluview_align::{DeviationStats, DeviationSummary, Observability, Unmeasured};
use std::sync::Arc;

/// A live scene with an armed tool, a named pair, and a landed refined match.
///
/// This is the state every one of these behaviours is about: two scans the
/// operator has paired, a fit that has actually run, and a map on screen.
fn app_with_a_landed_fit(name: &str) -> (OccluViewApp, SceneMeshId, SceneMeshId) {
    let mut app = test_app(name);
    let mut scene = named_scene("lower", 0.0);
    let fixed_id = scene.meshes()[0].id();
    let moving_id = push_named_layer(&mut scene, "upper", 5.0);
    app.document.scene = Some(Arc::new(scene));
    app.tools.align.tool.arm();
    app.tools.align.tool.imply_pair(&[moving_id, fixed_id]);
    app.tools.align.refined_match_ready = true;
    app.tools.align.settings.show_deviation = true;
    app.align_worker_mut();
    (app, moving_id, fixed_id)
}

/// The colours a map of the moving layer would carry: one per vertex.
fn map_colors(app: &OccluViewApp, moving_id: SceneMeshId) -> Vec<[u8; 4]> {
    let scene = app.document.scene.as_ref().expect("a scene");
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
    app.tools.align.rejected = vec![0, 1];
    let colors = map_colors(&app, moving_id);
    assert!(app.apply_deviation_colors(colors), "the map is up");
    assert_eq!(app.tools.align.overlay, AlignOverlay::Map);
    assert!(app.align_overlay_is_up());

    app.invalidate_alignment_for_geometry_changes(&[moving_id]);

    assert!(
        app.tools.align.rejected.is_empty(),
        "marks that name the pairs of a fit that no longer describes this surface \
         must not survive it"
    );
    assert!(
        !app.tools.align.refined_match_ready,
        "a fit measured against the old surface is not a refined match for this one"
    );
    assert!(!app.tools.align.settings.show_deviation);
    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "the colours describe the old surface"
    );
    assert!(!app.align_overlay_is_up());
    let reason = app.ui.locale.tr("align-status-scan-changed");
    assert_eq!(
        app.tools.align.status.as_deref(),
        Some(
            app.ui
                .locale
                .tr_with("align-status-remeasure", &[("reason", &reason)])
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

    let worker = app.align_worker_mut();
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

    app.drain_align_worker(&egui::Context::default());

    assert_eq!(
        app.document.scene.as_ref().expect("scene").meshes()[1].transform,
        first.to_affine(),
        "the pose that landed is the first one, not the one published behind it"
    );
    assert_eq!(
        app.document.edit_mode.undo_len(),
        1,
        "the overtaken result must not land as a second history step"
    );
    assert_ne!(
        app.document.scene.as_ref().expect("scene").meshes()[1].transform,
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
    app.tools.align.settings.show_deviation = false;
    app.apply_measured_outcome(
        colors.clone(),
        a_summary(),
        a_measurement_that_can_be_seen(),
        0.2,
    );
    assert_eq!(app.tools.align.overlay, AlignOverlay::Nothing);
    assert!(!app.align_overlay_is_up(), "a hidden map stays hidden");
    assert!(!app.tools.align.settings.show_deviation);
    assert!(
        app.tools.align.status.is_none(),
        "and no reading is reported for a map the operator did not ask to see"
    );

    // The refined claim went away under the running job.
    app.tools.align.settings.show_deviation = true;
    app.tools.align.refined_match_ready = false;
    app.apply_measured_outcome(colors, a_summary(), a_measurement_that_can_be_seen(), 0.2);
    assert_eq!(app.tools.align.overlay, AlignOverlay::Nothing);
    assert!(!app.align_overlay_is_up());
    assert!(
        app.tools.align.status.is_none(),
        "a measurement with no landed refined match behind it reports nothing"
    );

    // Positive control: the same call paints once the map is authorized, so the
    // two refusals above cannot pass on a call that never paints at all.
    app.tools.align.refined_match_ready = true;
    app.apply_measured_outcome(
        map_colors(&app, moving_id),
        a_summary(),
        a_measurement_that_can_be_seen(),
        0.2,
    );
    assert_eq!(app.tools.align.overlay, AlignOverlay::Map);
    assert_eq!(
        app.tools.align.status.as_deref(),
        Some(app.ui.locale.text("align-status-measured").as_str())
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

    app.apply_measured_outcome(
        map_colors(&app, moving_id),
        stats,
        a_measurement_that_can_be_seen(),
        0.2,
    );

    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "a map with no summary behind it must not reach the scan"
    );
    assert!(!app.align_overlay_is_up());
    assert!(
        !app.tools.align.settings.show_deviation,
        "the toggle must not claim a map is up"
    );
    assert_eq!(
        app.tools.align.stats,
        Some(stats),
        "the counts that explain the refusal are still reported"
    );
    assert_eq!(
        app.tools.align.status.as_deref(),
        Some(app.ui.locale.text("align-status-no-summary").as_str())
    );
}

/// Dropping a stale map also drops the work behind it. The job in flight is
/// about the pose the map described; letting it land re-creates the reading
/// that was just discarded.
#[test]
fn dropping_a_stale_map_also_drops_the_work_behind_it() {
    let (mut app, _moving_id, _fixed_id) = app_with_a_landed_fit("align-stale-map-drops-work");
    let generation = app.align_worker_mut().generation();
    app.run_align_measure();

    // Wait for the job to finish and publish: the queue is empty and nothing is
    // running exactly when its completion is waiting to be drained.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let worker = app.tools.align.worker.as_ref().expect("worker");
        if !worker.is_busy() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the measurement never ran"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    app.invalidate_deviation_map(app.ui.locale.tr("align-status-scan-changed").as_str());

    let worker = app.tools.align.worker.as_ref().expect("worker");
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
    app.tools.align.refined_match_ready = true;
    app.tools.align.settings.show_deviation = true;
    app.run_align_measure();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let worker = app.tools.align.worker.as_ref().expect("worker");
        if !worker.is_busy() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the second measurement never ran"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        !app.tools
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

    app.tools.align.refined_match_ready = false;
    app.run_align_measure();
    assert!(
        app.tools.align.status.is_none(),
        "with no landed refined match there is nothing to measure and nothing to say"
    );
    assert!(
        !app.tools.align.worker.as_ref().expect("worker").is_busy(),
        "and no job may be submitted"
    );

    app.tools.align.refined_match_ready = true;
    app.run_align_measure();
    assert_eq!(
        app.tools.align.status.as_deref(),
        Some(app.ui.locale.text("align-job-measure").as_str()),
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
    assert!(app.apply_deviation_colors(colors), "the map is up");
    app.tools.align.rejected = vec![0];
    assert_eq!(app.tools.align.overlay, AlignOverlay::Map);

    app.adopt_swapped_roles(app.ui.locale.tr("align-status-turned"));

    assert!(
        !app.tools.align.refined_match_ready,
        "a fit measured one way round is not a fit for the pair the other way round"
    );
    assert!(!app.tools.align.settings.show_deviation);
    assert_eq!(
        app.tools.align.overlay,
        AlignOverlay::Nothing,
        "the directional colours belong to the old order"
    );
    assert!(
        app.tools.align.rejected.is_empty(),
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
    assert!(app.apply_deviation_colors(colors), "the map is up");

    app.forget_align_fit(&app.ui.locale.tr("align-status-scan-changed"));

    assert!(
        !app.tools.align.refined_match_ready,
        "an optimizer change revokes the refined match"
    );
    assert!(!app.tools.align.settings.show_deviation);
    assert_eq!(app.tools.align.overlay, AlignOverlay::Nothing);
    assert!(
        app.tools.align.status.is_some(),
        "the operator is told why the reading went away"
    );
}

/// A setting change abandons the running fit immediately. The job may still be
/// mid-flight in the worker; waiting for its claim would let a result measured
/// against the old setting land on the new one.
#[test]
fn a_settings_change_abandons_a_running_fit_without_waiting_for_a_claim() {
    let (app, _moving_id, _fixed_id) = app_with_a_landed_fit("align-abandon-running");
    let before = app
        .tools
        .align
        .worker
        .as_ref()
        .expect("worker")
        .generation();

    app.abandon_align_jobs();

    let after = app
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
        app.apply_deviation_colors(colors),
        "a map is up to be dropped"
    );
    app.tools.align.tab = crate::align_panel::AlignTab::Automatically;

    app.settle_align_tab_change();

    assert!(
        !app.tools.align.refined_match_ready,
        "the old refined match does not survive the tab change"
    );
    assert!(!app.tools.align.settings.show_deviation);
    assert_ne!(
        app.tools.align.status.as_deref(),
        Some(app.ui.locale.text("align-job-measure").as_str()),
        "returning to the tab must not submit a measurement"
    );
    assert!(
        !app.tools.align.worker.as_ref().expect("worker").is_busy(),
        "and nothing is left running as if a measurement had been asked for"
    );
}

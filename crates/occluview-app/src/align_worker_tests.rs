//! Tests for [`super`]: the worker's colouring and its reuse contract.
//!
//! A `#[path]` child module of `align_worker.rs`, split out to hold the
//! workspace's 800-line file budget.
#![allow(
    clippy::expect_used,
    clippy::float_cmp,
    clippy::items_after_statements,
    clippy::panic
)]

use super::{
    color_map, matching_inputs_changed, AlignSettings, MeasureKey, SurfaceKey, WORKING_MAX_MM,
    WORKING_MIN_MM, WORKING_SCALE_MIN_MM,
};
use occluview_align::{
    deviation_colors, DeviationMap, Orientation, RampMode, RampSettings, Validity,
};

/// Best fit must run the full search, not a local-only refinement.
///
/// This is the regression that made "Best fit matching" stop finding an arch
/// that was more than a couple of millimetres out: `local_only: true` removed
/// the global feature seed and the radius ladder, so the fit kept whatever
/// surface it first touched and then failed the seating gate. Measured on a
/// real arch, the full search seated a start 8 mm out (seated fraction 0.998,
/// trustworthy) while the local-only path reported 0.016 at 4 mm and refused.
///
/// The assertion is on the setting the app builds, because that is the value
/// that decides whether the search runs at all.
#[test]
fn best_fit_runs_the_full_search_not_a_local_only_refinement() {
    let settings = AlignSettings::default().refine();
    assert!(
        !settings.local_only,
        "Best fit must be allowed to search; local_only removes the global seed \
         and the radius ladder, which is what made it stop finding a scan that \
         was more than a couple of millimetres out"
    );
    // The operator's own inputs still reach the solver unchanged.
    let tuned = AlignSettings {
        influence_radius_mm: 1.5,
        matching_ratio: 0.6,
        ..AlignSettings::default()
    }
    .refine();
    assert!((tuned.influence_radius_mm - 1.5).abs() < f64::EPSILON);
    assert!((tuned.matching_ratio - 0.6).abs() < f64::EPSILON);
    assert!(!tuned.local_only);
}

/// A map with one of everything: a hard negative, nominal, a hard positive,
/// something past the scale, and an entry that was never measured.
fn map() -> DeviationMap {
    DeviationMap {
        signed_mm: vec![-0.9, -0.31, 0.0, 0.17, 0.42, 3.0, 0.0],
        validity: vec![
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::OutOfReach,
        ],
    }
}

/// The worker colours in parallel to keep a re-colour instant. It must be
/// the SAME ramp the rest of the tool reads — the legend calls
/// `ramp_color`, and a map painted a shade off from its own legend is a
/// measurement nobody can trust.
#[test]
fn colouring_in_parallel_matches_the_library() {
    let map = map();
    for mode in [RampMode::Signed, RampMode::Magnitude] {
        for bands in [None, Some(6)] {
            let ramp = RampSettings {
                min_mm: 0.0,
                scale_mm: 0.5,
                tolerance_mm: 0.2,
                bands,
                mode,
            };
            assert_eq!(
                color_map(&map, &ramp),
                deviation_colors(&map, &ramp),
                "parallel colouring diverged at {mode:?} / {bands:?}"
            );
        }
    }
}

/// Vertices with nothing to measure against must stay grey, whatever the
/// ramp says — colouring them would invent a measurement.
#[test]
fn unmeasured_vertices_stay_grey() {
    let colors = color_map(
        &map(),
        &RampSettings {
            min_mm: 0.0,
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            mode: RampMode::Signed,
        },
    );
    assert_eq!(colors[6], occluview_align::NO_DATA_COLOR);
    assert_ne!(colors[0], occluview_align::NO_DATA_COLOR);
}

fn key() -> MeasureKey {
    MeasureKey {
        moving: (1, 2),
        fixed: SurfaceKey {
            geometry: 3,
            pose: 4,
            markings: 0,
        },
        mask: 0,
        influence_radius_bits: 5.0_f64.to_bits(),
        orientation: Orientation::Match,
    }
}

/// The reuse rule, stated as a test: a change that only repaints an
/// existing measurement must key the same way, and a change that alters
/// what is measured must not.
#[test]
fn only_the_settings_that_change_the_distances_change_the_key() {
    let base = key();
    assert_eq!(base, key(), "an unchanged measurement keys the same way");

    let mut moved = key();
    moved.moving.1 = 99;
    assert_ne!(base, moved, "moving the scan changes what is measured");

    let mut reached = key();
    reached.influence_radius_bits = 2.0_f64.to_bits();
    assert_ne!(base, reached, "the reach changes what is measured");

    let mut masked = key();
    masked.mask = 1;
    assert_ne!(base, masked, "the mask changes what is measured");

    let mut facing = key();
    facing.orientation = Orientation::Inverted;
    assert_ne!(base, facing, "facing changes the sign of every distance");
}

/// The window opens on the working range, and it opens there every time.
///
/// Dentistry works to a tenth of a millimetre. A map whose ends are five
/// millimetres apart cannot show a fit that is either good or bad in that
/// regime, and a range that FOLLOWS the measurement walks straight out of it
/// the moment two meshes are roughly placed — which is how an operator ended up
/// reading an arch in red and blue mosaic and calling it a thermal camera.
///
/// The magnitude ramp is intentionally continuous across the whole working
/// range. Tolerance is a measurement/statistics setting, not a hidden colour
/// plateau, so small but real differences remain visible to the operator.
#[test]
fn the_window_opens_on_the_working_range() {
    let settings = AlignSettings::default();
    assert!(
        (settings.scale_mm - WORKING_MAX_MM).abs() < f64::EPSILON,
        "the display maximum must open at the tightest standard range, got {}",
        settings.scale_mm
    );
    assert_eq!(settings.min_display_mm, 0.05);
    assert!(
        (settings.tolerance_mm - WORKING_MIN_MM).abs() < f64::EPSILON,
        "the nominal band must open at the one that goes with it, got {}",
        settings.tolerance_mm
    );
    assert!(
        !settings.auto_scale,
        "a range that follows the measurement leaves the working one behind"
    );
    assert_eq!(settings.ramp_mode, RampMode::Magnitude);
    assert!(
        settings.scale_mm <= WORKING_MAX_MM,
        "the range must stay inside the working display maximum"
    );
}

#[test]
fn a_persisted_wide_range_is_clamped_before_colouring() {
    let settings = AlignSettings {
        scale_mm: 1.0,
        ..AlignSettings::default()
    };
    assert_eq!(settings.ramp().scale_mm, WORKING_MAX_MM);
}

#[test]
fn the_display_range_is_absolute_zero_to_one_tenth() {
    let zero = AlignSettings {
        scale_mm: -1.0,
        ..AlignSettings::default()
    };
    assert_eq!(zero.ramp().scale_mm, WORKING_SCALE_MIN_MM);

    let above = AlignSettings {
        scale_mm: 1.0,
        ..AlignSettings::default()
    };
    assert_eq!(above.ramp().scale_mm, WORKING_MAX_MM);
}

#[test]
fn production_heatmap_ignores_legacy_banding_and_stays_continuous() {
    let settings = AlignSettings {
        bands: Some(5),
        ..AlignSettings::default()
    };

    assert_eq!(
        settings.ramp().bands,
        None,
        "the compact heatmap has no banded mode; old persisted bands must not quantize it"
    );
}

#[test]
fn optimizer_inputs_invalidate_a_refined_match_but_display_inputs_do_not() {
    let base = AlignSettings::default();

    let mut ratio = base;
    ratio.matching_ratio = 0.7;
    assert!(matching_inputs_changed(base, ratio));

    let mut radius = base;
    radius.influence_radius_mm = 4.0;
    assert!(matching_inputs_changed(base, radius));

    let mut orientation = base;
    orientation.orientation = Orientation::Inverted;
    assert!(matching_inputs_changed(base, orientation));

    let mut display = base;
    display.scale_mm = WORKING_MIN_MM;
    display.show_deviation = false;
    assert!(!matching_inputs_changed(base, display));
}

/// A 10 x 10 sheet on z = 0 with its outward normal along +Z: the surface a
/// measurement is taken against.
fn fixed_sheet() -> (Vec<f32>, Vec<u32>) {
    (
        vec![
            0.0, 0.0, 0.0, //
            10.0, 0.0, 0.0, //
            10.0, 10.0, 0.0, //
            0.0, 10.0, 0.0,
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// A sheet tilted so it stands off the fixed one by 0 mm at one edge and
/// `offset_mm` at the other — a real registration error, not a constant.
fn tilted_sheet(offset_mm: f32) -> (Vec<f32>, Vec<u32>) {
    const STEPS: usize = 20;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            #[allow(clippy::cast_precision_loss)]
            let across = column as f32 / STEPS as f32;
            #[allow(clippy::cast_precision_loss)]
            let along = row as f32 / STEPS as f32;
            positions.extend_from_slice(&[across * 10.0, along * 10.0, across * offset_mm]);
        }
    }
    let width = u32::try_from(STEPS + 1).unwrap_or(1);
    for row in 0..u32::try_from(STEPS).unwrap_or(0) {
        for column in 0..u32::try_from(STEPS).unwrap_or(0) {
            let corner = row * width + column;
            indices.extend_from_slice(&[corner, corner + 1, corner + width]);
            indices.extend_from_slice(&[corner + 1, corner + width + 1, corner + width]);
        }
    }
    (positions, indices)
}

/// The end of the chain, on geometry with a deviation a lab would care about:
/// a 0.30 mm standoff that closes to nothing across the surface.
///
/// Three separate bugs have already reached the screen here — a mirrored
/// legend, a ramp default that painted every good result one flat blue, and an
/// unlit draw that flattened the form. This one covers the first two at once:
/// the map has to show a TRANSITION rather than one colour, and every colour on
/// the surface has to be a colour the legend also shows, at the same distance.
#[test]
fn a_real_third_of_a_millimetre_shows_a_transition_the_legend_agrees_with() {
    use crate::align_overlay::legend_value_mm;
    use occluview_align::{
        deviation, deviation_stats, suggested_scale_mm, CancelFlag, DeviationSettings, Soup,
        SurfaceIndex,
    };

    let (fixed_positions, fixed_indices) = fixed_sheet();
    let index = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .expect("a fixed surface");
    let (moving_positions, moving_indices) = tilted_sheet(0.30);
    let map = deviation(
        Soup {
            positions: &moving_positions,
            indices: &moving_indices,
            mask: None,
        },
        &index,
        occluview_align::Rigid::default(),
        &DeviationSettings {
            influence_radius_mm: 5.0,
            orientation: Orientation::Match,
        },
        &CancelFlag::new(),
    );

    let stats = deviation_stats(&map, 0.2);
    assert_eq!(
        stats.unmeasured.total(),
        0,
        "every vertex had surface within reach"
    );
    let summary = stats
        .summary
        .expect("the 21 x 21 tilted sheet clears MIN_MEASURED");
    assert!(
        (summary.p95 - 0.30).abs() < 0.02,
        "the geometry does not carry the offset it was built with: p95 {:.3}",
        summary.p95
    );

    // The range the tool picks for itself, which is what the operator sees.
    // The nominal band is set well inside the 0.30 mm standoff on purpose: a
    // band as wide as the deviation is a legitimate way to get two colours, and
    // this test is about the ramp BETWEEN them.
    let ramp = RampSettings {
        min_mm: 0.0,
        scale_mm: suggested_scale_mm(&stats),
        tolerance_mm: 0.05,
        bands: None,
        mode: RampMode::Signed,
    };
    let colors = color_map(&map, &ramp);

    // A transition, not one flat colour: the nominal end has to read green and
    // the far end hot, with the two clearly different.
    let nominal = colors[0];
    let farthest = colors
        .iter()
        .zip(&map.signed_mm)
        .max_by(|left, right| {
            left.1
                .partial_cmp(right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(color, _)| *color)
        .expect("a farthest vertex");
    assert!(
        nominal[1] > nominal[0] && nominal[1] > nominal[2],
        "a vertex sitting on the surface must read nominal green, got {nominal:?}"
    );
    assert!(
        farthest[0] > farthest[1] && farthest[0] > farthest[2],
        "a 0.30 mm standoff must read hot, got {farthest:?}"
    );
    let distinct: std::collections::BTreeSet<[u8; 4]> = colors.iter().copied().collect();
    assert!(
        distinct.len() > 8,
        "the map came out in {} colours — this is the flat-blue bug",
        distinct.len()
    );

    // The legend has to describe the surface. For every measured vertex, the
    // step of the legend bar nearest its distance must carry its colour.
    const STEPS: usize = 64;
    for (color, value) in colors.iter().zip(&map.signed_mm) {
        let (_, legend) = (0..STEPS)
            .map(|step| {
                let at = legend_value_mm(step, STEPS, ramp.mode, ramp.scale_mm);
                (
                    (at - f64::from(*value)).abs(),
                    occluview_align::ramp_color(at, &ramp),
                )
            })
            .min_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("a legend step");
        for channel in 0..3 {
            let surface = i32::from(color[channel]);
            let bar = i32::from(legend[channel]);
            assert!(
                (surface - bar).abs() <= 24,
                "a vertex measured at {value:.3} mm is painted {color:?} but the legend \
                 shows {legend:?} at that distance"
            );
        }
    }
}

#[test]
fn worker_does_not_authorize_a_rank_deficient_refinement() {
    let mut job = measure_job(0);
    job.kind = super::AlignJobKind::Refine;
    let cancel = occluview_align::CancelFlag::new();
    let mut cache = super::WorkerCache::default();

    let outcome = super::execute(&job, &cancel, &mut cache);

    assert!(matches!(
        outcome,
        super::AlignOutcome::Failed {
            rejection: super::AlignFailure::Fit(occluview_align::FitRejection::NoImprovement)
        }
    ));
}

/// A display-only edit re-colours the map already measured; it does not
/// re-measure the surface. The "heavy at" slider is meant to be free, and a
/// re-measure would copy and pack megabytes on every notch.
#[test]
fn numeric_range_edits_recolour_the_cached_map() {
    let cancel = occluview_align::CancelFlag::new();
    let mut cache = super::WorkerCache::default();

    let mut job = observable_measure_job(0);
    match super::execute(&job, &cancel, &mut cache) {
        super::AlignOutcome::Measured { .. } => {}
        _ => panic!("the fixture must land a real measurement first"),
    }
    let map_ptr_before = cache
        .measured
        .as_ref()
        .expect("the measurement is cached")
        .1
        .signed_mm
        .as_ptr();

    // Change only the display range: the measurement key is untouched.
    job.settings.scale_mm *= 0.5;
    job.settings.min_display_mm = job.settings.scale_mm * 0.1;

    match super::execute(&job, &cancel, &mut cache) {
        super::AlignOutcome::Measured { .. } => {}
        _ => panic!("a display change must re-colour, not fail"),
    }
    let map_ptr_after = cache
        .measured
        .as_ref()
        .expect("still cached")
        .1
        .signed_mm
        .as_ptr();
    assert_eq!(
        map_ptr_before, map_ptr_after,
        "a numeric-range edit must reuse the measured map instead of recomputing it"
    );
}

/// The cache is keyed by the measurement, not by the job kind alone: a
/// different surface must not be coloured from the previous surface's map.
#[test]
fn a_recolour_is_refused_when_the_measurement_identity_changed() {
    let cancel = occluview_align::CancelFlag::new();
    let mut cache = super::WorkerCache::default();

    let mut job = observable_measure_job(0);
    let _ = super::execute(&job, &cancel, &mut cache);
    let before = cache.measured.as_ref().expect("cached").0.moving.1;

    // A different moving geometry identity invalidates the cached map.
    job.measure_key.moving.1 = before + 1;
    let outcome = super::execute(&job, &cancel, &mut cache);
    match outcome {
        super::AlignOutcome::Measured { .. } => {}
        _ => panic!("a re-measure must still produce a map"),
    }
    let after = cache.measured.as_ref().expect("re-cached").0.moving.1;
    assert_eq!(
        after,
        before + 1,
        "the map must be re-measured against the new identity, not reused"
    );
}

/// One real measurement job, on geometry small enough to finish immediately.
fn measure_job(generation: u64) -> super::AlignJob {
    use std::sync::Arc;

    let (fixed_positions, fixed_indices) = fixed_sheet();
    let (moving_positions, moving_indices) = tilted_sheet(0.30);
    super::AlignJob {
        generation,
        request_id: 0,
        kind: super::AlignJobKind::Measure,
        moving_positions: Arc::new(moving_positions),
        moving_indices: Arc::new(moving_indices),
        fixed_world_positions: Arc::new(fixed_positions),
        fixed_indices: Arc::new(fixed_indices),
        fixed_key: SurfaceKey {
            geometry: 1,
            pose: 2,
            markings: 0,
        },
        measure_key: key(),
        pose: occluview_align::Rigid::default(),
        pairs: Vec::new(),
        mask: None,
        fixed_mask: None,
        settings: AlignSettings::default(),
    }
}

/// A shallow bumpy surface spans all six rigid modes while remaining inside
/// the default two-millimetre correspondence radius. It is the positive
/// control for the observability gate; the flat sheet above is its negative
/// control.
fn observable_measure_job(generation: u64) -> super::AlignJob {
    use std::sync::Arc;

    const STEPS: usize = 20;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            #[allow(clippy::cast_precision_loss)]
            let x = column as f32 / STEPS as f32 * 10.0;
            #[allow(clippy::cast_precision_loss)]
            let y = row as f32 / STEPS as f32 * 10.0;
            let z = 0.35 * (x * 0.8).sin() + 0.25 * (y * 1.1).cos();
            positions.extend_from_slice(&[x, y, z]);
        }
    }
    let width = u32::try_from(STEPS + 1).unwrap_or(1);
    for row in 0..u32::try_from(STEPS).unwrap_or(0) {
        for column in 0..u32::try_from(STEPS).unwrap_or(0) {
            let corner = row * width + column;
            indices.extend_from_slice(&[corner, corner + 1, corner + width]);
            indices.extend_from_slice(&[corner + 1, corner + width + 1, corner + width]);
        }
    }
    let fixed_key = SurfaceKey {
        geometry: 11,
        pose: 12,
        markings: 0,
    };
    super::AlignJob {
        generation,
        request_id: 0,
        kind: super::AlignJobKind::Measure,
        moving_positions: Arc::new(positions.clone()),
        moving_indices: Arc::new(indices.clone()),
        fixed_world_positions: Arc::new(positions),
        fixed_indices: Arc::new(indices),
        fixed_key,
        measure_key: MeasureKey {
            moving: (13, 14),
            fixed: fixed_key,
            mask: 0,
            influence_radius_bits: 2.0_f64.to_bits(),
            orientation: Orientation::Match,
        },
        pose: occluview_align::Rigid::default(),
        pairs: Vec::new(),
        mask: None,
        fixed_mask: None,
        settings: AlignSettings {
            influence_radius_mm: 2.0,
            ..AlignSettings::default()
        },
    }
}

/// A line has enough vertices to produce a distance summary, but its true
/// rigid-motion metric is rank deficient. The worker must refuse its map
/// instead of presenting a numerically tidy but geometrically unobservable
/// result.
fn line_measure_job(generation: u64) -> super::AlignJob {
    use std::sync::Arc;

    let mut positions = Vec::new();
    for index in 0..100 {
        #[allow(clippy::cast_precision_loss)]
        let x = index as f32 / 99.0 * 10.0;
        positions.extend_from_slice(&[x, 0.0, 0.0]);
    }
    let (fixed_positions, fixed_indices) = fixed_sheet();
    let fixed_key = SurfaceKey {
        geometry: 21,
        pose: 22,
        markings: 0,
    };
    super::AlignJob {
        generation,
        request_id: 0,
        kind: super::AlignJobKind::Measure,
        moving_positions: Arc::new(positions),
        moving_indices: Arc::new(Vec::new()),
        fixed_world_positions: Arc::new(fixed_positions),
        fixed_indices: Arc::new(fixed_indices),
        fixed_key,
        measure_key: MeasureKey {
            moving: (23, 24),
            fixed: fixed_key,
            mask: 0,
            influence_radius_bits: 2.0_f64.to_bits(),
            orientation: Orientation::Match,
        },
        pose: occluview_align::Rigid::default(),
        pairs: Vec::new(),
        mask: None,
        fixed_mask: None,
        settings: AlignSettings {
            influence_radius_mm: 2.0,
            ..AlignSettings::default()
        },
    }
}

/// Poll until a result arrives, or give up. Bounded so a wedged worker fails the
/// test instead of hanging the suite. `is_busy` is no good here: it is still
/// false in the moment between submitting and the thread picking the job up.
fn harvest_one(worker: &super::AlignWorker) -> Vec<super::AlignCompletion> {
    for _ in 0..600 {
        let batch = worker.drain();
        if !batch.is_empty() {
            return batch;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Vec::new()
}

/// Poll for a fixed stretch and return everything that turned up. Used where the
/// expected answer is "nothing", which needs a wait rather than one look.
fn harvest_quiet(worker: &super::AlignWorker) -> Vec<super::AlignCompletion> {
    let mut out = Vec::new();
    for _ in 0..60 {
        out.extend(worker.drain());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    out
}

/// The positive control. Without this the staleness test below could pass on a
/// worker that simply never returns anything.
#[test]
fn a_job_of_the_current_generation_comes_back() {
    let worker = super::AlignWorker::spawn();
    let generation = worker.generation();
    worker.submit(observable_measure_job(generation));
    let completions = harvest_one(&worker);
    assert_eq!(
        completions.len(),
        1,
        "one submitted measurement, one result expected"
    );
    assert!(matches!(
        completions[0].outcome,
        super::AlignOutcome::Measured { .. }
    ));
}

#[test]
fn a_line_measurement_with_a_summary_is_rejected_as_unobservable() {
    let cancel = occluview_align::CancelFlag::new();
    let mut cache = super::WorkerCache::default();

    let outcome = super::execute(&line_measure_job(0), &cancel, &mut cache);

    assert!(matches!(
        outcome,
        super::AlignOutcome::Failed {
            rejection: super::AlignFailure::MeasurementUnobservable
        }
    ));
}

/// A poisoned queue must become an observable terminal failure instead of
/// turning every later Align action into a silent no-op.
#[test]
fn a_worker_lock_failure_is_observable() {
    let worker = super::AlignWorker::spawn();
    let queue = std::sync::Arc::clone(&worker.queue);
    let _ = std::thread::spawn(move || {
        let _guard = queue.state.lock().expect("queue lock before poisoning");
        panic!("poison the test queue");
    })
    .join();
    worker.queue.wake.notify_one();

    for _ in 0..60 {
        if worker.has_failed() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        worker.has_failed(),
        "a dead Align worker must be visible to the UI"
    );
}

/// A result the operator has overtaken never comes back.
///
/// This is the mechanism behind the whole class of "it undid what I just did"
/// reports: a **refine** result carries a pose and commits it. One landing after
/// a hand drag or a Ctrl+Z put the scan back where the operator had just taken it
/// from, as a fresh history step, with nothing on screen to say why.
#[test]
fn a_result_from_an_abandoned_generation_is_dropped() {
    let worker = super::AlignWorker::spawn();
    let generation = worker.generation();
    worker.submit(measure_job(generation));
    // Exactly what a hand drag, a step through history, or a turned-around pair
    // does: everything in flight stops being about this scan.
    let next = worker.bump_generation();
    assert!(next > generation, "the generation has to move");
    assert!(
        harvest_quiet(&worker).is_empty(),
        "a measurement of a pose the operator has left must not be applied"
    );
}

/// Abandoning also empties the queue, so a job that had not started yet does not
/// start after the operator has moved on. And the worker still takes work
/// afterwards — abandoning is not shutting down.
#[test]
fn abandoning_clears_work_that_had_not_started() {
    let worker = super::AlignWorker::spawn();
    let generation = worker.generation();
    for _ in 0..4 {
        worker.submit(measure_job(generation));
    }
    worker.bump_generation();
    assert!(harvest_quiet(&worker).is_empty());

    let fresh = worker.generation();
    worker.submit(measure_job(fresh));
    assert_eq!(harvest_one(&worker).len(), 1, "the worker still takes work");
}

/// Measurements queued back to back collapse. Dragging a slider must not queue a
/// hundred measurements of an arch.
#[test]
fn a_second_job_of_the_same_kind_replaces_the_one_still_queued() {
    let worker = super::AlignWorker::spawn();
    let generation = worker.generation();
    for _ in 0..5 {
        worker.submit(measure_job(generation));
    }
    let completions = harvest_quiet(&worker);
    assert!(
        completions.len() <= 2,
        "five submissions produced {} results — the queue is not collapsing",
        completions.len()
    );
}

/// A cancellation request can race with the last few instructions of a fast
/// job. Generation alone cannot distinguish that completion from the newest
/// job when both belong to the same scene. The request sequence is the
/// latest-wins guard for that same-generation race.
#[test]
fn an_older_same_generation_completion_is_not_applied() {
    let worker = super::AlignWorker::spawn();
    let generation = worker.generation();
    worker.submit(measure_job(generation));
    worker.submit(measure_job(generation));

    let completions = harvest_quiet(&worker);
    assert_eq!(
        completions.len(),
        1,
        "only the newest same-generation request may reach the UI"
    );
    assert_eq!(completions[0].request_id, 2);
}

/// A cylinder, the fixture the observability tests use to demonstrate a
/// *non-`None`* blind mode: an axial screw that slides along the axis without
/// changing any distance the map can see. It is full rank and every vertex has
/// a nearest hit, so `observability()` returns `Some`.
fn cylinder_positions(radius: f32, length: f32, around: usize, along: usize) -> Vec<f32> {
    let mut positions = Vec::new();
    for ring in 0..along {
        #[allow(clippy::cast_precision_loss)]
        let z = ring as f32 / (along - 1) as f32 * length - length * 0.5;
        for step in 0..around {
            #[allow(clippy::cast_precision_loss)]
            let angle = step as f32 / around as f32 * std::f32::consts::TAU;
            positions.extend_from_slice(&[radius * angle.cos(), radius * angle.sin(), z]);
        }
    }
    positions
}

#[allow(clippy::cast_possible_truncation)] // Indices are bounded by the fixture grid.
fn cylinder_indices(around: usize, along: usize) -> Vec<u32> {
    let mut indices = Vec::new();
    for ring in 0..along - 1 {
        for step in 0..around {
            let next = (step + 1) % around;
            let a = (ring * around + step) as u32;
            let b = (ring * around + next) as u32;
            let c = ((ring + 1) * around + step) as u32;
            let d = ((ring + 1) * around + next) as u32;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    indices
}

/// A weakly observable surface must still produce a map.
///
/// A cylinder slides along its own axis with every measured distance unchanged,
/// so its worst sensitivity is far below the threshold that marks an estimate
/// doing real work. That is a *warning about the measurement*, not a reason to
/// withhold it: the deviation map is still the operator's evidence, the
/// observability estimate is what bounds its blind mode, and refusing here
/// blocked legitimate full-arch alignments — the sensitivity a real arch scan is
/// allowed in `real_scans.rs` reaches below the same threshold.
#[test]
fn a_weakly_observable_surface_still_produces_its_map() {
    let cancel = occluview_align::CancelFlag::new();
    let mut cache = super::WorkerCache::default();

    let positions = cylinder_positions(5.0, 24.0, 96, 40);
    let indices = cylinder_indices(96, 40);
    let fixed_key = SurfaceKey {
        geometry: 31,
        pose: 32,
        markings: 0,
    };
    let job = super::AlignJob {
        generation: 0,
        request_id: 0,
        kind: super::AlignJobKind::Measure,
        moving_positions: std::sync::Arc::new(positions.clone()),
        moving_indices: std::sync::Arc::new(indices.clone()),
        fixed_world_positions: std::sync::Arc::new(positions),
        fixed_indices: std::sync::Arc::new(indices),
        fixed_key,
        measure_key: MeasureKey {
            moving: (33, 34),
            fixed: fixed_key,
            mask: 0,
            influence_radius_bits: 2.0_f64.to_bits(),
            orientation: Orientation::Match,
        },
        pose: occluview_align::Rigid::default(),
        pairs: Vec::new(),
        mask: None,
        fixed_mask: None,
        settings: AlignSettings {
            influence_radius_mm: 2.0,
            ..AlignSettings::default()
        },
    };

    let outcome = super::execute(&job, &cancel, &mut cache);

    assert!(
        matches!(outcome, super::AlignOutcome::Measured { .. }),
        "a measurable surface must still be measured, however blind one of its modes is"
    );
}

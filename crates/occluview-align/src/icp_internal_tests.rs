//! Focused tests for ICP state transitions that are not part of the public API.

// Synthetic fixtures index a grid with `usize` and place it in `f32` millimetres.
// The casts are bounded by the fixture sizes, which is what the lints cannot see.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use super::icp_overlap::ReciprocalSummary;
use super::{
    coarse_candidate_is_better, coarse_candidates_are_ambiguous, correspondences_at_radius,
    forward_coverage_is_sufficient, influence_radius_ladder, level_samples_are_usable, run_level,
    sample_vertices, vertex_normals, weak_axes_from_normal_matrix, CoarseCandidate, Level, Summary,
};
use crate::{CancelFlag, FitRejection, RefineSettings, Soup, SurfaceIndex};
use glam::{DQuat, DVec3};

fn flat_sheet() -> (Vec<f32>, Vec<u32>) {
    let positions = vec![
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, // row 0
        0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 1.0, 0.0, // row 1
        0.0, 2.0, 0.0, 1.0, 2.0, 0.0, 2.0, 2.0, 0.0, // row 2
    ];
    let indices = vec![
        0, 1, 3, 1, 4, 3, 1, 2, 4, 2, 5, 4, 3, 4, 6, 4, 7, 6, 4, 5, 7, 5, 8, 7,
    ];
    (positions, indices)
}

/// A coarse candidate for the comparator tests: only the evidence the decision
/// reads is filled in.
fn candidate(rms: f64, coverage: f64, reciprocal_coverage: Option<f64>) -> CoarseCandidate {
    CoarseCandidate {
        rigid: crate::Rigid::default(),
        summary: Summary {
            inliers: 400,
            inlier_ratio: 0.9,
            coverage,
            rms,
            geometric_rms: rms,
            median_abs: rms * 0.5,
            p95_abs: rms * 1.5,
            weak_rot_axes: [false; 3],
            weak_trans_axes: [false; 3],
        },
        reciprocal: reciprocal_coverage.map(|coverage| ReciprocalSummary {
            matched: 64,
            coverage,
            geometric_rms: rms,
        }),
        shift: 1.0,
        component: Some(0),
    }
}

/// A small smooth patch can have a lower residual than the true seating while
/// covering almost none of the moving scan, and the search floor admits a
/// candidate at 1% coverage. The operator's own start has to survive that.
#[test]
fn a_lower_residual_cannot_discard_the_coverage_it_does_not_explain() {
    let start = candidate(0.35, 1.0, Some(0.80));
    let distractor = candidate(0.05, 0.012, Some(0.05));

    assert!(
        !coarse_candidate_is_better(&distractor, &start),
        "a 1.2%-coverage patch must not displace a start that explains the whole scan"
    );
    // The same residual wins when it still explains the surface.
    let tight = candidate(0.05, 0.99, Some(0.80));
    assert!(
        coarse_candidate_is_better(&tight, &start),
        "a better residual at the same coverage is a real improvement"
    );
    // Or when it recovers materially more of the fixed surface: this is the
    // partial-crop case the seed search exists for.
    let wider = candidate(0.05, 0.50, Some(0.95));
    assert!(
        coarse_candidate_is_better(&wider, &start),
        "recovering more of the fixed surface justifies a coverage loss"
    );
    // A lower residual that loses coverage and explains no more of the fixed
    // surface is the failure this guard exists for.
    let narrower = candidate(0.05, 0.50, Some(0.70));
    assert!(
        !coarse_candidate_is_better(&narrower, &start),
        "a coverage loss with no fixed-surface gain must keep the incumbent"
    );
}

/// The coverage the candidate keeps is measured against the incumbent, not
/// against a fixed number of points. An absolute allowance did not matter in
/// the fixture above (a 50-point gap either way) but decided the outcome within
/// two points of the 1% search floor, which is where a small patch competes
/// with the operator's start.
#[test]
fn a_residual_win_near_the_search_floor_cannot_shrink_coverage_by_half() {
    // The incumbent is already close to the floor, so an absolute two-point
    // allowance used to accept the loss of most of what it had.
    let near_floor_start = candidate(0.35, 0.025, Some(0.80));
    let thinner = candidate(0.05, 0.010, Some(0.70));
    assert!(
        !coarse_candidate_is_better(&thinner, &near_floor_start),
        "losing 60% of a barely-covered start is not an improvement"
    );
    // Reciprocal evidence that merely ties is not a gain either.
    let tied = candidate(0.05, 0.012, Some(0.80));
    assert!(
        !coarse_candidate_is_better(&tied, &candidate(0.35, 1.0, Some(0.80))),
        "equal fixed-surface evidence cannot excuse a 99% coverage loss"
    );
    // Losing the fixed-surface evidence entirely is not a gain: a moving point
    // cloud has no reverse surface to query, and treating that absence as one
    // let a residual-only win discard the operator's coverage.
    let cloud_start = candidate(0.35, 1.0, None);
    let cloud_patch = candidate(0.05, 0.30, None);
    assert!(
        !coarse_candidate_is_better(&cloud_patch, &cloud_start),
        "an absent fixed surface must not authorize a coverage loss"
    );
    // A point-cloud candidate that keeps its coverage still wins on residual.
    let cloud_tight = candidate(0.05, 0.95, None);
    assert!(
        coarse_candidate_is_better(&cloud_tight, &cloud_start),
        "a point cloud can still improve on residual when it explains as much"
    );
}

#[test]
fn rank_deficient_nonzero_residual_is_not_reported_as_refined() {
    let (positions, indices) = flat_sheet();
    let moving = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let fixed = SurfaceIndex::build(moving).expect("flat sheet is usable");
    let normals = vertex_normals(moving);
    let fixed_samples = fixed.representative_samples(64);
    let samples = sample_vertices(moving, 64);
    let settings = RefineSettings::default();
    let cancel = CancelFlag::new();
    let level = Level {
        moving,
        normals: &normals,
        fixed: &fixed,
        moving_surface: Some(&fixed),
        fixed_samples: &fixed_samples,
        samples: &samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 0.2)),
    };

    let outcome = run_level(&level);

    assert!(
        matches!(outcome, Err(FitRejection::NoImprovement)),
        "rank-deficient nonzero residual must not authorize a heatmap"
    );
}

#[test]
fn correlated_motion_columns_are_marked_weak_even_with_large_diagonals() {
    let mut matrix = [[0.0; 6]; 6];
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    // Rotation X and translation X carry exactly the same signal. The
    // diagonal-only check sees six healthy columns; the normalized matrix has
    // a zero eigenvalue for their difference.
    matrix[0][3] = 1.0;
    matrix[3][0] = 1.0;

    let (weak_rot, weak_trans) = weak_axes_from_normal_matrix(&matrix);

    assert!(weak_rot[0], "the hidden rotational component was accepted");
    assert!(
        weak_trans[0],
        "the hidden translational component was accepted"
    );
    assert!(!weak_rot[1] && !weak_rot[2]);
    assert!(!weak_trans[1] && !weak_trans[2]);
}

#[test]
fn independent_motion_columns_are_not_scaled_into_degeneracy() {
    let mut matrix = [[0.0; 6]; 6];
    for (index, row) in matrix.iter_mut().enumerate() {
        // Deliberately span twelve orders of magnitude. These are different
        // units in a real normal matrix, so raw diagonal comparison must not
        // reject the smaller columns merely because the mesh is large.
        row[index] = if index < 3 { 1.0e12 } else { 1.0e-6 };
    }

    let (weak_rot, weak_trans) = weak_axes_from_normal_matrix(&matrix);

    assert_eq!(weak_rot, [false; 3]);
    assert_eq!(weak_trans, [false; 3]);
}

#[test]
fn an_empty_dense_level_cannot_reuse_coarse_evidence() {
    let coarse = candidate(0.05, 0.8, Some(0.8)).summary;

    assert!(matches!(
        level_samples_are_usable(Some(coarse), &[]),
        Err(FitRejection::TooFewPairs { have: 0, .. })
    ));
    assert!(!level_samples_are_usable(None, &[]).expect("an empty first level is skippable"));
    assert!(level_samples_are_usable(Some(coarse), &[0]).expect("a real level is usable"));
}

#[test]
fn equally_supported_poses_in_one_component_are_ambiguous() {
    let best = candidate(0.05, 0.8, Some(0.8));
    let mut twin = candidate(0.05, 0.8, Some(0.8));
    twin.rigid = crate::Rigid::new(DQuat::IDENTITY, DVec3::new(2.0, 0.0, 0.0));

    // A 2 mm slide on a scan with a 2 mm influence radius: half a radius is
    // 1 mm, so 2 mm is unmistakably a different answer.
    assert!(
        coarse_candidates_are_ambiguous(&twin, &best, 60.0, 1.0),
        "a repeated/symmetric window must not be selected by component id"
    );

    // The same call with the two poses six micrometres apart — one seating,
    // parameterised twice — must NOT be treated as two answers. This is the
    // case that refused every real arch pair.
    let mut nudge = candidate(0.05, 0.8, Some(0.8));
    nudge.rigid = crate::Rigid::new(
        DQuat::from_axis_angle(DVec3::Z, 0.0109),
        DVec3::new(0.006, 0.0, 0.0),
    );
    assert!(
        !coarse_candidates_are_ambiguous(&nudge, &best, 60.0, 1.0),
        "two parameterisations of one seating are not two answers"
    );
}

#[test]
fn a_large_scan_cannot_be_registered_from_a_tiny_accidental_patch() {
    assert!(!forward_coverage_is_sufficient(6, 40_000));
    assert!(forward_coverage_is_sufficient(400, 40_000));
    assert!(forward_coverage_is_sufficient(6, 400));
}

#[test]
fn radius_ladder_widens_past_a_six_point_edge_patch() {
    // A 256 x 256 grid sampled at the coarse 8,000-point budget has only a
    // thin edge band in reach at 1 mm: that band clears the six-correspondence
    // minimum but remains below the one-percent coverage floor. At 2 mm the
    // reachable band is large enough to be meaningful. The ladder must judge
    // both conditions together, or it exits one rung too early.
    let n = 256usize;
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    for j in 0..=n {
        for i in 0..=n {
            positions.extend_from_slice(&[i as f32 * 0.5, j as f32 * 0.5, 0.0]);
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    let stride = u32::try_from(n + 1).expect("fixture stride fits");
    for j in 0..u32::try_from(n).expect("fixture span fits") {
        for i in 0..u32::try_from(n).expect("fixture span fits") {
            let a = j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride]);
            indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
        }
    }
    let moving = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let fixed = SurfaceIndex::build(moving).expect("fixture surface is usable");
    let normals = vertex_normals(moving);
    let samples = sample_vertices(moving, 8_000);
    let fixed_samples = Vec::new();
    let settings = RefineSettings::default();
    let cancel = CancelFlag::new();
    let level = Level {
        moving,
        normals: &normals,
        fixed: &fixed,
        moving_surface: None,
        fixed_samples: &fixed_samples,
        samples: &samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::new(DQuat::IDENTITY, DVec3::new(128.75, 0.0, 0.0)),
    };
    let radii = influence_radius_ladder(settings.influence_radius_mm);
    let mut radius_slot = 0;

    let result = correspondences_at_radius(&level, level.start, &radii, &mut radius_slot);

    assert!(result.is_ok(), "the useful 2 mm band was not reached");
    // The operator's own 2 mm is now the FIRST rung, so a six-point edge patch
    // 2 mm away is found without any widening at all. This used to assert slot
    // 2, which was the third rung of a ladder that started at a quarter of the
    // operator's setting.
    assert_eq!(
        radius_slot, 0,
        "the operator's own radius must be the first rung, not a later one"
    );
}

/// A hypothesis that is worse on both axes is not a rival answer.
///
/// The equivalence test used to be an absolute difference against each
/// tolerance. A candidate 1.9 % worse in residual AND 1.9 % worse in coverage
/// therefore counted as "equally plausible", the ambiguity guard refused the
/// fit, and the operator saw the tool give up on a pair it could have seated.
/// Nothing about such a candidate is better than the incumbent, so it is search
/// noise: the guard must keep the better pose and carry on.
#[test]
fn a_candidate_worse_on_both_axes_is_not_a_rival_answer() {
    let mut best = candidate(0.050, 0.800, Some(0.800));
    best.rigid = crate::Rigid::IDENTITY;
    // Two millimetres away on a 60 mm scan with a 1 mm tolerance, so the guard
    // reaches the equivalence test rather than stopping at "not distinct".
    let mut worse = candidate(0.050 * 1.019, 0.800 * 0.981, Some(0.800 * 0.981));
    worse.rigid = crate::Rigid::new(DQuat::IDENTITY, DVec3::new(2.0, 0.0, 0.0));

    assert!(
        !coarse_candidates_are_ambiguous(&worse, &best, 60.0, 1.0),
        "a candidate worse in residual and coverage must not refuse the fit"
    );

    // The real rival is unchanged: a hypothesis that explains the same surface
    // exactly as well, two millimetres away, is still a second answer.
    let twin = {
        let mut twin = candidate(0.050, 0.800, Some(0.800));
        twin.rigid = crate::Rigid::IDENTITY;
        twin
    };
    let mut rival = candidate(0.050, 0.800, Some(0.800));
    rival.rigid = crate::Rigid::new(DQuat::IDENTITY, DVec3::new(2.0, 0.0, 0.0));
    assert!(
        coarse_candidates_are_ambiguous(&rival, &twin, 60.0, 1.0),
        "an equally supported distinct pose is still ambiguous"
    );
}

/// The ladder starts where the operator set it, not at a quarter of it.
#[test]
fn the_search_radius_starts_at_the_operators_own_setting() {
    let radii = influence_radius_ladder(2.0);

    assert_eq!(
        radii.first().copied(),
        Some(2.0),
        "the first radius must be the setting the operator sees"
    );
    assert!(
        radii.windows(2).all(|pair| pair[0] < pair[1]),
        "the ladder still has to widen monotonically: {radii:?}"
    );
}

/// A level that can no longer move the surface is converged, whatever its
/// residual measured.
///
/// `converged` used to also require the residual to be at or below a
/// nanometre. Two real surfaces never meet that closely — their best possible
/// answer carries the sampling error between them — so a level that had
/// genuinely stopped was reported as unconverged and the worker turned it into
/// "Best fit could not confirm an improvement". The stopping rule may only
/// describe the step that was taken; how good the fit turned out to be is the
/// trust gate's judgement, not this one's.
/// A level that can no longer move the surface is converged, whatever its
/// residual measured.
///
/// `converged` used to also require the residual to be at or below a
/// nanometre. Two independently sampled real surfaces never meet that closely
/// — their best possible answer carries the sampling error between them — so a
/// level that had genuinely stopped was reported as unconverged, and the worker
/// turned that into "Best fit could not confirm an improvement" on a pair the
/// solver had in fact seated. The stopping rule may only describe the step that
/// was taken; how good the fit turned out to be is the trust gate's judgement.
///
/// The moving surface is sampled independently of the fixed one, so a step can
/// reduce the residual without ever reaching zero. A mesh fitted to an index
/// built from its own vertices reaches an exact zero and would let the old rule
/// pass by accident — which is why the fixtures in this file could stay green
/// while the real tool refused every pair.
#[test]
fn a_level_that_cannot_move_any_further_is_converged_whatever_its_residual() {
    // Half a cell apart at the same physical extent: no rigid pose seats these
    // two samplings of the same dome exactly.
    let (moving_positions, moving_indices) = dome_for_level(24, 0.5);
    let (fixed_positions, fixed_indices) = dome_for_level(48, 0.25);
    let moving = Soup {
        positions: &moving_positions,
        indices: &moving_indices,
        mask: None,
    };
    let fixed_soup = Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    };
    let fixed = SurfaceIndex::build(fixed_soup).expect("a dome indexes");
    let normals = vertex_normals(moving);
    let fixed_samples = fixed.representative_samples(256);
    let samples = sample_vertices(moving, 512);
    let settings = RefineSettings {
        max_iterations: 200,
        ..RefineSettings::default()
    };
    let cancel = CancelFlag::new();
    let level = Level {
        moving,
        normals: &normals,
        fixed: &fixed,
        moving_surface: Some(&fixed),
        fixed_samples: &fixed_samples,
        samples: &samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 0.4)),
    };

    let outcome = run_level(&level).expect("a level with real overlap reports");

    assert!(
        outcome.converged,
        "a level that stopped stepping is converged: iterations={} rms={}",
        outcome.iterations, outcome.summary.geometric_rms
    );
}

/// A curved fixture the level tests can build directly.
#[allow(clippy::cast_precision_loss)]
fn dome_for_level(n: usize, step: f32) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    let centre = n as f32 * step * 0.5;
    for j in 0..=n {
        for i in 0..=n {
            let x = i as f32 * step;
            let y = j as f32 * step;
            let (dx, dy) = (x - centre, y - centre);
            let texture = 0.25 * (0.7 * x).sin() * (0.53 * y).cos() + 0.12 * (0.31 * x * y).sin();
            positions.extend_from_slice(&[x, y, 0.05 * dx * dx + 0.04 * dy * dy + texture]);
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    let stride = u32::try_from(n + 1).expect("stride fits");
    let span = u32::try_from(n).expect("span fits");
    for j in 0..span {
        for i in 0..span {
            let a = j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride]);
            indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
        }
    }
    (positions, indices)
}

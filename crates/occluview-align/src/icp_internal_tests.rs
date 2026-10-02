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

use super::icp_overlap::{
    common_support_coverage, directional_forward_evidence_is_sufficient,
    reciprocal_evidence_is_usable, ReciprocalSummary,
};
use super::icp_search::principal_frame_matches;
use super::icp_solve::correspondences;
use super::icp_unique::principal_axes;
use super::{
    coarse_candidate_is_better, coarse_candidates_are_ambiguous, correspondences_at_radius,
    forward_coverage_is_sufficient, influence_radius_ladder, level_samples_are_usable, run_level,
    sample_vertices, vertex_normals, weak_axes_from_normal_matrix, CoarseCandidate, Level, Summary,
};
use crate::{CancelFlag, FitRejection, RefineSettings, Soup, SurfaceIndex};
use glam::{DMat3, DQuat, DVec3};

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
/// reads is filled in. By default, common support follows forward coverage, as
/// it does when the moving surface is smaller or has no reciprocal surface.
/// Fixed-small fixtures override it with their smaller-side reciprocal support.
fn candidate(rms: f64, coverage: f64, reciprocal_coverage: Option<f64>) -> CoarseCandidate {
    CoarseCandidate {
        rigid: crate::Rigid::default(),
        summary: Summary {
            inliers: 400,
            inlier_ratio: 0.9,
            support_coverage: coverage,
            coverage,
            rms,
            geometric_rms: rms,
            seated_fraction: 0.0,
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
    let mut start = candidate(0.35, 1.0, Some(0.80));
    start.summary.support_coverage = 0.80;
    let mut distractor = candidate(0.05, 0.012, Some(0.05));
    distractor.summary.support_coverage = 0.05;

    assert!(
        !coarse_candidate_is_better(&distractor, &start),
        "a 1.2%-coverage patch must not displace a start that explains the whole scan"
    );
    // The same residual wins when it still explains the surface.
    let mut tight = candidate(0.05, 0.99, Some(0.80));
    tight.summary.support_coverage = 0.80;
    assert!(
        coarse_candidate_is_better(&tight, &start),
        "a better residual at the same coverage is a real improvement"
    );
    // Or when it recovers materially more of the fixed surface: this is the
    // partial-crop case the seed search exists for.
    let mut wider = candidate(0.05, 0.50, Some(0.95));
    wider.summary.support_coverage = 0.95;
    assert!(
        coarse_candidate_is_better(&wider, &start),
        "recovering more of the fixed surface justifies a coverage loss"
    );
    // A lower residual that loses coverage and explains no more of the fixed
    // surface is the failure this guard exists for.
    let mut narrower = candidate(0.05, 0.50, Some(0.70));
    narrower.summary.support_coverage = 0.70;
    assert!(
        !coarse_candidate_is_better(&narrower, &start),
        "a coverage loss with no fixed-surface gain must keep the incumbent"
    );
}

/// The coverage the candidate keeps is measured against the incumbent, not
/// against a fixed number of points. An absolute allowance does not matter in
/// the fixture above (a 50-point gap either way) but decides the outcome within
/// two points of the 1% search floor, which is where a small patch competes
/// with the operator's start.
#[test]
fn a_residual_win_near_the_search_floor_cannot_shrink_coverage_by_half() {
    // The incumbent is already close to the floor, so an absolute two-point
    // allowance would accept the loss of most of what it had.
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
    // would let a residual-only win discard the operator's coverage.
    let cloud_start = candidate(0.35, 1.0, None);
    let cloud_patch = candidate(0.05, 0.30, None);
    assert!(
        !coarse_candidate_is_better(&cloud_patch, &cloud_start),
        "an absent fixed surface must not authorize a coverage loss"
    );
    // A five-point support loss is outside the comparator's two-point tie
    // band, so it cannot win on residual alone.
    let cloud_outside_tie = candidate(0.05, 0.95, None);
    assert!(
        !coarse_candidate_is_better(&cloud_outside_tie, &cloud_start),
        "a point cloud below the common-support tie band cannot win on residual alone"
    );
    // A point-cloud candidate within the support tie band still wins on
    // residual while retaining at least 90% of the incumbent support.
    let cloud_tight = candidate(0.05, 0.99, None);
    assert!(
        coarse_candidate_is_better(&cloud_tight, &cloud_start),
        "a point cloud can still improve on residual when it explains as much"
    );
}

#[test]
fn coarse_ranking_compares_common_support_when_the_fixed_scan_is_smaller() {
    let mut incumbent = candidate(1.01, 0.60, Some(0.50));
    incumbent.summary.support_coverage = 0.50;
    incumbent.summary.seated_fraction = 0.90;

    let mut improved_small_side = candidate(1.0, 0.05, Some(0.80));
    improved_small_side.summary.support_coverage = 0.80;
    improved_small_side.summary.seated_fraction = 0.0;

    assert!(
        coarse_candidate_is_better(&improved_small_side, &incumbent),
        "a fixed-small crop's common support must be compared on that crop, even when the whole-moving directional coverage is lower"
    );
}

#[test]
fn open_border_correspondences_are_rejected_only_for_a_smaller_fixed_surface() {
    let small_positions = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let large_positions = vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0];
    let indices = vec![0, 1, 2];
    let small_soup = Soup {
        positions: &small_positions,
        indices: &indices,
        mask: None,
    };
    let large_soup = Soup {
        positions: &large_positions,
        indices: &indices,
        mask: None,
    };
    let small_surface = SurfaceIndex::build(small_soup).expect("small triangle indexes");
    let large_surface = SurfaceIndex::build(large_soup).expect("large triangle indexes");
    let settings = RefineSettings::default();
    let cancel = CancelFlag::new();
    let sample = [0];

    let large_normals = vertex_normals(large_soup);
    let fixed_small_samples = small_surface.representative_samples(8);
    let fixed_small_level = Level {
        moving: large_soup,
        normals: &large_normals,
        fixed: &small_surface,
        moving_surface: Some(&large_surface),
        fixed_samples: &fixed_small_samples,
        samples: &sample,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::IDENTITY,
    };
    assert!(
        correspondences(&fixed_small_level, crate::Rigid::IDENTITY, 1.0)[0].is_none(),
        "a full moving surface must not solve against the artificial open edge of a smaller fixed fragment"
    );

    let small_normals = vertex_normals(small_soup);
    let fixed_large_samples = large_surface.representative_samples(8);
    let fixed_large_level = Level {
        moving: small_soup,
        normals: &small_normals,
        fixed: &large_surface,
        moving_surface: Some(&small_surface),
        fixed_samples: &fixed_large_samples,
        samples: &sample,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::IDENTITY,
    };
    assert!(
        correspondences(&fixed_large_level, crate::Rigid::IDENTITY, 1.0)[0].is_some(),
        "the existing solve must keep border correspondences when the fixed surface is equal or larger"
    );
}

#[test]
fn rank_deficient_nonzero_residual_retains_weak_axis_evidence() {
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

    match outcome {
        Ok(outcome) => {
            assert!(outcome.summary.rms > 0.0);
            assert!(
                outcome
                    .summary
                    .weak_rot_axes
                    .into_iter()
                    .chain(outcome.summary.weak_trans_axes)
                    .any(|weak| weak),
                "the plane's nonzero residual must remain marked as rank deficient"
            );
        }
        Err(FitRejection::NoImprovement) => {}
        Err(rejection) => panic!("unexpected rank-deficient fit rejection: {rejection:?}"),
    }
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
        // Span twelve orders of magnitude. These are different
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
        coarse_candidates_are_ambiguous(&twin, &best, DVec3::ZERO, 60.0, 1.0),
        "a repeated/symmetric window must not be selected by component id"
    );

    // The same call with the two poses six micrometres apart — one seating,
    // parameterised twice — must not be treated as two answers. Real arch
    // pairs produce this pair of hypotheses.
    let mut nudge = candidate(0.05, 0.8, Some(0.8));
    nudge.rigid = crate::Rigid::new(
        DQuat::from_axis_angle(DVec3::Z, 0.0109),
        DVec3::new(0.006, 0.0, 0.0),
    );
    assert!(
        !coarse_candidates_are_ambiguous(&nudge, &best, DVec3::ZERO, 60.0, 1.0),
        "two parameterisations of one seating are not two answers"
    );
}

#[test]
fn coarse_ambiguity_does_not_depend_on_the_mesh_coordinate_origin() {
    let best = candidate(0.05, 0.8, Some(0.8));
    let rotation = DQuat::from_axis_angle(DVec3::Z, 0.01);
    for center in [DVec3::ZERO, DVec3::new(10_000.0, -20_000.0, 0.0)] {
        let mut nudge = best;
        nudge.rigid = crate::Rigid::new(rotation, center - rotation * center);
        assert!(
            !coarse_candidates_are_ambiguous(&nudge, &best, center, 60.0, 1.0),
            "a turn moving the scan rim by only 0.3 mm is one seating at {center:?}"
        );
    }
}

#[test]
fn a_large_scan_cannot_be_registered_from_a_tiny_accidental_patch() {
    assert!(!forward_coverage_is_sufficient(6, 40_000));
    assert!(forward_coverage_is_sufficient(400, 40_000));
    assert!(forward_coverage_is_sufficient(6, 400));
}

#[test]
fn common_support_uses_the_smaller_surface_in_either_role() {
    let (small_positions, indices) = flat_sheet();
    let large_positions: Vec<f32> = small_positions.iter().map(|value| value * 2.0).collect();
    let small_soup = Soup {
        positions: &small_positions,
        indices: &indices,
        mask: None,
    };
    let large_soup = Soup {
        positions: &large_positions,
        indices: &indices,
        mask: None,
    };
    let small_surface = SurfaceIndex::build(small_soup).expect("small surface");
    let large_surface = SurfaceIndex::build(large_soup).expect("large surface");
    let small_surface_samples = small_surface.representative_samples(64);
    let large_surface_samples = large_surface.representative_samples(64);
    let moving_samples: [u32; 0] = [];
    let settings = RefineSettings::default();
    let cancel = CancelFlag::new();
    let small_moving = Level {
        moving: small_soup,
        normals: &[],
        fixed: &large_surface,
        moving_surface: Some(&small_surface),
        fixed_samples: &large_surface_samples,
        samples: &moving_samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::IDENTITY,
    };
    let sparse_reverse = Some(ReciprocalSummary {
        matched: 6,
        coverage: 0.003,
        geometric_rms: 0.001,
    });

    assert_eq!(
        small_surface.surface_area_mm2() * 4.0,
        large_surface.surface_area_mm2()
    );
    assert!(directional_forward_evidence_is_sufficient(
        &small_moving,
        6,
        400
    ));
    assert!(!directional_forward_evidence_is_sufficient(
        &small_moving,
        6,
        40_000
    ));
    assert!(
        reciprocal_evidence_is_usable(&small_moving, sparse_reverse),
        "a sparse reverse sample of the larger full scan cannot veto a supported fragment"
    );
    assert_eq!(
        common_support_coverage(&small_moving, 0.8, sparse_reverse),
        0.8
    );

    let small_fixed = Level {
        moving: large_soup,
        normals: &[],
        fixed: &small_surface,
        moving_surface: Some(&large_surface),
        fixed_samples: &small_surface_samples,
        samples: &moving_samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::IDENTITY,
    };
    assert!(directional_forward_evidence_is_sufficient(
        &small_fixed,
        6,
        40_000
    ));
    assert!(
        !reciprocal_evidence_is_usable(&small_fixed, sparse_reverse),
        "a large moving scan needs usable support measured on the smaller fixed surface"
    );
    let supported_reverse = Some(ReciprocalSummary {
        matched: 32,
        coverage: 0.8,
        geometric_rms: 0.001,
    });
    assert!(reciprocal_evidence_is_usable(
        &small_fixed,
        supported_reverse
    ));
    assert_eq!(
        common_support_coverage(&small_fixed, 0.08, supported_reverse),
        0.8
    );
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
    // The operator's own 2 mm is the first rung, so a six-point edge patch
    // 2 mm away is found without any widening at all.
    assert_eq!(
        radius_slot, 0,
        "the operator's own radius must be the first rung, not a later one"
    );
}

/// A hypothesis that is worse on both axes is not a rival answer.
///
/// A candidate 1.9 % worse in residual and 1.9 % worse in coverage is within
/// each absolute tolerance, but nothing about it is better than the incumbent,
/// so it is search noise: the ambiguity guard must keep the better pose and
/// carry on rather than refuse a pair it can seat.
#[test]
fn a_candidate_worse_on_both_axes_is_not_a_rival_answer() {
    let mut best = candidate(0.050, 0.800, Some(0.800));
    best.rigid = crate::Rigid::IDENTITY;
    // Two millimetres away on a 60 mm scan with a 1 mm tolerance, so the guard
    // reaches the equivalence test rather than stopping at "not distinct".
    let mut worse = candidate(0.050 * 1.019, 0.800 * 0.981, Some(0.800 * 0.981));
    worse.rigid = crate::Rigid::new(DQuat::IDENTITY, DVec3::new(2.0, 0.0, 0.0));

    assert!(
        !coarse_candidates_are_ambiguous(&worse, &best, DVec3::ZERO, 60.0, 1.0),
        "a candidate worse in residual and coverage must not refuse the fit"
    );

    // A real rival stays ambiguous: a hypothesis that explains the same surface
    // equally well, two millimetres away, is a second answer.
    let twin = {
        let mut twin = candidate(0.050, 0.800, Some(0.800));
        twin.rigid = crate::Rigid::IDENTITY;
        twin
    };
    let mut rival = candidate(0.050, 0.800, Some(0.800));
    rival.rigid = crate::Rigid::new(DQuat::IDENTITY, DVec3::new(2.0, 0.0, 0.0));
    assert!(
        coarse_candidates_are_ambiguous(&rival, &twin, DVec3::ZERO, 60.0, 1.0),
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

/// A level that cannot move the surface further is converged, whatever its
/// residual measured.
///
/// Two independently sampled real surfaces never meet within a nanometre —
/// their best possible answer carries the sampling error between them — so a
/// residual threshold in the stopping rule would report a stopped level as
/// unconverged, and the worker would turn that into "Best fit could not confirm
/// an improvement" on a pair the solver has seated. The stopping rule describes
/// only the step that was taken; how good the fit is remains the trust gate's
/// judgement.
///
/// The moving surface is sampled independently of the fixed one, so a step can
/// reduce the residual without ever reaching zero. A mesh fitted to an index
/// built from its own vertices reaches an exact zero and would satisfy a
/// residual threshold by accident.
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

/// The principal-frame hypotheses have to contain the true rotation, whatever
/// the axis, because nothing downstream can recover a turn the hypotheses miss.
#[test]
fn principal_frame_matches_contain_the_true_rotation() {
    let (positions, _) = dome_for_level(12, 1.0);
    let points: Vec<DVec3> = positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| {
            DVec3::new(
                f64::from(point[0]),
                f64::from(point[1]),
                f64::from(point[2]),
            )
        })
        .collect();
    let true_rotation =
        DQuat::from_axis_angle(DVec3::new(0.31, 0.9, 0.29).normalize(), 0.8).normalize();
    let centre = DVec3::new(4.0, 5.0, 6.0);
    let posed: Vec<DVec3> = points
        .iter()
        .map(|point| true_rotation * (*point - centre) + centre)
        .collect();

    let (_, moving_axes) = principal_axes(&points).expect("a frame for the source points");
    let (_, fixed_axes) = principal_axes(&posed).expect("a frame for the posed points");
    let matches = principal_frame_matches(moving_axes, fixed_axes);

    let identity = DMat3::IDENTITY;
    let closest = matches
        .iter()
        .map(|rotation| {
            let error = DMat3::from_quat(*rotation) * DMat3::from_quat(true_rotation).transpose();
            (error - identity)
                .to_cols_array()
                .iter()
                .fold(0.0_f64, |worst, value| worst.max(value.abs()))
        })
        .fold(f64::INFINITY, f64::min);
    // The axes come from a fixed-count power iteration, so they carry a
    // sub-millidegree error that the local refine absorbs. A hypothesis set
    // that missed the turn would sit tens of degrees away.
    assert!(
        closest < 0.005,
        "no hypothesis matched the true rotation; the closest was {closest} off"
    );
}

//! Focused tests for ICP state transitions that are not part of the public API.

#![allow(clippy::expect_used, clippy::panic)]

use super::icp_overlap::ReciprocalSummary;
use super::{
    coarse_candidate_is_better, correspondences_at_radius, forward_coverage_is_sufficient,
    influence_radius_ladder, run_level, sample_vertices, vertex_normals, CoarseCandidate, Level,
    Summary,
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
    assert_eq!(
        radius_slot, 2,
        "the ladder stopped before the meaningful rung"
    );
}

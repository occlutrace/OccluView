//! Tests for the point-pair fit, split out of `pairs.rs` to hold the
//! workspace's file budget.

use crate::pairs::{fit_pairs, FitRejection};
use crate::Rigid;
use glam::{DMat3, DQuat, DVec3};

fn pose() -> Rigid {
    Rigid::new(
        DQuat::from_axis_angle(DVec3::new(0.2, 0.9, -0.3).normalize(), 0.41),
        DVec3::new(2.0, -1.0, 0.75),
    )
}

/// Four well-spread points: not collinear, not coincident, and far enough
/// apart that a fit is genuinely determined.
fn spread() -> Vec<DVec3> {
    vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(10.0, 0.0, 1.0),
        DVec3::new(0.0, 9.0, -2.0),
        DVec3::new(7.0, 8.0, 5.0),
    ]
}

fn posed(points: &[DVec3]) -> Vec<DVec3> {
    points.iter().map(|&point| pose().apply(point)).collect()
}

#[test]
fn four_clean_pairs_recover_the_transform() {
    let moving = spread();
    let fixed = posed(&moving);
    let fit = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    for point in &moving {
        let error = (fit.rigid.apply(*point) - pose().apply(*point)).length();
        assert!(error < 1e-9, "recovered pose is off by {error}");
    }
    assert!(fit.pair_rms < 1e-9);
    assert!(fit.rejected.is_empty());
}

#[test]
fn three_pairs_are_enough() {
    let moving = spread()[..3].to_vec();
    let fixed = posed(&moving);
    let fit = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    assert!(fit.pair_rms < 1e-9);
}

#[test]
fn fewer_than_two_pairs_is_refused() {
    let outcome = fit_pairs(&[DVec3::ZERO], &[DVec3::ONE], None, 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::TooFewPairs { have: 1, need: 2 })),
        "expected TooFewPairs, got {outcome:?}"
    );
}

#[test]
fn collinear_pairs_are_refused_with_a_weak_axis() {
    let moving: Vec<DVec3> = (0..5).map(|k| DVec3::new(f64::from(k), 0.0, 0.0)).collect();
    let fixed = posed(&moving);
    let outcome = fit_pairs(&moving, &fixed, None, 40.0);
    match outcome {
        Err(FitRejection::Degenerate { weak_axes }) => {
            assert!(weak_axes[0], "a line along x leaves rotation about x free");
        }
        other => assert!(
            matches!(other, Err(FitRejection::Degenerate { .. })),
            "expected Degenerate, got {other:?}"
        ),
    }
}

#[test]
fn a_unit_mismatch_is_reported_not_fitted() {
    let moving = spread();
    let fixed: Vec<DVec3> = moving
        .iter()
        .map(|&point| pose().apply(point * 10.0))
        .collect();
    let outcome = fit_pairs(&moving, &fixed, None, 400.0);
    match outcome {
        Err(FitRejection::UnitMismatch { ratio }) => {
            assert!((ratio - 10.0).abs() < 0.5, "ratio was {ratio}");
        }
        other => assert!(
            matches!(other, Err(FitRejection::UnitMismatch { .. })),
            "expected UnitMismatch, got {other:?}"
        ),
    }
}

#[test]
fn one_bad_pair_is_rejected_and_the_rest_still_fit() {
    let mut moving = spread();
    moving.push(DVec3::new(3.0, 3.0, 3.0));
    let mut fixed = posed(&moving);
    let last = fixed.len() - 1;
    fixed[last] += DVec3::new(12.0, -9.0, 7.0);

    let fit = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    assert_eq!(fit.rejected, vec![u32::try_from(last).unwrap()]);
    let error = (fit.rigid.apply(moving[0]) - pose().apply(moving[0])).length();
    assert!(error < 1e-6, "the outlier leaked into the fit: {error}");
}

#[test]
fn a_clean_fit_rejects_nothing_even_at_floating_point_noise() {
    let moving = spread();
    let fixed = posed(&moving);
    let fit = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    assert!(
        fit.rejected.is_empty(),
        "residual noise must not look like an outlier: {:?}",
        fit.rejected
    );
}

#[test]
fn a_reflection_is_never_returned() {
    let moving = spread();
    let mirrored: Vec<DVec3> = moving
        .iter()
        .map(|point| DVec3::new(-point.x, point.y, point.z))
        .collect();
    if let Ok(fit) = fit_pairs(&moving, &mirrored, None, 40.0) {
        let basis = DMat3::from_quat(fit.rigid.rotation);
        assert!(
            basis.determinant() > 0.0,
            "a reflection escaped the fit: {}",
            basis.determinant()
        );
    }
}

#[test]
fn a_runaway_fit_is_refused() {
    let moving = spread();
    let fixed: Vec<DVec3> = moving
        .iter()
        .map(|&point| point + DVec3::new(5000.0, 0.0, 0.0))
        .collect();
    let outcome = fit_pairs(&moving, &fixed, None, 40.0);
    match outcome {
        Err(FitRejection::Runaway { moved_by, allowed }) => {
            assert!(moved_by > allowed, "{moved_by} should exceed {allowed}");
        }
        other => assert!(
            matches!(other, Err(FitRejection::Runaway { .. })),
            "expected Runaway, got {other:?}"
        ),
    }
}

#[test]
fn two_pairs_with_normals_produce_a_defined_frame() {
    let moving = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(6.0, 0.0, 0.0)];
    let moving_normals = vec![DVec3::Z, DVec3::Z];
    let fixed = posed(&moving);
    let fixed_normals: Vec<DVec3> = moving_normals
        .iter()
        .map(|&normal| pose().apply_normal(normal))
        .collect();

    let fit = fit_pairs(
        &moving,
        &fixed,
        Some((&moving_normals, &fixed_normals)),
        40.0,
    )
    .unwrap();
    for point in &moving {
        let error = (fit.rigid.apply(*point) - pose().apply(*point)).length();
        assert!(error < 1e-6, "two-pair frame is off by {error}");
    }
}

#[test]
fn two_pairs_without_normals_are_refused() {
    let moving = vec![DVec3::ZERO, DVec3::new(6.0, 0.0, 0.0)];
    let fixed = posed(&moving);
    let outcome = fit_pairs(&moving, &fixed, None, 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::Degenerate { .. })),
        "two points alone cannot determine a rotation, got {outcome:?}"
    );
}

#[test]
fn two_pairs_whose_normal_lies_along_the_segment_are_refused() {
    let moving = vec![DVec3::ZERO, DVec3::new(6.0, 0.0, 0.0)];
    let normals = vec![DVec3::X, DVec3::X];
    let fixed = posed(&moving);
    let fixed_normals: Vec<DVec3> = normals
        .iter()
        .map(|&normal| pose().apply_normal(normal))
        .collect();
    let outcome = fit_pairs(&moving, &fixed, Some((&normals, &fixed_normals)), 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::Degenerate { .. })),
        "a normal along the segment leaves the roll free, got {outcome:?}"
    );
}

#[test]
fn non_finite_input_is_refused() {
    let moving = vec![
        DVec3::new(f64::NAN, 0.0, 0.0),
        DVec3::ONE,
        DVec3::X,
        DVec3::Y,
    ];
    let outcome = fit_pairs(&moving, &spread(), None, 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::NonFinite)),
        "expected NonFinite, got {outcome:?}"
    );
}

#[test]
fn mismatched_lengths_are_refused_rather_than_truncated() {
    let outcome = fit_pairs(&spread(), &spread()[..2], None, 40.0);
    assert!(
        matches!(
            outcome,
            Err(FitRejection::Unpaired {
                moving: 4,
                fixed: 2
            })
        ),
        "silently dropping a clicked point would be worse than refusing, got {outcome:?}"
    );
}

#[test]
fn the_fit_is_bit_identical_across_repeats() {
    let moving = spread();
    let fixed = posed(&moving);
    let first = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    let second = fit_pairs(&moving, &fixed, None, 40.0).unwrap();
    assert_eq!(
        first.rigid.translation.to_array(),
        second.rigid.translation.to_array()
    );
    assert_eq!(
        first.rigid.rotation.to_array(),
        second.rigid.rotation.to_array()
    );
    assert_eq!(first.rejected, second.rejected);
}

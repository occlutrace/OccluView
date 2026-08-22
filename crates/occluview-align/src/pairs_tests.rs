//! Tests for the point-pair fit, split out of `pairs.rs` to hold the
//! workspace's file budget.

use crate::pairs::{fit_pairs, FitBounds, FitRejection, PairFit};
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

/// Centre of a point set — where a mesh sitting on those points would be.
fn centre(points: &[DVec3]) -> DVec3 {
    if points.is_empty() {
        return DVec3::ZERO;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = points.len() as f64;
    points.iter().fold(DVec3::ZERO, |sum, point| sum + *point) / count
}

/// A fit set up the way the worker sets one up: each mesh sitting on its own
/// clicked points, both `extent` millimetres across.
fn fit(
    moving: &[DVec3],
    fixed: &[DVec3],
    normals: Option<(&[DVec3], &[DVec3])>,
    extent: f64,
) -> Result<PairFit, FitRejection> {
    fit_pairs(
        moving,
        fixed,
        normals,
        &FitBounds {
            moving_center: centre(moving),
            moving_extent: extent,
            fixed_center: centre(fixed),
            fixed_extent: extent,
        },
    )
}

fn posed(points: &[DVec3]) -> Vec<DVec3> {
    points.iter().map(|&point| pose().apply(point)).collect()
}

#[test]
fn four_clean_pairs_recover_the_transform() {
    let moving = spread();
    let fixed = posed(&moving);
    let fit = fit(&moving, &fixed, None, 40.0).unwrap();
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
    let fit = fit(&moving, &fixed, None, 40.0).unwrap();
    assert!(fit.pair_rms < 1e-9);
}

#[test]
fn fewer_than_two_pairs_is_refused() {
    let outcome = fit(&[DVec3::ZERO], &[DVec3::ONE], None, 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::TooFewPairs { have: 1, need: 2 })),
        "expected TooFewPairs, got {outcome:?}"
    );
}

#[test]
fn collinear_pairs_are_refused_with_a_weak_axis() {
    let moving: Vec<DVec3> = (0..5).map(|k| DVec3::new(f64::from(k), 0.0, 0.0)).collect();
    let fixed = posed(&moving);
    let outcome = fit(&moving, &fixed, None, 40.0);
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
    let outcome = fit(&moving, &fixed, None, 400.0);
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

    let fit = fit(&moving, &fixed, None, 40.0).unwrap();
    assert_eq!(fit.rejected, vec![u32::try_from(last).unwrap()]);
    let error = (fit.rigid.apply(moving[0]) - pose().apply(moving[0])).length();
    assert!(error < 1e-6, "the outlier leaked into the fit: {error}");
}

#[test]
fn a_clean_fit_rejects_nothing_even_at_floating_point_noise() {
    let moving = spread();
    let fixed = posed(&moving);
    let fit = fit(&moving, &fixed, None, 40.0).unwrap();
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
    if let Ok(fit) = fit(&moving, &mirrored, None, 40.0) {
        let basis = DMat3::from_quat(fit.rigid.rotation);
        assert!(
            basis.determinant() > 0.0,
            "a reflection escaped the fit: {}",
            basis.determinant()
        );
    }
}

#[test]
fn a_fit_that_throws_the_scan_clear_of_its_partner_is_refused() {
    // Both scans sit at the origin, 40 mm across. The clicked pairs ask for a
    // pose that parks the moving one 300 mm away: whatever the operator meant
    // to click, the answer is not a registration, because the two scans no
    // longer touch.
    let moving = spread();
    let fixed: Vec<DVec3> = moving
        .iter()
        .map(|&point| point + DVec3::new(300.0, 0.0, 0.0))
        .collect();
    let bounds = FitBounds {
        moving_center: centre(&moving),
        moving_extent: 40.0,
        fixed_center: centre(&moving),
        fixed_extent: 40.0,
    };

    let outcome = fit_pairs(&moving, &fixed, None, &bounds);

    match outcome {
        Err(FitRejection::Apart {
            separation,
            allowed,
        }) => assert!(separation > allowed, "{separation} should exceed {allowed}"),
        other => assert!(
            matches!(other, Err(FitRejection::Apart { .. })),
            "expected Apart, got {other:?}"
        ),
    }
}

#[test]
fn turning_a_scan_over_where_it_stands_is_not_a_runaway() {
    // The case the guard used to refuse, at full-arch scale: an 88 mm bounding-box
    // diagonal whose vertices sit 71 mm from the file's own zero — the shape
    // real exports take. The partner scan is the same arch 2.3 mm away, stored with the
    // opposite occlusal convention — the ordinary difference between a .dcm
    // written by one system and an .stl written by another. Nothing has to
    // travel; the scan only has to turn over.
    const EXTENT: f64 = 88.23;
    let content = DVec3::new(3.0, -17.51, 68.53);
    let axis = content.cross(DVec3::X).normalize();
    let over = DQuat::from_axis_angle(axis, std::f64::consts::PI);
    let landing = content + DVec3::new(2.0, 1.0, 0.5);
    let truth = Rigid::new(over, landing - over * content);

    let moving: Vec<DVec3> = spread().iter().map(|point| *point + content).collect();
    let fixed: Vec<DVec3> = moving.iter().map(|point| truth.apply(*point)).collect();
    let bounds = FitBounds {
        moving_center: content,
        moving_extent: EXTENT,
        fixed_center: landing,
        fixed_extent: EXTENT,
    };

    let outcome = fit_pairs(&moving, &fixed, None, &bounds);

    let travelled = (truth.apply(content) - content).length();
    assert!(
        travelled < 3.0,
        "the scan should barely move, got {travelled}"
    );
    assert!(
        outcome.is_ok(),
        "the scan travels {travelled:.1} mm and lands on its partner, \
         yet the fit was refused: {outcome:?}"
    );
}

#[test]
fn where_the_file_puts_its_zero_does_not_change_the_verdict() {
    // A file's origin is the scanner's bookkeeping, not a fact about the
    // patient, and nothing downstream may depend on it at any distance.
    // Re-express the moving scan in a frame whose zero has been pushed away —
    // the same two scans, the same registration, the same sizes — and the
    // guard has to keep reaching the same conclusion.
    //
    // The distances are the ones the formats actually produce: on top of
    // the geometry, the ~71 mm a full-arch STL routinely carries, and
    // the hundreds of millimetres a surface carries when it is quoted in the
    // patient coordinates of the volume it was lifted from.
    const EXTENT: f64 = 88.23;
    let moving = spread();
    let fixed = posed(&moving);

    for zero in [0.0, 70.0, 400.0, 2000.0] {
        let elsewhere = DVec3::new(3.0, -17.51, 68.53).normalize_or_zero() * zero;
        let shifted: Vec<DVec3> = moving.iter().map(|point| *point + elsewhere).collect();
        let bounds = FitBounds {
            moving_center: centre(&shifted),
            moving_extent: EXTENT,
            fixed_center: centre(&fixed),
            fixed_extent: EXTENT,
        };

        let outcome = fit_pairs(&shifted, &fixed, None, &bounds);

        assert!(
            outcome.is_ok(),
            "the same registration was refused with the file's zero {zero} mm away: {outcome:?}"
        );
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

    let fit = fit(
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
    let outcome = fit(&moving, &fixed, None, 40.0);
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
    let outcome = fit(&moving, &fixed, Some((&normals, &fixed_normals)), 40.0);
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
    let outcome = fit(&moving, &spread(), None, 40.0);
    assert!(
        matches!(outcome, Err(FitRejection::NonFinite)),
        "expected NonFinite, got {outcome:?}"
    );
}

#[test]
fn mismatched_lengths_are_refused_rather_than_truncated() {
    let outcome = fit(&spread(), &spread()[..2], None, 40.0);
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
    let first = fit(&moving, &fixed, None, 40.0).unwrap();
    let second = fit(&moving, &fixed, None, 40.0).unwrap();
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

#[test]
fn collinear_fixed_clicks_are_refused_like_collinear_moving_ones() {
    // The degeneracy is symmetric. Four spread clicks against four on a line
    // leave the rotation about that line free, and an arbitrary member of the
    // circle of equally good rotations must not come back as a confident fit.
    let moving = spread();
    let fixed: Vec<DVec3> = (0..4)
        .map(|step| DVec3::new(f64::from(step) * 3.0, 0.0, 0.0))
        .collect();

    let outcome = fit(&moving, &fixed, None, 40.0);

    assert!(
        matches!(outcome, Err(FitRejection::Degenerate { .. })),
        "a line of fixed clicks determines no rotation, got {outcome:?}"
    );
}

#[test]
fn one_wild_click_is_an_outlier_not_a_unit_problem() {
    // A click on the wrong side of an arch lands ~60 mm off. Gating the unit
    // check on MEAN pairwise distances read that as "3x apart in size —
    // probably different units" and sent the operator to import scaling,
    // while the trimming loop that exists for exactly that click never ran.
    // Seven clean anchors, so the fit cannot tilt far enough to absorb the
    // wild click into everyone's residuals.
    let mut moving = spread();
    moving.extend([
        DVec3::new(3.0, 2.0, -4.0),
        DVec3::new(-5.0, 4.0, 2.0),
        DVec3::new(8.0, -6.0, -3.0),
    ]);
    let mut fixed = posed(&moving);
    fixed[6] += DVec3::new(60.0, 0.0, 0.0);

    let fit = fit(&moving, &fixed, None, 40.0).unwrap();

    assert_eq!(fit.rejected, vec![6], "the wild click is the one to drop");
    assert!(fit.pair_rms < 1e-6, "the surviving pairs fit cleanly");
}

#[test]
fn the_overlap_allowance_sits_exactly_at_touching_spheres() {
    // Pin the guard's SHAPE, not only its origin-independence: the boundary
    // is the sum of the two bounding-sphere radii. A fit landing just inside
    // passes; just outside is refused; and two tiny scans fall back to the
    // one-millimetre floor.
    let points = spread();
    let bounds_at = |gap: f64, extent: f64| FitBounds {
        moving_center: centre(&points),
        moving_extent: extent,
        fixed_center: centre(&points) + DVec3::X * gap,
        fixed_extent: extent,
    };

    // Identical point sets fit to the identity, so the separation IS the gap.
    assert!(
        fit_pairs(&points, &points, None, &bounds_at(39.9, 40.0)).is_ok(),
        "just inside touching must pass"
    );
    assert!(
        matches!(
            fit_pairs(&points, &points, None, &bounds_at(40.1, 40.0)),
            Err(FitRejection::Apart { .. })
        ),
        "just outside touching must be refused"
    );
    assert!(
        fit_pairs(&points, &points, None, &bounds_at(0.9, 0.2)).is_ok(),
        "tiny scans judge against the floor, not their own size"
    );
    assert!(
        matches!(
            fit_pairs(&points, &points, None, &bounds_at(1.1, 0.2)),
            Err(FitRejection::Apart { .. })
        ),
        "past the floor is a miss even for tiny scans"
    );
}

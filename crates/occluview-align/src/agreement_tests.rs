//! Tests for the symmetric measure, split out of `agreement.rs` to hold the
//! workspace's file budget.
//!
//! The fixture that matters here is the one-sided blindness case: a moving mesh
//! missing a region the fixed mesh has. A one-sided map calls that a perfect
//! fit, and every assertion below exists because that must never again reach an
//! operator as a passing number.
#![allow(clippy::expect_used)]

use crate::agreement::{reverse_deviation, surface_agreement};
use crate::{deviation, CancelFlag, DeviationSettings, Orientation, Rigid, Soup, SurfaceIndex};

/// A gently curved sheet spanning `-10..10` in both axes.
fn dome(steps: usize) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity((steps + 1) * (steps + 1) * 3);
    for row in 0..=steps {
        for col in 0..=steps {
            #[allow(clippy::cast_precision_loss)]
            let x = col as f32 / steps as f32 * 20.0 - 10.0;
            #[allow(clippy::cast_precision_loss)]
            let y = row as f32 / steps as f32 * 20.0 - 10.0;
            positions.extend_from_slice(&[x, y, 0.02 * x * x + 0.015 * y * y]);
        }
    }
    let mut indices = Vec::with_capacity(steps * steps * 6);
    let stride = u32::try_from(steps + 1).unwrap();
    let span = u32::try_from(steps).unwrap();
    for row in 0..span {
        for col in 0..span {
            let corner = row * stride + col;
            indices.extend_from_slice(&[corner, corner + 1, corner + stride]);
            indices.extend_from_slice(&[corner + 1, corner + stride + 1, corner + stride]);
        }
    }
    (positions, indices)
}

/// The same sheet with every triangle inside a central square dropped.
fn holed(positions: &[f32], indices: &[u32], half: f32) -> (Vec<f32>, Vec<u32>) {
    let mut out_positions = Vec::new();
    let mut out_indices = Vec::new();
    for triangle in indices.chunks_exact(3) {
        let corners: Vec<[f32; 3]> = triangle
            .iter()
            .map(|slot| {
                let at = *slot as usize * 3;
                [positions[at], positions[at + 1], positions[at + 2]]
            })
            .collect();
        if corners
            .iter()
            .all(|corner| corner[0].abs() < half && corner[1].abs() < half)
        {
            continue;
        }
        let first = u32::try_from(out_positions.len() / 3).unwrap();
        for corner in corners {
            out_positions.extend_from_slice(&corner);
        }
        out_indices.extend_from_slice(&[first, first + 1, first + 2]);
    }
    (out_positions, out_indices)
}

fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

fn settings() -> DeviationSettings {
    DeviationSettings {
        influence_radius_mm: 5.0,
        orientation: Orientation::Match,
    }
}

/// Both directions and the summary for one pose.
fn compare(moving: Soup<'_>, fixed: Soup<'_>, pose: Rigid) -> crate::SurfaceAgreement {
    let moving_index = SurfaceIndex::build(moving).unwrap();
    let fixed_index = SurfaceIndex::build(fixed).unwrap();
    let cancel = CancelFlag::new();
    let forward = deviation(moving, &fixed_index, pose, &settings(), &cancel);
    let backward = reverse_deviation(fixed, &moving_index, pose, &settings(), &cancel);
    surface_agreement(&forward, &backward, 0.2)
}

#[test]
fn a_moving_scan_missing_a_region_no_longer_reports_a_perfect_fit() {
    let (positions, indices) = dome(40);
    let (cropped_positions, cropped_indices) = holed(&positions, &indices, 6.0);
    let full = soup(&positions, &indices);
    let cropped = soup(&cropped_positions, &cropped_indices);

    let agreement = compare(cropped, full, Rigid::IDENTITY);

    let moving_to_fixed = agreement
        .moving_to_fixed
        .summary
        .expect("the cropped dome(40) still clears MIN_MEASURED forward");
    let fixed_to_moving = agreement
        .fixed_to_moving
        .summary
        .expect("the full dome(40) still clears MIN_MEASURED backward");
    assert!(
        moving_to_fixed.mean_abs < 1e-4,
        "the one-sided direction is supposed to be blind here: {}",
        moving_to_fixed.mean_abs
    );
    assert!(
        fixed_to_moving.mean_abs > 0.05,
        "the reverse direction must see the missing region, got {}",
        fixed_to_moving.mean_abs
    );
    assert!(
        agreement.hausdorff > 1.0,
        "a 12 mm hole must show as a large symmetric Hausdorff, got {}",
        agreement.hausdorff
    );
    let asymmetry = agreement
        .asymmetry_mm()
        .expect("both directions measured enough here for the summary to exist");
    assert!(
        asymmetry > 0.05,
        "the two directions must not agree here, got {asymmetry}"
    );
    assert!(
        agreement.within_tolerance < 0.999,
        "a hole cannot be 100% within tolerance, got {}",
        agreement.within_tolerance
    );
}

#[test]
fn two_copies_of_one_surface_agree_in_both_directions() {
    let (positions, indices) = dome(24);
    let mesh = soup(&positions, &indices);
    let agreement = compare(mesh, mesh, Rigid::IDENTITY);

    assert!(agreement.mean_abs < 1e-5, "{}", agreement.mean_abs);
    assert!(agreement.hausdorff < 1e-4, "{}", agreement.hausdorff);
    let asymmetry = agreement
        .asymmetry_mm()
        .expect("dome(24) clears MIN_MEASURED on both sides");
    assert!(asymmetry < 1e-5, "{asymmetry}");
    assert!((agreement.within_tolerance - 1.0).abs() < 1e-9);
}

#[test]
fn the_reverse_direction_keeps_the_forward_sign_convention() {
    // Lift the moving sheet along its own outward normal: the forward map calls
    // that positive, and the reverse must agree rather than mirror it.
    let (positions, indices) = dome(16);
    let lifted: Vec<f32> = positions
        .chunks_exact(3)
        .flat_map(|point| [point[0], point[1], point[2] + 0.3])
        .collect();
    let fixed = soup(&positions, &indices);
    let moving = soup(&lifted, &indices);

    let moving_index = SurfaceIndex::build(moving).unwrap();
    let fixed_index = SurfaceIndex::build(fixed).unwrap();
    let cancel = CancelFlag::new();
    let forward = deviation(moving, &fixed_index, Rigid::IDENTITY, &settings(), &cancel);
    let backward = reverse_deviation(fixed, &moving_index, Rigid::IDENTITY, &settings(), &cancel);

    let forward_median = crate::deviation_stats(&forward, 0.2)
        .summary
        .expect("dome(16) clears MIN_MEASURED")
        .median;
    let backward_median = crate::deviation_stats(&backward, 0.2)
        .summary
        .expect("dome(16) clears MIN_MEASURED")
        .median;
    assert!(forward_median > 0.2, "forward {forward_median}");
    assert!(
        backward_median > 0.2,
        "the reverse direction flipped the sign: {backward_median}"
    );
}

#[test]
fn symmetry_does_not_rescue_a_tangential_offset() {
    // A cylinder slid along its own axis. Both directions are blind, and the
    // summary must not pretend otherwise: this is the case that needs
    // `observability`, not a second direction.
    let (positions, indices) = cylinder(5.0, 24.0, 96, 40);
    let mesh = soup(&positions, &indices);
    let pose = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::Z * 0.30);

    let agreement = compare(mesh, mesh, pose);

    assert!(
        agreement.mean_abs < 0.05,
        "a symmetric measure cannot see an axial slide either, and this test \
         pins that so nobody claims it does: {}",
        agreement.mean_abs
    );
    let asymmetry = agreement
        .asymmetry_mm()
        .expect("the cylinder fixture clears MIN_MEASURED on both sides");
    assert!(
        asymmetry < 0.01,
        "both directions are equally blind here: {asymmetry}"
    );
}

#[test]
fn an_unmeasurable_pair_summarizes_to_zero_rather_than_to_nonsense() {
    let (positions, indices) = dome(8);
    let far: Vec<f32> = positions
        .chunks_exact(3)
        .flat_map(|point| [point[0], point[1], point[2] + 500.0])
        .collect();
    let agreement = compare(
        soup(&far, &indices),
        soup(&positions, &indices),
        Rigid::IDENTITY,
    );

    assert_eq!(agreement.measured, 0);
    assert!(agreement.skipped > 0);
    assert_eq!(agreement.mean_abs, 0.0);
    assert_eq!(agreement.hausdorff, 0.0);
}

#[test]
fn the_summary_is_identical_whatever_thread_count_produced_the_maps() {
    let (positions, indices) = dome(30);
    let mesh = soup(&positions, &indices);
    let pose = Rigid::new(
        glam::DQuat::from_axis_angle(glam::DVec3::X, 0.01),
        glam::DVec3::new(0.05, -0.03, 0.02),
    );

    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| compare(mesh, mesh, pose))
    };
    assert_eq!(run(1), run(4), "the symmetric summary must not drift");
    assert_eq!(run(1), run(7));
}

/// A closed cylinder wall about the Z axis, centred on the origin.
fn cylinder(radius: f32, length: f32, around: usize, along: usize) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity(around * along * 3);
    for ring in 0..along {
        #[allow(clippy::cast_precision_loss)]
        let z = ring as f32 / (along - 1) as f32 * length - length * 0.5;
        for step in 0..around {
            #[allow(clippy::cast_precision_loss)]
            let angle = step as f32 / around as f32 * std::f32::consts::TAU;
            positions.extend_from_slice(&[radius * angle.cos(), radius * angle.sin(), z]);
        }
    }
    let mut indices = Vec::with_capacity(around * (along - 1) * 6);
    let stride = u32::try_from(around).unwrap();
    for ring in 0..u32::try_from(along - 1).unwrap() {
        for step in 0..stride {
            let next = (step + 1) % stride;
            let corner = ring * stride + step;
            let neighbour = ring * stride + next;
            indices.extend_from_slice(&[corner, neighbour, corner + stride]);
            indices.extend_from_slice(&[neighbour, neighbour + stride, corner + stride]);
        }
    }
    (positions, indices)
}

//! Tests for the refine stage, split out of `icp.rs` to hold the workspace's
//! file budget.

use crate::icp::{refine, IcpReport, Orientation, RefineSettings};
use crate::{CancelFlag, FitRejection, Rigid, Soup, SurfaceIndex};
use glam::{DQuat, DVec3};

/// A shallow dome with quasi-random surface texture on top.
///
/// Curvature is what makes this a real ICP fixture: a flat sheet slides freely
/// in plane, so a test on one would pass whatever the solver did. The two
/// curvature coefficients differ so the dome is not rotationally symmetric
/// either.
fn dome(n: usize, step: f32) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    #[allow(clippy::cast_precision_loss)]
    let centre = n as f32 * step * 0.5;
    for j in 0..=n {
        for i in 0..=n {
            #[allow(clippy::cast_precision_loss)]
            let x = i as f32 * step;
            #[allow(clippy::cast_precision_loss)]
            let y = j as f32 * step;
            let (dx, dy) = (x - centre, y - centre);
            #[allow(clippy::cast_precision_loss)]
            let texture = ((i * 5 + j * 3) % 7) as f32 * 0.02;
            positions.extend_from_slice(&[x, y, 0.05 * dx * dx + 0.04 * dy * dy + texture]);
        }
    }
    (positions, grid_indices(n))
}

/// The same grid with every height at zero — the fixture for the sliding case.
fn flat(n: usize, step: f32) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    for j in 0..=n {
        for i in 0..=n {
            #[allow(clippy::cast_precision_loss)]
            positions.extend_from_slice(&[i as f32 * step, j as f32 * step, 0.0]);
        }
    }
    (positions, grid_indices(n))
}

fn grid_indices(n: usize) -> Vec<u32> {
    let mut indices = Vec::with_capacity(n * n * 6);
    let stride = u32::try_from(n + 1).unwrap();
    let span = u32::try_from(n).unwrap();
    for j in 0..span {
        for i in 0..span {
            let a = j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride]);
            indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
        }
    }
    indices
}

/// Append a translated copy of a component without welding it to the first
/// one. This models an arch made of separate teeth: a global fixed bounding box
/// has a centre in the gap, while the matching component is still local.
#[allow(clippy::cast_possible_truncation)]
fn append_component(
    positions: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    component_positions: &[f32],
    component_indices: &[u32],
    offset: DVec3,
) {
    let base = u32::try_from(positions.len() / 3).unwrap();
    positions.extend(
        component_positions
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|point| {
                [
                    (f64::from(point[0]) + offset.x) as f32,
                    (f64::from(point[1]) + offset.y) as f32,
                    (f64::from(point[2]) + offset.z) as f32,
                ]
            }),
    );
    indices.extend(component_indices.iter().map(|index| base + *index));
}

fn settings() -> RefineSettings {
    RefineSettings {
        influence_radius_mm: 2.0,
        matching_ratio: 0.8,
        orientation: Orientation::Match,
        max_iterations: 40,
    }
}

fn trustworthy_report() -> IcpReport {
    IcpReport {
        rigid: Rigid::IDENTITY,
        iterations: 4,
        converged: true,
        inliers: 800,
        inlier_ratio: 0.8,
        coverage: 0.8,
        rms: 0.02,
        median_abs: 0.01,
        p95_abs: 0.05,
        weak_rot_axes: [false; 3],
        weak_trans_axes: [false; 3],
    }
}

#[test]
fn only_converged_full_rank_coverage_can_authorize_refinement() {
    assert!(trustworthy_report().is_trustworthy_refinement());

    let mut stalled = trustworthy_report();
    stalled.converged = false;
    assert!(!stalled.is_trustworthy_refinement());

    let mut local_patch = trustworthy_report();
    local_patch.coverage = 0.01;
    assert!(!local_patch.is_trustworthy_refinement());

    let mut rank_deficient = trustworthy_report();
    rank_deficient.weak_trans_axes[0] = true;
    assert!(!rank_deficient.is_trustworthy_refinement());
}

fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

/// The same vertices, re-quoted in a frame whose zero sits `delta` away.
///
/// Nothing about the surface changes — only the arbitrary point the file
/// counts from.
#[allow(clippy::cast_possible_truncation)]
fn requoted(positions: &[f32], delta: DVec3) -> Vec<f32> {
    positions
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|vertex| {
            [
                (f64::from(vertex[0]) + delta.x) as f32,
                (f64::from(vertex[1]) + delta.y) as f32,
                (f64::from(vertex[2]) + delta.z) as f32,
            ]
        })
        .collect()
}

#[test]
fn refine_closes_a_small_offset() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(
        DQuat::from_axis_angle(DVec3::Z, 0.02),
        DVec3::new(0.25, -0.18, 0.12),
    );

    let report = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();

    assert!(report.rms < 0.02, "residual {} is too high", report.rms);
    assert!(report.inlier_ratio > 0.7, "ratio {}", report.inlier_ratio);
    assert!(
        report.rigid.translation.length() < 0.05,
        "the pose did not come home: {:?}",
        report.rigid.translation
    );
}

#[test]
fn refine_leaves_an_already_seated_pose_alone() {
    let (positions, indices) = dome(16, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();

    let report = refine(
        mesh,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    assert!(report.rigid.translation.length() < 1e-6);
    assert!(report.rms < 1e-6);
}

#[test]
fn refine_reports_a_weak_axis_on_a_flat_sheet() {
    let (positions, indices) = flat(16, 1.0);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();

    let report = refine(
        mesh,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    assert!(
        report.weak_trans_axes[0] || report.weak_trans_axes[1],
        "a flat sheet slides in plane and must say so: {:?}",
        report.weak_trans_axes
    );
    assert!(
        !report.weak_trans_axes[2],
        "the sheet normal direction is well determined"
    );
}

#[test]
fn refine_stops_when_already_cancelled() {
    let (positions, indices) = dome(24, 0.25);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let cancel = CancelFlag::new();
    cancel.cancel();

    let report = refine(mesh, &index, Rigid::IDENTITY, &settings(), &cancel).unwrap();

    assert_eq!(report.iterations, 0);
    assert!(!report.converged);
}

#[test]
fn refine_rejects_a_nonfinite_start_before_cancellation_can_hide_it() {
    let (positions, indices) = dome(12, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let cancel = CancelFlag::new();
    cancel.cancel();
    let broken = Rigid {
        rotation: DQuat::IDENTITY,
        translation: DVec3::new(f64::NAN, 0.0, 0.0),
    };

    assert_eq!(
        refine(mesh, &index, broken, &settings(), &cancel),
        Err(FitRejection::NonFinite)
    );
}

#[test]
fn refine_is_bit_identical_across_repeats() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(0.2, 0.1, 0.05));

    let first = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();
    let second = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();

    assert_eq!(
        first.rigid.translation.to_array(),
        second.rigid.translation.to_array()
    );
    assert_eq!(
        first.rigid.rotation.to_array(),
        second.rigid.rotation.to_array()
    );
    assert_eq!(first.iterations, second.iterations);
    assert_eq!(first.inliers, second.inliers);
}

#[test]
fn refine_is_bit_identical_across_thread_counts() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(
        DQuat::from_axis_angle(DVec3::Y, 0.015),
        DVec3::new(0.2, -0.1, 0.08),
    );

    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap())
    };

    let single = run(1);
    let many = run(8);

    assert_eq!(
        single.rigid.translation.to_array(),
        many.rigid.translation.to_array(),
        "one thread and eight must land on the same pose, to the bit"
    );
    assert_eq!(
        single.rigid.rotation.to_array(),
        many.rigid.rotation.to_array()
    );
    assert_eq!(single.iterations, many.iterations);
    assert_eq!(single.rms.to_bits(), many.rms.to_bits());
}

#[test]
fn the_mask_removes_vertices_from_the_fit() {
    let (positions, indices) = dome(16, 0.5);
    let plain = soup(&positions, &indices);
    let mut mask = vec![0u8; plain.vertex_count()];
    // Exclude one local patch while leaving most triangles usable. Masking
    // every other grid vertex would remove every face from this alternating
    // triangulation: a masked face has no valid normal by contract, so the
    // test would accidentally ask ICP to fit an empty moving surface.
    for vertex in [0usize, 1, 17, 18] {
        mask[vertex] = 1;
    }
    let masked = Soup {
        positions: &positions,
        indices: &indices,
        mask: Some(&mask),
    };
    let index = SurfaceIndex::build(plain).unwrap();
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(0.15, 0.0, 0.1));

    let with_mask = refine(masked, &index, start, &settings(), &CancelFlag::new()).unwrap();
    let without = refine(plain, &index, start, &settings(), &CancelFlag::new()).unwrap();

    assert!(
        with_mask.inliers < without.inliers,
        "the mask changed nothing: {} vs {}",
        with_mask.inliers,
        without.inliers
    );
}

#[test]
fn a_start_with_no_surface_in_reach_is_refused() {
    let (positions, indices) = dome(8, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(900.0, 0.0, 0.0));

    let outcome = refine(mesh, &index, start, &settings(), &CancelFlag::new());

    assert!(
        outcome.is_err(),
        "a hopeless start must be refused, not guessed at"
    );
}

#[test]
fn an_inverted_orientation_setting_rejects_matching_normals() {
    let (positions, indices) = dome(12, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let inverted = RefineSettings {
        orientation: Orientation::Inverted,
        ..settings()
    };

    let outcome = refine(mesh, &index, Rigid::IDENTITY, &inverted, &CancelFlag::new());

    assert!(
        outcome.is_err(),
        "every normal agrees, so an inverted-only match has nothing to work with"
    );
}

#[test]
fn an_empty_moving_mesh_is_refused() {
    let (positions, indices) = dome(8, 0.5);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let empty = Soup {
        positions: &[],
        indices: &[],
        mask: None,
    };

    let outcome = refine(
        empty,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    );

    assert!(outcome.is_err());
}

#[test]
fn where_the_file_puts_its_zero_does_not_change_the_refine() {
    // A scan's file coordinates say nothing about where the scanner's zero
    // was: a surface lifted out of a DICOM volume is quoted in patient
    // coordinates, hundreds of millimetres from the anatomy, while an STL off
    // the same case sits on top of its own origin. Re-quote the moving mesh in
    // such a frame and compensate in the start pose, and the refine sees a
    // bit-identical surface in a bit-identical place. It has to answer the
    // same — the guard included.
    //
    // The pose's translation column does NOT stay the same: turning the mesh
    // through a small angle swings it by twice the distance to the file's
    // zero, which here is far more than the mesh's own size. Reading that as
    // "how far the scan moved" is what refused refines that had not moved the
    // scan at all.
    // Across the turn axis, not along it: an offset parallel to the axis
    // survives the rotation untouched and would leave the two numbers below
    // identical, testing nothing.
    let elsewhere = DVec3::new(2000.0, 0.0, 0.0);
    let (positions, indices) = dome(24, 0.5);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let start = Rigid::new(
        DQuat::from_axis_angle(DVec3::Z, 0.02),
        DVec3::new(0.25, -0.18, 0.12),
    );

    // Both refines must land. An unwrap here prints the refusal itself, which
    // is the part worth reading: a `Runaway` is the regression this test is
    // for, and any other rejection is a different bug.
    let here = refine(
        soup(&positions, &indices),
        &index,
        start,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    let requoted_positions = requoted(&positions, elsewhere);
    let compensated = Rigid::new(
        start.rotation,
        start.translation - start.rotation * elsewhere,
    );
    let there = refine(
        soup(&requoted_positions, &indices),
        &index,
        compensated,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    // Read the second answer back in the first one's frame.
    // The offset is a whole multiple of the grid step, so requoting is
    // bit-exact in `f32` and any drift is the solver's own — measured at
    // ~1e-13 mm. The bound stays a loose micron so the test pins the
    // regression, not one fixture's noise floor.
    let read_back = there.rigid.translation + there.rigid.rotation * elsewhere;
    let drift = (read_back - here.rigid.translation).length();
    assert!(
        drift < 1e-3,
        "the same surface in the same place reached a different pose, off by {drift} mm"
    );

    // The guard must measure scan travel rather than translation-column change.
    let bookkeeping = (there.rigid.translation - compensated.translation).length();
    let travelled = (here.rigid.translation - start.translation).length();
    assert!(
        bookkeeping > travelled * 5.0,
        "expected the translation column to swing wider than the scan moved, \
         got {bookkeeping} against {travelled}"
    );
}

#[test]
fn best_fit_recovers_from_a_one_mm_lateral_start() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(1.0, 0.0, 0.0));

    let report = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();

    assert!(
        report.rigid.translation.length() < 0.05,
        "a rough lateral start settled sideways at {:?}",
        report.rigid.translation
    );
}

#[test]
fn best_fit_recovers_from_a_quarter_turn_start() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let rotation = DQuat::from_axis_angle(DVec3::Z, std::f64::consts::FRAC_PI_2);
    let centre = DVec3::splat(6.0);
    let start = Rigid::new(rotation, centre - rotation * centre);

    let report = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();
    let remaining_rotation = report.rigid.rotation.to_scaled_axis().length();

    assert!(
        remaining_rotation < 0.05,
        "a rough quarter-turn start stayed sideways: {:?}",
        report.rigid
    );
    assert!(
        report.rigid.translation.length() < 0.05,
        "a rough quarter-turn start did not return to the source pose: {:?}",
        report.rigid.translation
    );
}

#[test]
fn best_fit_prefers_the_nearby_component_over_an_adjacent_distractor() {
    let (component, component_indices) = dome(24, 0.5);
    let mut fixed = component.clone();
    let mut fixed_indices = component_indices.clone();
    append_component(
        &mut fixed,
        &mut fixed_indices,
        &component,
        &component_indices,
        DVec3::new(14.0, 0.0, 0.0),
    );
    let moving = soup(&component, &component_indices);
    let fixed_index = SurfaceIndex::build(soup(&fixed, &fixed_indices)).unwrap();
    assert_eq!(fixed_index.component_bounds().len(), 2);
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(1.0, 0.0, 0.0));

    let report = refine(moving, &fixed_index, start, &settings(), &CancelFlag::new()).unwrap();

    assert!(
        report.rigid.translation.length() < 0.05,
        "the adjacent component stole the fit: {:?}",
        report.rigid.translation
    );
}

#[test]
fn best_fit_refuses_equally_plausible_disconnected_components() {
    let (component, component_indices) = dome(24, 0.5);
    let mut fixed = component.clone();
    let mut fixed_indices = component_indices.clone();
    append_component(
        &mut fixed,
        &mut fixed_indices,
        &component,
        &component_indices,
        DVec3::new(14.0, 0.0, 0.0),
    );
    let moving = soup(&component, &component_indices);
    let fixed_index = SurfaceIndex::build(soup(&fixed, &fixed_indices)).unwrap();
    // The moving centre is 6 mm from either fixed component after this start.
    // Geometry alone cannot tell which identical component the operator meant.
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(7.0, 0.0, 0.0));

    let outcome = refine(moving, &fixed_index, start, &settings(), &CancelFlag::new());

    assert_eq!(outcome, Err(FitRejection::Ambiguous));
}

#[test]
fn best_fit_recovers_when_the_initial_gap_is_outside_the_search_radius() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 8.0));

    let report = refine(mesh, &index, start, &settings(), &CancelFlag::new()).unwrap();

    assert!(
        report.rigid.translation.length() < 0.05,
        "the bounded center hypothesis did not recover the nearby mesh: {:?}",
        report.rigid.translation
    );
}

#[test]
fn a_refine_that_cannot_prove_an_improvement_is_refused() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let limited = RefineSettings {
        max_iterations: 1,
        ..settings()
    };
    let start = Rigid::new(
        DQuat::from_axis_angle(DVec3::Z, 1.0),
        DVec3::new(1.5, -0.18, 0.12),
    );

    let outcome = refine(mesh, &index, start, &limited, &CancelFlag::new());

    let trustworthy = match &outcome {
        Ok(report) => report.is_trustworthy_refinement(),
        Err(_) => false,
    };
    assert!(
        !trustworthy,
        "a non-converged fit must not authorize a map: {outcome:?}"
    );
}

#[test]
fn a_single_accepted_step_is_the_pose_that_refine_returns() {
    let (positions, indices) = dome(24, 0.5);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let start = Rigid::new(DQuat::from_axis_angle(DVec3::Z, 0.08), DVec3::ZERO);
    let one_step = RefineSettings {
        max_iterations: 1,
        ..settings()
    };

    let report = refine(mesh, &index, start, &one_step, &CancelFlag::new()).unwrap();
    let rotation_change = (report.rigid.rotation * start.rotation.inverse())
        .to_scaled_axis()
        .length();

    assert!(
        rotation_change > 1e-5,
        "the accepted first step was discarded before returning the report: {:?}",
        report.rigid
    );
}

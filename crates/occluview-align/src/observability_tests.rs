//! Tests for the sensitivity measure, split out of `observability.rs` to hold
//! the workspace's file budget.
//!
//! Two kinds of assertion. The first pins the sensitivities of surfaces whose
//! sliding directions are known by inspection — a plane slides in its own plane
//! and turns in it, a cylinder slides and turns about its axis, a sphere turns
//! about anything. The second closes the loop: displace a fixture *along the
//! direction the measure names as blindest*, measure the deviation that
//! actually results, and require the reported bound to come back to the
//! displacement that was applied.
#![allow(clippy::expect_used)]

use glam::{DMat3, DQuat, DVec3};

use crate::observability::observability;
use crate::{
    deviation, deviation_stats, CancelFlag, DeviationSettings, Observability, Orientation, Rigid,
    Soup, SurfaceIndex,
};

fn settings() -> DeviationSettings {
    DeviationSettings {
        influence_radius_mm: 5.0,
        orientation: Orientation::Match,
    }
}

fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

fn measure(positions: &[f32], indices: &[u32], pose: Rigid) -> Observability {
    let mesh = soup(positions, indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    observability(mesh, &index, pose, &settings(), &CancelFlag::new()).unwrap()
}

#[test]
fn a_plane_is_blind_to_two_slides_and_one_turn_and_sees_the_other_three() {
    let (positions, indices) = plane(20.0, 40);
    let seen = measure(&positions, &indices, Rigid::IDENTITY);

    for value in &seen.sensitivity[0..3] {
        assert!(
            *value < 1e-6,
            "a plane must be blind in three modes: {value}"
        );
    }
    for value in &seen.sensitivity[3..6] {
        assert!(
            (*value - 1.0).abs() < 1e-6,
            "and see the other three in full: {value}"
        );
    }
    assert!(seen.has_blind_direction());
    assert_eq!(
        seen.hidden_displacement_mm(0.01),
        f64::INFINITY,
        "a free slide is unbounded, and saying so is the point"
    );
}

#[test]
fn a_cylinder_is_blind_to_the_axial_slide_and_the_axial_turn() {
    let (positions, indices) = cylinder(5.0, 24.0, 96, 40);
    let seen = measure(&positions, &indices, Rigid::IDENTITY);

    assert!(seen.sensitivity[0] < 1e-3, "{:?}", seen.sensitivity);
    // Not zero: a tessellated cylinder is a ninety-six-sided prism, and turning
    // a prism about its axis does shear its facets a little. What must hold is
    // the *gap* — the axial screw is an order of magnitude less visible than
    // anything else the surface can do.
    assert!(seen.sensitivity[1] < 0.05, "{:?}", seen.sensitivity);
    assert!(
        seen.sensitivity[2] > seen.sensitivity[1] * 10.0 && seen.sensitivity[2] > 0.5,
        "the other four modes are visible: {:?}",
        seen.sensitivity
    );
    // Whichever of the two blind modes the solver names first, it is a motion
    // along or about Z and nothing else.
    let sideways = seen.blind_translation.truncate().length();
    let tipping = seen.blind_rotation.truncate().length();
    assert!(
        sideways < 0.05 && tipping < 0.05,
        "the blind mode must be the axial screw, got translation {:?} rotation {:?}",
        seen.blind_translation,
        seen.blind_rotation
    );
}

#[test]
fn a_sphere_is_blind_to_every_turn_and_sees_every_slide() {
    let (positions, indices) = sphere(5.0, 96, 48, 0.0);
    let seen = measure(&positions, &indices, Rigid::IDENTITY);

    // A tessellated sphere is a faceted one, so its turns are not perfectly
    // free; the gap to the slides is what carries the claim.
    for value in &seen.sensitivity[0..3] {
        assert!(
            *value < 0.05,
            "a sphere turns freely: {:?}",
            seen.sensitivity
        );
    }
    assert!(
        seen.sensitivity[3] > seen.sensitivity[2] * 10.0,
        "the turns must be an order of magnitude less visible than the slides: {:?}",
        seen.sensitivity
    );
    for value in &seen.sensitivity[3..6] {
        assert!(
            *value > 0.4,
            "but it cannot slide anywhere: {:?}",
            seen.sensitivity
        );
    }
}

#[test]
fn texture_on_the_sphere_closes_every_blind_direction() {
    let (positions, indices) = sphere(5.0, 120, 60, 0.6);
    let seen = measure(&positions, &indices, Rigid::IDENTITY);

    assert!(
        seen.worst_sensitivity() > 0.1,
        "bumps give a rotation something to bite on: {:?}",
        seen.sensitivity
    );
    assert!(!seen.has_blind_direction());
    assert!(seen.best_sensitivity() <= 1.0 + 1e-9);
}

#[test]
fn the_sensitivities_are_ordered_and_bounded() {
    for (positions, indices) in [
        plane(20.0, 24),
        cylinder(5.0, 24.0, 64, 24),
        sphere(5.0, 64, 32, 0.4),
    ] {
        let seen = measure(&positions, &indices, Rigid::IDENTITY);
        for pair in seen.sensitivity.windows(2) {
            assert!(pair[0] <= pair[1] + 1e-12, "{:?}", seen.sensitivity);
        }
        assert!(seen.sensitivity[0] >= 0.0);
        assert!(seen.sensitivity[5] <= 1.0 + 1e-12);
    }
}

/// The assertion the under-reporting needed. Displace the fixture along the
/// direction the measure calls blindest, by a known amount, and require the
/// reported bound to come back to that amount.
#[test]
fn the_hidden_displacement_bound_recovers_a_known_offset_along_the_blind_mode() {
    /// Displacement applied, as an RMS over the surface, in millimetres.
    const APPLIED_MM: f64 = 0.10;
    /// The estimate is first order, so at a finite offset it may miss either
    /// way by a few percent — measured at 0.94 to 1.01 of the truth on real
    /// arch scans across 0.02 to 0.30 mm. What must hold is that it lands near
    /// the truth rather than near the roughly-half-of-it the raw statistic
    /// reports.
    const LOW: f64 = 0.9;
    const HIGH: f64 = 1.25;

    let (positions, indices) = sphere(5.0, 120, 60, 0.6);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let seen = observability(
        mesh,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    let pose = twist(
        seen.pivot,
        seen.blind_rotation * APPLIED_MM,
        seen.blind_translation * APPLIED_MM,
    );
    let truth = rms_displacement(&positions, pose);
    assert!(
        (truth - APPLIED_MM).abs() < APPLIED_MM * 0.1,
        "the fixture should move by {APPLIED_MM} mm RMS, it moved by {truth}"
    );

    let map = deviation(mesh, &index, pose, &settings(), &CancelFlag::new());
    let reported = deviation_stats(&map, 0.2)
        .summary
        .expect("sphere(5.0, 120, 60, 0.6) clears MIN_MEASURED")
        .rms;
    assert!(
        reported < truth * 0.9,
        "this fixture is supposed to under-report: reported {reported} against {truth}"
    );

    let bound = seen.hidden_displacement_mm(reported);
    assert!(
        bound >= truth * LOW,
        "the estimate understated a real displacement: {bound} against {truth}"
    );
    assert!(
        bound <= truth * HIGH,
        "the estimate is looser than stated: {bound} against {truth}"
    );
}

/// The same closing of the loop on a cylinder, where the blind mode is exact
/// and the bound is therefore infinite — which is the honest answer, not a
/// failure.
#[test]
fn a_free_slide_reports_an_unbounded_hidden_displacement() {
    let (positions, indices) = cylinder(5.0, 24.0, 96, 40);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let seen = observability(
        mesh,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .unwrap();

    let pose = Rigid::new(DQuat::IDENTITY, DVec3::Z * 0.30);
    let map = deviation(mesh, &index, pose, &settings(), &CancelFlag::new());
    let reported = deviation_stats(&map, 0.2)
        .summary
        .expect("cylinder(5.0, 24.0, 96, 40) clears MIN_MEASURED")
        .rms;

    assert!(
        reported < 0.06,
        "a 0.30 mm axial slide is nearly invisible here: {reported}"
    );
    assert!(
        seen.hidden_displacement_mm(reported) > 0.30,
        "and the bound must not claim otherwise"
    );
}

#[test]
fn nothing_in_reach_reports_no_sensitivity_rather_than_a_made_up_one() {
    let (positions, indices) = sphere(5.0, 48, 24, 0.4);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let far = Rigid::new(DQuat::IDENTITY, DVec3::Z * 500.0);
    assert!(
        observability(mesh, &index, far, &settings(), &CancelFlag::new()).is_none(),
        "an unmeasurable pair has no sensitivity to report"
    );
}

#[test]
fn a_cancelled_run_reports_nothing() {
    let (positions, indices) = sphere(5.0, 48, 24, 0.4);
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(observability(mesh, &index, Rigid::IDENTITY, &settings(), &cancel).is_none());
}

#[test]
fn a_surface_too_small_to_span_six_freedoms_reports_nothing() {
    // One triangle: not enough samples, and no six-dimensional span even if
    // there were.
    let positions = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let indices = vec![0u32, 1, 2];
    let mesh = soup(&positions, &indices);
    let index = SurfaceIndex::build(mesh).unwrap();
    assert!(observability(
        mesh,
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new()
    )
    .is_none());
}

#[test]
fn the_sensitivity_is_identical_whatever_thread_count_computed_it() {
    let (positions, indices) = sphere(5.0, 96, 48, 0.5);
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| measure(&positions, &indices, Rigid::IDENTITY))
    };
    assert_eq!(
        run(1),
        run(4),
        "the sensitivity must not drift with threads"
    );
    assert_eq!(run(1), run(9));
}

// ---- helpers --------------------------------------------------------------

/// The rigid motion whose first-order displacement field is
/// `ω × (p − pivot) + v`.
fn twist(pivot: DVec3, rotation: DVec3, translation: DVec3) -> Rigid {
    let angle = rotation.length();
    let turn = if angle > 0.0 {
        DQuat::from_axis_angle(rotation / angle, angle)
    } else {
        DQuat::IDENTITY
    };
    Rigid::new(turn, pivot + translation - DMat3::from_quat(turn) * pivot)
}

/// Root-mean-square true displacement of every vertex under `pose`.
fn rms_displacement(positions: &[f32], pose: Rigid) -> f64 {
    let mut squares = 0.0;
    let mut count = 0usize;
    for point in positions.as_chunks::<3>().0 {
        let local = DVec3::new(
            f64::from(point[0]),
            f64::from(point[1]),
            f64::from(point[2]),
        );
        squares += (pose.apply(local) - local).length_squared();
        count += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    let total = count.max(1) as f64;
    (squares / total).sqrt()
}

// ---- fixtures -------------------------------------------------------------

fn quad_indices(cols: usize, rows: usize, wrap: bool) -> Vec<u32> {
    let stride = u32::try_from(cols).unwrap();
    let last = if wrap { stride } else { stride - 1 };
    let mut indices = Vec::new();
    for row in 0..u32::try_from(rows - 1).unwrap() {
        for col in 0..last {
            let next = if wrap { (col + 1) % stride } else { col + 1 };
            let corner = row * stride + col;
            let neighbour = row * stride + next;
            indices.extend_from_slice(&[corner, neighbour, corner + stride]);
            indices.extend_from_slice(&[neighbour, neighbour + stride, corner + stride]);
        }
    }
    indices
}

fn plane(size: f32, steps: usize) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::new();
    for row in 0..steps {
        for col in 0..steps {
            #[allow(clippy::cast_precision_loss)]
            let x = col as f32 / (steps - 1) as f32 * size - size * 0.5;
            #[allow(clippy::cast_precision_loss)]
            let y = row as f32 / (steps - 1) as f32 * size - size * 0.5;
            positions.extend_from_slice(&[x, y, 0.0]);
        }
    }
    (positions, quad_indices(steps, steps, false))
}

fn cylinder(radius: f32, length: f32, around: usize, along: usize) -> (Vec<f32>, Vec<u32>) {
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
    (positions, quad_indices(around, along, true))
}

/// A sphere, optionally roughened by a radial ripple so its rotations stop
/// being free.
fn sphere(radius: f32, around: usize, down: usize, texture: f32) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::new();
    for ring in 0..down {
        #[allow(clippy::cast_precision_loss)]
        let polar = ring as f32 / (down - 1) as f32 * std::f32::consts::PI;
        for step in 0..around {
            #[allow(clippy::cast_precision_loss)]
            let azimuth = step as f32 / around as f32 * std::f32::consts::TAU;
            let bumped = radius + texture * (3.0 * azimuth).sin() * (4.0 * polar).sin();
            positions.extend_from_slice(&[
                bumped * polar.sin() * azimuth.cos(),
                bumped * polar.sin() * azimuth.sin(),
                bumped * polar.cos(),
            ]);
        }
    }
    (positions, quad_indices(around, down, true))
}

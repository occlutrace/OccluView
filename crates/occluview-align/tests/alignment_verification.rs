//! Public-API regressions for trustworthy scan alignment.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::expect_used,
    clippy::panic
)]

use glam::{DQuat, DVec3};
use occluview_align::{
    refine, CancelFlag, FitRejection, RefineSettings, Rigid, Soup, SurfaceIndex,
};

#[derive(Clone)]
struct Mesh {
    positions: Vec<f32>,
    indices: Vec<u32>,
}

impl Mesh {
    fn soup(&self) -> Soup<'_> {
        Soup {
            positions: &self.positions,
            indices: &self.indices,
            mask: None,
        }
    }

    fn transform(&mut self, rigid: Rigid) {
        for point in self.positions.as_chunks_mut::<3>().0 {
            let world = rigid.apply(DVec3::new(
                f64::from(point[0]),
                f64::from(point[1]),
                f64::from(point[2]),
            ));
            point[0] = world.x as f32;
            point[1] = world.y as f32;
            point[2] = world.z as f32;
        }
    }
}

#[derive(Clone, Copy)]
enum ArchShape {
    Upper,
    Lower,
}

/// A textured, asymmetric arch surface. `origin` selects a local crop from it.
fn arch(n: usize, step: f32, origin: DVec3, shape: ArchShape) -> Mesh {
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    for j in 0..=n {
        for i in 0..=n {
            let x = i as f32 * step;
            let y = j as f32 * step;
            let gx = x + origin.x as f32;
            let gy = y + origin.y as f32;
            let z = match shape {
                ArchShape::Upper => {
                    let dx = gx - 20.0;
                    let dy = gy - 20.0;
                    0.025 * dx * dx
                        + 0.032 * dy * dy
                        + 0.22 * (0.71 * gx).sin() * (0.53 * gy).cos()
                        + 0.13 * (0.31 * gx * gy).sin()
                        + 1.1 * (-((gx - 19.0).powi(2) + (gy - 21.0).powi(2)) / 4.0).exp()
                }
                ArchShape::Lower => {
                    let dx = gx - 18.0;
                    let dy = gy - 22.0;
                    0.018 * dx * dx - 0.012 * dx * dy
                        + 0.029 * dy * dy
                        + 0.24 * (0.43 * gx + 0.4).sin() * (0.37 * gy).cos()
                        + 0.17 * (0.27 * gx * gy + 0.8).sin()
                        + 0.9 * (-((gx - 10.0).powi(2) + (gy - 29.0).powi(2)) / 5.0).exp()
                }
            };
            positions.extend_from_slice(&[x, y, z]);
        }
    }
    Mesh {
        positions,
        indices: grid_indices(n),
    }
}

fn grid_indices(n: usize) -> Vec<u32> {
    let stride = u32::try_from(n + 1).expect("fixture stride fits in u32");
    let span = u32::try_from(n).expect("fixture span fits in u32");
    let mut indices = Vec::with_capacity(n * n * 6);
    for j in 0..span {
        for i in 0..span {
            let a = j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride, a + 1, a + stride + 1, a + stride]);
        }
    }
    indices
}

fn append_mesh(target: &mut Mesh, component: &Mesh, offset: DVec3) {
    let base = u32::try_from(target.positions.len() / 3).expect("fixture vertex count fits u32");
    target.positions.extend(
        component
            .positions
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
    target
        .indices
        .extend(component.indices.iter().map(|index| base + *index));
}

fn deterministic_noise(mesh: &mut Mesh, amplitude_mm: f64) {
    for (index, point) in mesh.positions.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let phase = index as f64;
        point[0] += (phase * 0.71).sin() as f32 * amplitude_mm as f32;
        point[1] += (phase * 0.39).cos() as f32 * amplitude_mm as f32;
        point[2] += (phase * 0.17).sin() as f32 * amplitude_mm as f32;
    }
}

/// Lift one local patch of `mesh` by `mm` along z, the change a rescan after
/// treatment carries where the operator has worked.
fn lift_patch(mesh: &mut Mesh, min: DVec3, max: DVec3, mm: f64) {
    for point in mesh.positions.as_chunks_mut::<3>().0 {
        let local = DVec3::new(
            f64::from(point[0]),
            f64::from(point[1]),
            f64::from(point[2]),
        );
        if local.x >= min.x && local.x <= max.x && local.y >= min.y && local.y <= max.y {
            point[2] += mm as f32;
        }
    }
}

fn settings() -> RefineSettings {
    RefineSettings::default()
}

fn refusal_is_untrusted(outcome: Result<occluview_align::IcpReport, FitRejection>) -> bool {
    match outcome {
        Err(_) => true,
        Ok(report) => !report.is_trustworthy_refinement_for(&settings()),
    }
}

#[test]
fn a_noisy_displaced_rescan_is_trustworthy() {
    let fixed = arch(48, 0.5, DVec3::ZERO, ArchShape::Upper);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed arch index");
    let acquisition = Rigid::new(
        DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.01),
        DVec3::new(0.24, -0.16, 0.12),
    );
    let mut moving = fixed.clone();
    moving.transform(acquisition);
    deterministic_noise(&mut moving, 0.006);

    let report = refine(
        moving.soup(),
        &fixed_index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .expect("a noisy rescan of the same arch has one supported seating");

    assert!(
        report.is_trustworthy_refinement_for(&settings()),
        "the known rescan must pass the public trust gate: {report:?}"
    );
    let remaining = report.rigid.compose(&acquisition);
    let max_error = moving
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| {
            let local = DVec3::new(
                f64::from(point[0]),
                f64::from(point[1]),
                f64::from(point[2]),
            );
            remaining.apply(local).distance(local)
        })
        .fold(0.0, f64::max);
    assert!(
        max_error < 0.05,
        "rescan remains {max_error:.4} mm from truth"
    );
}

/// An operated patch makes a rescan's trimmed residual tail heavy. The pose is
/// still decided by the surface that did not change, so the public trust gate
/// must authorize it: this is the pre-/post-treatment pair an operator seats to
/// compare a case, and refusing it would refuse the whole workflow.
#[test]
fn a_changed_region_does_not_refuse_an_otherwise_seated_rescan() {
    let fixed = arch(48, 0.5, DVec3::ZERO, ArchShape::Upper);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed arch index");
    let acquisition = Rigid::new(
        DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.01),
        DVec3::new(0.24, -0.16, 0.12),
    );
    let mut moving = fixed.clone();
    moving.transform(acquisition);
    deterministic_noise(&mut moving, 0.006);
    // A quarter of the arch lifted 0.2 mm: intentionally different surface,
    // three quarters unchanged, which is what the fit must decide on.
    lift_patch(
        &mut moving,
        DVec3::new(0.0, 0.0, f64::MIN),
        DVec3::new(12.0, 24.0, f64::MAX),
        0.2,
    );

    let report = refine(
        moving.soup(),
        &fixed_index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .expect("a rescan with an operated patch has one supported seating");

    assert!(
        report.is_trustworthy_refinement_for(&settings()),
        "the unchanged region of a changed pair must authorize the pose: {report:?}"
    );
    // The untrimmed median limit the gate and the verification rule share.
    assert!(
        report.verified_median_mm < 0.05,
        "the unchanged region seats: verified median {:.4} mm",
        report.verified_median_mm
    );
    assert!(
        report.verified_coverage > 0.5,
        "most of the surface still has a counterpart: {:.3}",
        report.verified_coverage
    );
}

#[test]
fn a_partial_jaw_seats_against_its_full_arch() {
    let fixed = arch(80, 0.5, DVec3::ZERO, ArchShape::Upper);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed arch index");
    let crop_origin = DVec3::new(27.0, 27.0, 0.0);
    let moving = arch(24, 0.5, crop_origin, ArchShape::Upper);

    let report = refine(
        moving.soup(),
        &fixed_index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    )
    .expect("a partial jaw has a matching window in the full arch");

    assert!(
        report.is_trustworthy_refinement_for(&settings()),
        "the known partial-arch fit must pass the public trust gate: {report:?}"
    );
    assert!(
        (report.rigid.translation - crop_origin).length() < 0.1,
        "the crop returns to its source window: {:?}",
        report.rigid
    );
}

#[test]
fn repeated_partial_arches_are_refused_as_wrong_basins() {
    let component = arch(24, 0.5, DVec3::ZERO, ArchShape::Upper);
    let mut fixed = component.clone();
    append_mesh(&mut fixed, &component, DVec3::new(14.0, 0.0, 0.0));
    let moving = component;
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("two-component index");
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(7.0, 0.0, 0.0));

    let outcome = refine(
        moving.soup(),
        &fixed_index,
        start,
        &settings(),
        &CancelFlag::new(),
    );

    assert!(
        matches!(outcome, Err(FitRejection::Ambiguous)),
        "two equally good source windows must refuse as ambiguous: {outcome:?}"
    );
}

#[test]
fn a_partial_overlap_with_a_different_arch_is_not_trusted() {
    let fixed = arch(80, 0.5, DVec3::ZERO, ArchShape::Upper);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed arch index");
    let moving = arch(24, 0.5, DVec3::new(27.0, 27.0, 0.0), ArchShape::Lower);

    let outcome = refine(
        moving.soup(),
        &fixed_index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    );

    let details = format!("{outcome:?}");
    assert!(
        refusal_is_untrusted(outcome),
        "a small unrelated overlap must not authorize a heatmap: {details}"
    );
}

#[test]
fn two_different_arches_are_not_trusted_as_a_pair() {
    let fixed = arch(48, 0.5, DVec3::ZERO, ArchShape::Upper);
    let moving = arch(48, 0.5, DVec3::ZERO, ArchShape::Lower);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed arch index");

    let outcome = refine(
        moving.soup(),
        &fixed_index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    );

    assert!(
        refusal_is_untrusted(outcome),
        "two different synthetic jaws have no single correct seating"
    );
}

#[test]
fn a_symmetric_cylindrical_patch_is_not_trusted() {
    let fixed = cylinder(48, 48, 0.0, 24.0);
    let fixed_index = SurfaceIndex::build(fixed.soup()).expect("fixed cylinder index");
    let moving = cylinder(48, 24, 3.0, 18.0);
    let start = Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 2.0));

    let outcome = refine(
        moving.soup(),
        &fixed_index,
        start,
        &settings(),
        &CancelFlag::new(),
    );

    assert!(
        refusal_is_untrusted(outcome),
        "an axial slide on a symmetric patch has no unique pose"
    );
}

fn cylinder(around: usize, along: usize, start_z: f64, length: f64) -> Mesh {
    let mut positions = Vec::with_capacity((along + 1) * around * 3);
    for j in 0..=along {
        let z = start_z + length * j as f64 / along as f64;
        for i in 0..around {
            let angle = std::f64::consts::TAU * i as f64 / around as f64;
            positions.extend_from_slice(&[
                (4.0 * angle.cos()) as f32,
                (4.0 * angle.sin()) as f32,
                z as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity(along * around * 6);
    let ring = u32::try_from(around).expect("fixture ring fits in u32");
    for j in 0..u32::try_from(along).expect("fixture rows fit in u32") {
        for i in 0..ring {
            let next = (i + 1) % ring;
            let a = j * ring + i;
            let b = j * ring + next;
            let c = a + ring;
            let d = b + ring;
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    Mesh { positions, indices }
}

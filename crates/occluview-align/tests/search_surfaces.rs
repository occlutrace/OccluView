//! Synthetic surface/control gates; pose classification belongs to independent verification.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod support;
use glam::{DAffine3, DMat3, DQuat, DVec3};
use occluview_align::*;
use occluview_geometry::surface::{GeometryControl, GeometryLimits, GeometryStop, QueryOutcome};
use std::time::{Duration, Instant};
use support::{arch, operators};

fn mesh(soup: Soup<'_>, world_from_local: DAffine3) -> MeshInput<'_> {
    MeshInput {
        soup,
        world_from_local,
        revision: 17,
    }
}
fn control() -> GeometryControl {
    GeometryControl::new(
        CancelFlag::new(),
        Duration::from_secs(10),
        GeometryLimits::default(),
    )
}
fn prepare(mesh: MeshInput<'_>) -> SurfacePreparation {
    prepare_alignment_surface(
        mesh,
        SurfaceSide::Moving,
        RegionPolicy::AllEligible,
        &control(),
    )
    .unwrap()
}
fn gum() -> support::SyntheticMesh {
    arch::dental_arch(&arch::ArchSpec {
        teeth: 0,
        palate_mm: 0.,
        grid: [100, 24],
        ..Default::default()
    })
}

/// ID07 surface gate: role exchange preserves area denominators and reversible frames.
#[test]
fn crop_roles_can_be_swapped_surface_accounting() {
    let full = gum();
    let large = prepare(mesh(full.soup(), DAffine3::IDENTITY))
        .surface
        .unwrap();
    for fraction in [0.3, 0.5, 0.7] {
        let crop = operators::crop_by_area(&full, fraction, operators::CropWindow::OneSided);
        let small = prepare(mesh(crop.soup(), DAffine3::IDENTITY))
            .surface
            .unwrap();
        assert!((small.eligible_area_mm2 / large.eligible_area_mm2 - fraction).abs() <= 0.005);
        let world = Rigid::new(DQuat::from_rotation_y(0.1), DVec3::new(3., -2., 1.));
        let internal = small.frame.correction_to_query(large.frame, world).unwrap();
        let inverse = large
            .frame
            .correction_to_query(small.frame, world.inverse())
            .unwrap();
        let composed = internal.compose(&inverse);
        for sample in &small.samples[0].samples {
            assert!(composed.apply(sample.point).distance(sample.point) <= 1e-10);
        }
        let restored = small
            .frame
            .correction_to_world(large.frame, internal)
            .unwrap();
        assert!(restored.translation.distance(world.translation) <= 1e-10);
    }
}

/// ID11 surface gate: 1:20 and 20:1 density do not reweight physical support.
#[test]
fn density_ratio_does_not_reweight_surface_samples() {
    let coarse = arch::dental_arch(&arch::ArchSpec {
        grid: [40, 12],
        remesh_offset: 0.37,
        teeth: 0,
        palate_mm: 0.,
        ..Default::default()
    });
    let dense = arch::dental_arch(&arch::ArchSpec {
        grid: [200, 48],
        remesh_offset: 0.13,
        teeth: 0,
        palate_mm: 0.,
        ..Default::default()
    });
    assert_eq!(dense.triangles.len(), coarse.triangles.len() * 20);
    let a = prepare(mesh(coarse.soup(), DAffine3::IDENTITY))
        .surface
        .unwrap();
    let b = prepare(mesh(dense.soup(), DAffine3::IDENTITY))
        .surface
        .unwrap();
    let ca = control();
    let cb = control();
    let sa = area_samples(&a.original_index, 16_384, AREA_SAMPLE_SEED, true, &ca).unwrap();
    let sb = area_samples(&b.original_index, 16_384, AREA_SAMPLE_SEED, true, &cb).unwrap();
    for (left, right) in [(&sa, &sb), (&sb, &sa)] {
        for cut in [-15., -5., 5., 15.] {
            let fraction = |batch: &SampleBatch| {
                batch
                    .samples
                    .iter()
                    .filter(|s| s.point.x < cut)
                    .map(|s| s.area_weight_mm2)
                    .sum::<f64>()
                    / batch.samples.iter().map(|s| s.area_weight_mm2).sum::<f64>()
            };
            assert!((fraction(left) - fraction(right)).abs() <= 0.03);
        }
    }
    assert!((a.eligible_area_mm2 / b.eligible_area_mm2 - 1.).abs() <= 0.03);
    for surface in [&a, &b] {
        assert!(surface.samples[0].samples.len() <= 1_024);
        assert!(surface.samples[1].samples.len() <= 4_096);
        assert!(surface.samples[2].samples.len() <= 16_384);
        assert!(surface.samples[3].samples.len() <= 8_192);
    }
}

/// ID32 surface gate: malformed/empty/masked/degenerate geometry has no fake residual.
#[test]
fn finite_empty_or_masked_returns_weak_surface_evidence() {
    let positions = [0., 0., 0., 1., 0., 0., 0., 1., 0., 7.];
    for soup in [
        Soup {
            positions: &[],
            indices: &[],
            mask: None,
        },
        Soup {
            positions: &[0.; 9],
            indices: &[0, 1, 2],
            mask: None,
        },
        Soup {
            positions: &positions,
            indices: &[0, u32::MAX, 2, 1],
            mask: None,
        },
        Soup {
            positions: &positions,
            indices: &[0, 1, 2],
            mask: Some(&[1, 1, 1]),
        },
    ] {
        let p = prepare(mesh(soup, DAffine3::IDENTITY));
        assert!(p.surface.is_none());
        assert_eq!(p.completion, Completion::NoUsableSurface);
        assert_eq!(p.input_check, InputCheck::Complete);
        let i = AlignmentInput {
            moving: mesh(soup, DAffine3::IDENTITY),
            fixed: mesh(soup, DAffine3::IDENTITY),
            landmarks: &[],
            seeds: &[],
        };
        let r =
            search_alignment(&i, &SearchSettings::default(), &SearchControl::default()).unwrap();
        assert_eq!(r.candidates[0].confidence, Confidence::Weak);
        assert!(matches!(
            r.candidates[0].evidence.euclidean_mm,
            Metric::Missing(_)
        ));
        assert!(matches!(
            r.candidates[0].evidence.plane_mm,
            Metric::Missing(_)
        ));
    }
    let nan = Soup {
        positions: &[0., 0., 0., f32::NAN],
        indices: &[],
        mask: Some(&[1]),
    };
    assert_eq!(
        prepare_alignment_surface(
            mesh(nan, DAffine3::IDENTITY),
            SurfaceSide::Fixed,
            RegionPolicy::AllEligible,
            &control()
        )
        .unwrap_err(),
        AlignmentInputError::NonFinite {
            field: InputField::FixedPositions,
            index: 3
        }
    );
}

/// ID34 preprocessing/sampling gate: cancellation latency and reservation release.
#[test]
fn cancel_every_stage_of_surface_preparation() {
    let base = gum();
    let baseline = control();
    let p = prepare_alignment_surface(
        mesh(base.soup(), DAffine3::IDENTITY),
        SurfaceSide::Moving,
        RegionPolicy::AllEligible,
        &baseline,
    )
    .unwrap();
    drop(p);
    let operations = baseline.counters().operations;
    for fraction in [0.01, 0.20, 0.75, 0.92, 0.99] {
        let flag = CancelFlag::new();
        let c = GeometryControl::new(
            flag.clone(),
            Duration::from_secs(10),
            GeometryLimits::default(),
        );
        let threshold = (operations as f64 * fraction) as u64;
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let echo = c.clone();
            scope.spawn(move || {
                let start = Instant::now();
                while echo.counters().operations < threshold
                    && start.elapsed() < Duration::from_secs(2)
                {
                    std::thread::yield_now();
                }
                let at = Instant::now();
                flag.cancel();
                sent.send(at).unwrap();
            });
            let result = prepare_alignment_surface(
                mesh(base.soup(), DAffine3::IDENTITY),
                SurfaceSide::Moving,
                RegionPolicy::AllEligible,
                &c,
            )
            .unwrap();
            let terminal = Instant::now();
            let at = received.recv().unwrap();
            assert_eq!(result.completion, Completion::Cancelled);
            assert!(terminal.saturating_duration_since(at) < Duration::from_millis(100));
            drop(result);
            assert!(at.elapsed() < Duration::from_millis(500));
        });
        assert_eq!(c.counters().memory_bytes, 0);
    }
}

/// ID35: two million triangles remain bounded and resource-limited, never certified.
#[test]
fn deadline_and_work_caps_are_honest_for_large_surfaces() {
    let positions = [0., 0., 0., 10., 0., 0., 0., 10., 0.];
    let triangles: Vec<u32> = (0..2_000_000).flat_map(|_| [0, 1, 2]).collect();
    let soup = Soup {
        positions: &positions,
        indices: &triangles,
        mask: None,
    };
    let c = control();
    let started = Instant::now();
    let result = prepare_alignment_surface(
        mesh(soup, DAffine3::IDENTITY),
        SurfaceSide::Moving,
        RegionPolicy::AllEligible,
        &c,
    )
    .unwrap();
    assert!(started.elapsed() <= Duration::from_millis(10_100));
    assert_eq!(result.completion, Completion::ResourceLimit);
    assert_eq!(result.input_check, InputCheck::Complete);
    assert!(result.surface.is_none());
    assert_eq!(result.eligible_area_mm2, Metric::Measured(100_000_000.));
    assert!(c.counters().peak_memory_bytes <= 256 * 1024 * 1024);
    assert_eq!(c.counters().memory_bytes, 0);
    let tiny = GeometryControl::new(
        CancelFlag::new(),
        Duration::from_secs(10),
        GeometryLimits {
            operations: 256,
            ..Default::default()
        },
    );
    let result = prepare_alignment_surface(
        mesh(soup, DAffine3::IDENTITY),
        SurfaceSide::Moving,
        RegionPolicy::AllEligible,
        &tiny,
    )
    .unwrap();
    assert_eq!(result.completion, Completion::WorkLimit);
    assert!(tiny.counters().operations <= 256);
    assert!(matches!(result.eligible_area_mm2, Metric::Missing(_)));
    let expired =
        GeometryControl::new(CancelFlag::new(), Duration::ZERO, GeometryLimits::default());
    let p = prepare_alignment_surface(
        mesh(soup, DAffine3::IDENTITY),
        SurfaceSide::Moving,
        RegionPolicy::AllEligible,
        &expired,
    )
    .unwrap();
    assert_eq!(p.completion, Completion::Deadline);
    assert!(matches!(
        p.input_check,
        InputCheck::Partial { checked: 0, .. }
    ));
}

/// ID37: authored million-mm translations preserve f64 geometry, scale and winding.
#[test]
fn authored_frame_and_normal_sign_invariance_surface_view() {
    let base = gum();
    let inverse = operators::invert_winding(&base, false);
    let affine = DAffine3::from_scale_rotation_translation(
        DVec3::new(1.02, 1.1, 0.9),
        DQuat::from_rotation_z(0.7),
        DVec3::new(2., -1., 3.),
    );
    let a = prepare(mesh(base.soup(), affine)).surface.unwrap();
    let offset = DVec3::splat(1_000_000.);
    let b = prepare(mesh(
        base.soup(),
        DAffine3 {
            translation: affine.translation + offset,
            ..affine
        },
    ))
    .surface
    .unwrap();
    let flipped = prepare(mesh(inverse.soup(), affine)).surface.unwrap();
    assert!((a.eligible_area_mm2 / b.eligible_area_mm2 - 1.).abs() <= 1e-8);
    let ca = control();
    let cb = control();
    let ci = control();
    let sa = area_samples(&a.original_index, 8192, AREA_SAMPLE_SEED, true, &ca).unwrap();
    let sb = area_samples(&b.original_index, 8192, AREA_SAMPLE_SEED, true, &cb).unwrap();
    let si = area_samples(&flipped.original_index, 8192, AREA_SAMPLE_SEED, true, &ci).unwrap();
    for (x, y) in sa.samples.iter().zip(&sb.samples) {
        assert!(x.point.distance(y.point) <= 1e-4);
    }
    for x in &sa.samples {
        let QueryOutcome::Complete(Some(hit)) =
            flipped.original_index.nearest_controlled(x.point, 0.1, &ci)
        else {
            panic!("same surface under winding inversion");
        };
        assert!(hit.point.distance(x.point) <= 0.02);
        assert!(x.normal.unwrap().dot(hit.normal) <= -0.999);
    }
    let mut reordered = base.clone();
    reordered.triangles.as_chunks_mut::<3>().0.reverse();
    let reordered = prepare(mesh(reordered.soup(), affine)).surface.unwrap();
    let cr = control();
    for sample in &sa.samples {
        let world_point = sample.point + a.frame.center_world;
        let query = world_point - reordered.frame.center_world;
        let QueryOutcome::Complete(Some(hit)) =
            reordered.original_index.nearest_controlled(query, 0.1, &cr)
        else {
            panic!("source record order preserves geometry");
        };
        assert!((hit.point + reordered.frame.center_world).distance(world_point) <= 0.05);
    }
    assert_eq!(si.samples.len(), sa.samples.len());
    assert!((DMat3::from_quat(DQuat::from_rotation_z(0.7)).determinant() - 1.).abs() <= 1e-10);
    let plane = Soup {
        positions: &[0., 0., 0., 1., 0., 0., 0., 1., 0.],
        indices: &[0, 1, 2],
        mask: None,
    };
    let mirrored_plane = prepare(mesh(plane, DAffine3::from_scale(DVec3::new(-2., 3., 1.))))
        .surface
        .unwrap();
    for (_, _, normal) in mirrored_plane.original_index.triangles() {
        assert!(normal.dot(DVec3::Z) >= 1. - 1e-12);
    }
    let singular = prepare(mesh(
        base.soup(),
        DAffine3::from_scale(DVec3::new(1., 1., 0.)),
    ))
    .surface
    .unwrap();
    assert!(!singular.quality.orientation_coherent);
    assert!(singular
        .samples
        .iter()
        .flat_map(|b| &b.samples)
        .all(|s| s.normal.is_none()));
}

/// ID38 sampler: analytic area cells >=1% have absolute occupancy error <=.03.
#[test]
fn grid_and_area_sampler_are_measured_area_part() {
    let positions = [0., 0., 0., 100., 0., 0., 100., 100., 0., 0., 100., 0.];
    let c = control();
    let index = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &[0, 1, 2, 0, 2, 3],
        mask: None,
    })
    .unwrap();
    let first = area_samples(&index, 16_384, AREA_SAMPLE_SEED, true, &c).unwrap();
    let second = area_samples(&index, 16_384, AREA_SAMPLE_SEED, true, &c).unwrap();
    assert_eq!(first.samples, second.samples);
    let mut cells = [0usize; 100];
    for sample in &first.samples {
        let x = (sample.point.x / 10.).floor() as usize;
        let y = (sample.point.y / 10.).floor() as usize;
        cells[y * 10 + x] += 1;
        assert!((sample.barycentric.iter().sum::<f64>() - 1.).abs() <= 1e-12);
        assert!(sample.barycentric.iter().all(|&x| x > 0.));
    }
    for count in cells {
        assert!((count as f64 / 16_384. - 0.01).abs() <= 0.03);
    }
    assert!((first.samples.iter().map(|s| s.area_weight_mm2).sum::<f64>() - 10_000.).abs() <= 1e-8);
    let split = split_samples(&first.samples, 8192, &c).unwrap();
    let cells_of = |batch: &SampleBatch| {
        batch
            .samples
            .iter()
            .map(|s| {
                (
                    s.point.x.floor() as i64,
                    s.point.y.floor() as i64,
                    s.point.z.floor() as i64,
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert!(cells_of(&split.training).is_disjoint(&cells_of(&split.holdout)));
    let strata = split
        .holdout
        .samples
        .iter()
        .map(|s| spatial_stratum(s.point).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(strata.len(), 8);
    assert!(matches!(
        area_samples(&index, usize::MAX, 1, true, &c),
        Err(GeometryStop::ResourceLimit)
    ));
}

/// Declared finite negative controls never gain confidence from preprocessing alone.
#[test]
fn unrelated_plane_cylinder_sphere_surface_evidence_is_not_certified() {
    let shapes = [arch::plane(), arch::cylinder(), arch::sphere()];
    for seed in 0..100usize {
        let moving = &shapes[seed % 3];
        let fixed = &shapes[(seed + 1 + (seed / 3) % 2) % 3];
        let i = AlignmentInput {
            moving: mesh(
                moving.soup(),
                DAffine3::from_rotation_translation(
                    DQuat::from_rotation_z(seed as f64 * 0.1),
                    DVec3::new(seed as f64 * 0.2, -3., 1.),
                ),
            ),
            fixed: mesh(fixed.soup(), DAffine3::IDENTITY),
            landmarks: &[],
            seeds: &[],
        };
        let r = search_alignment(
            &i,
            &SearchSettings {
                profile: SearchProfile::Local,
                ..Default::default()
            },
            &SearchControl::default(),
        )
        .unwrap();
        assert!(!r.candidates.is_empty());
        for candidate in r.candidates {
            assert!(candidate.pose.is_finite());
            assert_ne!(candidate.confidence, Confidence::Verified);
        }
    }
}

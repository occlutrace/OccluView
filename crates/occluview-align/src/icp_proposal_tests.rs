//! Synthetic coarse proposal gates. Truth is used only after pool generation.
use super::*;
use crate::proposal_test_support::{arch, metrics, operators};
use crate::{AlignmentInput, Completion, NormalPolicy, SearchControl, SearchSettings, SurfaceSide};
use glam::DAffine3;
use std::time::{Duration, Instant};

fn input<'a>(
    moving: &'a crate::proposal_test_support::SyntheticMesh,
    fixed: &'a crate::proposal_test_support::SyntheticMesh,
) -> AlignmentInput<'a> {
    crate::proposal_test_support::alignment_input(moving.soup(), fixed.soup())
}
fn shortlist(
    request: &AlignmentInput<'_>,
    settings: &SearchSettings,
    options: ProposalOptions,
) -> (
    ProposalSearch,
    Vec<Rigid>,
    occluview_geometry::surface::GeometryCounters,
) {
    let clock = Instant::now();
    let search_control = SearchControl::default();
    let control = search_control.geometry_control(settings);
    let moving = crate::prepare_alignment_surface(
        request.moving,
        SurfaceSide::Moving,
        settings.reference_regions,
        &control,
    )
    .unwrap();
    let fixed = crate::prepare_alignment_surface(
        request.fixed,
        SurfaceSide::Fixed,
        settings.reference_regions,
        &control,
    )
    .unwrap();
    assert_eq!(moving.completion, Completion::Complete);
    assert_eq!(fixed.completion, Completion::Complete);
    let moving = moving.surface.unwrap();
    let fixed = fixed.surface.unwrap();
    let result = generate_hypotheses(&moving, &fixed, request, settings, &control, options);
    assert!(clock.elapsed() <= Duration::from_millis(10_100));
    assert!(result.pool.len() <= 32);
    let poses = result
        .pool
        .iter()
        .map(|p| {
            moving
                .frame
                .correction_to_world(fixed.frame, p.pose)
                .unwrap()
        })
        .collect::<Vec<_>>();
    for pose in &poses {
        assert!(pose.is_finite());
        assert!((pose.rotation.length() - 1.).abs() <= 1e-12);
        assert!((DMat3::from_quat(pose.rotation).determinant() - 1.).abs() <= 1e-10);
    }
    (result, poses, control.counters())
}
fn area_center(mesh: &crate::proposal_test_support::SyntheticMesh) -> DVec3 {
    mesh.triangles
        .as_chunks::<3>()
        .0
        .iter()
        .fold(DVec3::ZERO, |sum, t| {
            let p = t.map(|i| mesh.point(i as usize));
            let area = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
            sum + (p[0] + p[1] + p[2]) * (area / 3.)
        })
        / mesh.area()
}
fn coarse_recall(
    poses: &[Rigid],
    truth: Rigid,
    moving: &crate::proposal_test_support::SyntheticMesh,
) -> bool {
    let center = area_center(moving);
    poses.iter().any(|&pose| {
        let error = metrics::pose_error(pose, truth);
        error[0] <= 4. && pose.apply(center).distance(truth.apply(center)) <= 1.
    })
}
fn frozen_arch() -> crate::proposal_test_support::SyntheticMesh {
    let mesh = arch::dental_arch(&arch::ArchSpec::default());
    assert_identifiable(&mesh);
    mesh
}
fn assert_identifiable(mesh: &crate::proposal_test_support::SyntheticMesh) {
    let (information, cells) = metrics::reference_information(mesh);
    assert!(
        information[0] >= 0.004 && cells >= 200 && mesh.area() >= 100.,
        "independent identifiable preconditions {information:?}, cells={cells}"
    );
}
fn truth(index: u32) -> Rigid {
    let axis = DVec3::new(
        (f64::from(index) * 1.17 + 0.3).sin(),
        (f64::from(index) * 0.71 + 0.8).cos(),
        0.31 + f64::from(index) / 31.,
    )
    .normalize();
    let angle = if index == 19 {
        std::f64::consts::PI
    } else {
        std::f64::consts::TAU * (f64::from(index) + 0.5) / 20.
    };
    Rigid::new(
        DQuat::from_axis_angle(axis, angle),
        DVec3::new(
            32. * (f64::from(index) + 0.2).sin(),
            27. * (f64::from(index) + 0.3).cos(),
            19.,
        ),
    )
}
/// ID03: declared frozen 20-pose corpus, full independently remeshed arch.
#[test]
fn arbitrary_rotations_find_common_arch_coarse() {
    let fixed = frozen_arch();
    let remesh = operators::resample_density(&fixed, 1.);
    let mut successful = 0;
    let mut measurements = Vec::new();
    for index in 0..20 {
        let truth = truth(index);
        let moving = operators::rigid_offset(&remesh, truth);
        let (result, poses, counters) = shortlist(
            &input(&moving, &fixed),
            &SearchSettings::default(),
            ProposalOptions::default(),
        );
        let hit = result.stop.is_none()
            && result.family_stops.is_empty()
            && coarse_recall(&poses, truth, &moving);
        successful += usize::from(hit);
        let best = poses
            .iter()
            .map(|&p| metrics::pose_error(p, truth))
            .min_by(|a, b| (a[0] + a[1]).total_cmp(&(b[0] + b[1])));
        measurements.push((
            index,
            result.stop,
            result.family_stops.clone(),
            best,
            counters,
        ));
    }
    record_measurements(
        "gate03",
        &format!("successful={successful}/20\n{measurements:#?}"),
    );
    assert!(
        successful >= 19,
        "coarse recall {successful}/20: {measurements:?}"
    );
}
/// ID04: exact-area crop, no descriptor support used as an acceptance crutch.
#[test]
fn thirty_percent_moving_crop_coarse() {
    let fixed = frozen_arch();
    let crop = operators::crop_by_area(&fixed, 0.3, operators::CropWindow::Centered);
    assert!((crop.area() / fixed.area() - 0.3).abs() <= 0.005);
    assert_identifiable(&crop);
    for truth in [Rigid::IDENTITY, truth(7)] {
        let moving = operators::rigid_offset(&crop, truth);
        let (result, poses, _counters) = shortlist(
            &input(&moving, &fixed),
            &SearchSettings::default(),
            ProposalOptions {
                features: false,
                bases: true,
                ..ProposalOptions::default()
            },
        );
        record_coarse("gate04", &result, &poses, truth, &moving);
        assert!(
            result.stop.is_none() && coarse_recall(&poses, truth, &moving),
            "crop: stop={:?}, errors={:?}",
            result.stop,
            poses
                .iter()
                .map(|&p| metrics::pose_error(p, truth))
                .collect::<Vec<_>>()
        );
    }
}
/// ID08: one-sided crop; input displacement cannot gate centroid proposals.
#[test]
fn offset_crop_centroids_do_not_block_coarse() {
    let fixed = frozen_arch();
    let crop = operators::crop_by_area(&fixed, 0.3, operators::CropWindow::OneSided);
    assert_identifiable(&crop);
    assert!(area_center(&crop).distance(area_center(&fixed)) >= 15.);
    let truth = truth(13);
    let moving = operators::rigid_offset(&crop, truth);
    let (result, poses, _counters) = shortlist(
        &input(&moving, &fixed),
        &SearchSettings::default(),
        ProposalOptions {
            features: false,
            bases: true,
            ..ProposalOptions::default()
        },
    );
    record_coarse("gate08", &result, &poses, truth, &moving);
    assert!(
        result.stop.is_none() && coarse_recall(&poses, truth, &moving),
        "one-sided stop={:?}",
        result.stop
    );
}
/// ID15: rotation search and unsigned geometry ranking ignore winding policy.
#[test]
fn inverted_winding_keeps_search_coarse() {
    let fixed = frozen_arch();
    let truth = truth(11);
    let moving = operators::invert_winding(&operators::rigid_offset(&fixed, truth), false);
    let mut checkpoints = Vec::new();
    for normal_policy in [
        NormalPolicy::Opposed,
        NormalPolicy::Unsigned,
        NormalPolicy::Match,
    ] {
        let (result, poses, counters) = shortlist(
            &input(&moving, &fixed),
            &SearchSettings {
                normal_policy,
                ..SearchSettings::default()
            },
            ProposalOptions {
                features: false,
                bases: true,
                ..ProposalOptions::default()
            },
        );
        assert!(
            result.stop.is_none() && coarse_recall(&poses, truth, &moving),
            "stop={:?}, examined={}, graph={}, counters={counters:?}, errors={:?}",
            result.stop,
            result.examined,
            result.graph_visits,
            poses
                .iter()
                .map(|&p| metrics::pose_error(p, truth))
                .collect::<Vec<_>>()
        );
        // Policy reservations may change the retained rivals; the geometric
        // optimum itself remains invariant and truth stays in every shortlist.
        checkpoints.push(result.best.unwrap().pose);
    }
    for pose in &checkpoints[1..] {
        assert!(pose.translation.distance(checkpoints[0].translation) <= 0.05);
        assert!(
            2. * pose
                .rotation
                .dot(checkpoints[0].rotation)
                .abs()
                .clamp(0., 1.)
                .acos()
                <= 0.05f64.to_radians()
        );
    }
}
/// ID18: a tenfold unrelated exterior must not enter the smaller denominator.
#[test]
fn outer_extent_tenfold_coarse() {
    let gum_spec = arch::ArchSpec {
        teeth: 0,
        ..arch::ArchSpec::default()
    };
    let gum = arch::dental_arch(&gum_spec);
    let shell = arch::prosthesis_shell(&gum, 2., &arch::ArchSpec::default());
    let fixed = operators::outer_extent(&shell, gum.area());
    let truth = Rigid::new(
        DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 57.3f64.to_radians()),
        DVec3::new(7., -5., 4.),
    );
    let moving = operators::rigid_offset(&gum, truth);
    let (result, poses, _counters) = shortlist(
        &input(&moving, &fixed),
        &SearchSettings {
            normal_policy: NormalPolicy::Unsigned,
            ..SearchSettings::default()
        },
        ProposalOptions {
            features: false,
            bases: true,
            ..ProposalOptions::default()
        },
    );
    record_coarse("gate18", &result, &poses, truth, &moving);
    assert!(
        result.stop.is_none() && coarse_recall(&poses, truth, &moving),
        "extent stop={:?}",
        result.stop
    );
}
/// ID20: both fitting surfaces must survive as distinct geometric alternatives.
#[test]
fn twin_shell_surfaces_are_ambiguous_coarse() {
    let reference = frozen_arch();
    let fixed = operators::twin_surfaces(&reference);
    let mut successful = 0;
    let mut measurements = Vec::new();
    for index in 0..20 {
        let truth = truth(index);
        let moving = operators::rigid_offset(&reference, truth);
        let (result, poses, counters) = shortlist(
            &input(&moving, &fixed),
            &SearchSettings {
                normal_policy: NormalPolicy::Unsigned,
                ..SearchSettings::default()
            },
            ProposalOptions {
                features: false,
                bases: true,
                ..ProposalOptions::default()
            },
        );
        let second = Rigid {
            translation: truth.translation + DVec3::Z * 2.,
            ..truth
        };
        let hit = result.stop.is_none()
            && coarse_recall(&poses, truth, &moving)
            && coarse_recall(&poses, second, &moving);
        successful += usize::from(hit);
        measurements.push((index, result.stop, hit, counters));
    }
    record_measurements(
        "gate20",
        &format!("successful={successful}/20\n{measurements:#?}"),
    );
    assert_eq!(
        successful, 20,
        "twin coarse recall {successful}/20: {measurements:?}"
    );
}
/// ID21: fewer than 10,000 triangles and 2,000 feature voxels are not refusals.
#[test]
fn few_feature_points_still_propose_coarse() {
    let fixed = arch::dental_arch(&arch::ArchSpec {
        grid: [48, 12],
        ..arch::ArchSpec::default()
    });
    assert!(fixed.triangles.len() / 3 < 10_000);
    let truth = truth(3);
    let moving = operators::rigid_offset(&fixed, truth);
    let (result, poses, counters) = shortlist(
        &input(&moving, &fixed),
        &SearchSettings::default(),
        ProposalOptions::default(),
    );
    assert!(
        result.stop.is_none() && coarse_recall(&poses, truth, &moving),
        "stop={:?}, examined={}, graph={}, counters={counters:?}, errors={:?}",
        result.stop,
        result.examined,
        result.graph_visits,
        poses
            .iter()
            .map(|&p| metrics::pose_error(p, truth))
            .collect::<Vec<_>>()
    );
}
/// ID28: repeated geometry retains two placements separated by tooth pitch.
#[test]
fn repeated_teeth_keep_rivals_coarse() {
    let spec = arch::ArchSpec {
        grid: [32, 8],
        ..arch::ArchSpec::default()
    };
    let patch = arch::dental_arch(&spec);
    let fixed = operators::repeat_patch(&patch, 2, 6.);
    let (result, poses, counters) = shortlist(
        &input(&patch, &fixed),
        &SearchSettings {
            normal_policy: NormalPolicy::Unsigned,
            ..SearchSettings::default()
        },
        ProposalOptions {
            features: false,
            bases: true,
            ..ProposalOptions::default()
        },
    );
    record_coarse("gate28", &result, &poses, Rigid::IDENTITY, &patch);
    record_measurements("gate28-counters", &format!("{counters:?}"));
    assert!(result.stop.is_none());
    assert!(poses
        .iter()
        .any(|&p| metrics::pose_error(p, Rigid::IDENTITY)[1] < 0.2));
    assert!(poses.iter().any(|&p| metrics::pose_error(
        p,
        Rigid::new(DQuat::IDENTITY, DVec3::new(6., 0., 0.))
    )[1] < 0.2));
}

/// Independent optional allowances may stop descriptor/basis production; the
/// complete geometric grid still runs and reports its actual attempted nodes.
#[test]
fn global_fallback_survives_family_exhaustion() {
    let fixed = arch::plane();
    let (result, poses, counters) = shortlist(
        &input(&fixed, &fixed),
        &SearchSettings::default(),
        ProposalOptions {
            optional_point_pair_limit: Some(0),
            ..ProposalOptions::default()
        },
    );
    assert!(
        result.stop.is_none(),
        "global stop {:?}, counters {counters:?}",
        result.stop
    );
    assert_eq!(result.rotations_attempted, 444);
    assert_eq!(result.families[6].rotations_attempted, 444);
    assert!(result.families[6].complete);
    assert_eq!(result.families[7].interruption, Some(Completion::WorkLimit));
    assert!(result.pool.iter().any(|p| p.id.family == 0));
    assert!(!poses.is_empty());
}

#[test]
fn coherent_landmark_scale_warns_without_extent_heuristics() {
    let mesh = arch::plane();
    let request = input(&mesh, &mesh);
    let points = [
        DVec3::ZERO,
        DVec3::X * 10.,
        DVec3::Y * 10.,
        DVec3::new(3., 7., 2.),
    ];
    for scale in [1., 1.1] {
        let pairs = points.map(|p| crate::PointPair {
            moving_local: p,
            fixed_local: p * scale,
            normals_local: None,
        });
        let request = AlignmentInput {
            landmarks: &pairs,
            ..request
        };
        let ratio = crate::pairs::coherent_landmark_scale(
            &request,
            &occluview_geometry::surface::GeometryControl::unlimited(),
        )
        .unwrap()
        .unwrap();
        assert!((ratio - scale).abs() < 1e-12);
        assert_eq!((ratio - 1.).abs() > 0.02, scale > 1.);
        let frame = crate::SurfaceFrame {
            center_world: DVec3::ZERO,
            query_from_local: DAffine3::IDENTITY,
        };
        assert!(!crate::pairs::landmark_hypotheses(&request, frame, frame).is_empty());
    }
    assert!(crate::pairs::coherent_landmark_scale(
        &request,
        &occluview_geometry::surface::GeometryControl::unlimited()
    )
    .unwrap()
    .is_none());
}

fn record_measurements(name: &str, measurements: &str) {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.refactor-scratch");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{name}-measurements.txt")),
        measurements,
    )
    .unwrap();
}

fn record_coarse(
    name: &str,
    result: &ProposalSearch,
    poses: &[Rigid],
    truth: Rigid,
    moving: &crate::proposal_test_support::SyntheticMesh,
) {
    let center = area_center(moving);
    let errors: Vec<_> = poses
        .iter()
        .map(|&pose| {
            [
                metrics::pose_error(pose, truth)[0],
                pose.apply(center).distance(truth.apply(center)),
                metrics::pose_error(pose, truth)[1],
            ]
        })
        .collect();
    record_measurements(name, &format!(
        "stop={:?}, family_stops={:?}, incomplete_patches={}, examined={}, grid={}, graph={}\nrotation_deg, center_mm, translation_mm={errors:?}\nfamilies={:#?}",
        result.stop, result.family_stops, result.incomplete_patches, result.examined,
        result.rotations_attempted, result.graph_visits, result.families,
    ));
}

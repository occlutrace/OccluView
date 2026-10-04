//! Synthetic multiscale registration gates; no patient fixtures.
#![allow(clippy::unwrap_used, clippy::print_stdout)]
mod support;
use glam::{DQuat, DVec3};
use occluview_align::{
    Completion, Confidence, Metric, NormalPolicy, Rigid, SearchControl, SearchSettings,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use support::{alignment_input, arch, metrics, operators, SyntheticMesh};

struct Accuracy<'a> {
    id: &'a str,
    truth: Rigid,
    tolerance: [f64; 2],
    top1: bool,
}
struct Observation {
    result: occluview_align::AlignmentSearchResult,
    error: [f64; 2],
    probe: f64,
    within_time: bool,
}
impl Observation {
    fn passes(&self, accuracy: &Accuracy<'_>) -> bool {
        self.result.completion == Completion::Complete
            && self.within_time
            && self.error[0] <= accuracy.tolerance[0]
            && self.error[1] <= accuracy.tolerance[1]
    }
}
fn observe(
    moving: &SyntheticMesh,
    fixed: &SyntheticMesh,
    accuracy: &Accuracy<'_>,
    settings: &SearchSettings,
) -> Observation {
    observe_input(
        &alignment_input(moving.soup(), fixed.soup()),
        moving,
        accuracy,
        settings,
    )
}
fn observe_input(
    input: &occluview_align::AlignmentInput<'_>,
    moving: &SyntheticMesh,
    accuracy: &Accuracy<'_>,
    settings: &SearchSettings,
) -> Observation {
    // Every search owns the whole machine for its wall-clock limit. The
    // harness runs these tests on parallel threads, where each search would
    // spend the others' budget and fail on time rather than on its answer.
    static TIMED_SEARCH: Mutex<()> = Mutex::new(());
    let (result, elapsed) = {
        let _alone = TIMED_SEARCH
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let clock = Instant::now();
        let result =
            occluview_align::search_alignment(input, settings, &SearchControl::default()).unwrap();
        (result, clock.elapsed())
    };
    assert!((1..=5).contains(&result.candidates.len()));
    let count = if accuracy.top1 {
        1
    } else {
        result.candidates.len()
    };
    let best = result.candidates[..count]
        .iter()
        .map(|c| (c, metrics::pose_error(c.pose, accuracy.truth)))
        .min_by(|a, b| (a.1[0] + a.1[1]).total_cmp(&(b.1[0] + b.1[1])))
        .unwrap();
    let probes: Vec<_> = (0..moving.positions.len() / 3)
        .step_by((moving.positions.len() / 3 / 1024).max(1))
        .take(1024)
        .map(|i| moving.point(i))
        .collect();
    let probe = metrics::probe_error(best.0.pose, accuracy.truth, &probes);
    let error = best.1;
    println!("{}: completion={:?} class={:?} pose_deg_mm={error:?} probe_rms_mm={} overlap={:?} rounds={} elapsed={elapsed:?}", accuracy.id, result.completion, best.0.confidence, probe[0], best.0.evidence.overlap_smaller, result.work.iterations);
    assert!(result.candidates.iter().all(|c| c.pose.is_finite()
        && (c.pose.rotation.length() - 1.).abs() <= 1e-12
        && (glam::DMat3::from_quat(c.pose.rotation).determinant() - 1.).abs() <= 1e-10));
    Observation {
        result,
        error,
        probe: probe[0],
        within_time: elapsed <= Duration::from_millis(10_100),
    }
}
fn unsigned() -> SearchSettings {
    SearchSettings {
        normal_policy: NormalPolicy::Unsigned,
        ..SearchSettings::default()
    }
}
fn small_truth() -> Rigid {
    Rigid::new(
        DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 5f64.to_radians()),
        DVec3::new(3., -2., 1.),
    )
}
fn far_truth(index: u32) -> Rigid {
    let axis = DVec3::new(
        (f64::from(index) * 1.17 + 0.3).sin(),
        (f64::from(index) * 0.71 + 0.8).cos(),
        0.31 + f64::from(index) / 31.,
    )
    .normalize();
    Rigid::new(
        DQuat::from_axis_angle(axis, std::f64::consts::TAU * (f64::from(index) + 0.5) / 20.),
        DVec3::new(
            32. * (f64::from(index) + 0.2).sin(),
            27. * (f64::from(index) + 0.3).cos(),
            19.,
        ),
    )
}
fn crop_case(
    fraction: f64,
    id: &str,
    tolerance: [f64; 2],
    top1: bool,
    window: operators::CropWindow,
) {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let crop = operators::crop_by_area(&operators::resample_density(&fixed, 1.), fraction, window);
    let (values, cells) = metrics::reference_information(&crop);
    assert!(values[0] >= 0.004 && cells >= 200 && crop.area() >= 100.);
    let mut passed = 0;
    let rms = match id {
        "ID04" | "ID08" => 0.12,
        "ID05" => 0.08,
        "ID06" => 0.06,
        _ => 0.10,
    };
    for truth in [Rigid::IDENTITY, far_truth(7)] {
        let accuracy = Accuracy {
            id,
            truth,
            tolerance,
            top1,
        };
        let moving = operators::rigid_offset(&crop, truth);
        let observation = observe(&moving, &fixed, &accuracy, &unsigned());
        passed += usize::from(observation.passes(&accuracy) && observation.probe <= rms);
    }
    assert_eq!(passed, 2, "{id}: complete accuracy {passed}/2");
}
/// ID02: independent triangulation at 5 degrees / 3,-2,1 mm.
#[test]
fn small_rigid_offset_recovers() {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let accuracy = Accuracy {
        id: "ID02",
        truth: small_truth(),
        tolerance: [0.25, 0.05],
        top1: true,
    };
    let moving = operators::rigid_offset(&operators::resample_density(&fixed, 1.), accuracy.truth);
    let observation = observe(&moving, &fixed, &accuracy, &unsigned());
    assert!(observation.passes(&accuracy) && observation.probe <= 0.06);
}
/// ID04: measured 30% crop, independent triangulation, near and far.
#[test]
fn thirty_percent_moving_crop() {
    crop_case(
        0.3,
        "ID04",
        [0.5, 0.1],
        false,
        operators::CropWindow::Centered,
    );
}
/// ID05: 50% area, top-one.
#[test]
fn fifty_percent_moving_crop() {
    crop_case(
        0.5,
        "ID05",
        [0.3, 0.07],
        true,
        operators::CropWindow::Centered,
    );
}
/// ID06: 70% area, top-one.
#[test]
fn seventy_percent_moving_crop() {
    crop_case(
        0.7,
        "ID06",
        [0.25, 0.05],
        true,
        operators::CropWindow::Centered,
    );
}
/// ID08: one-sided 30% crop with displaced whole-area centroid.
#[test]
fn offset_crop_centroids_do_not_block() {
    crop_case(
        0.3,
        "ID08",
        [0.5, 0.1],
        false,
        operators::CropWindow::OneSided,
    );
}
/// ID09: two islands; no component is discarded.
#[test]
fn disconnected_common_islands() {
    crop_case(
        0.3,
        "ID09",
        [0.5, 0.1],
        false,
        operators::CropWindow::TwoIslands,
    );
}

/// ID07: inverse estimates on all three crop sizes, near/far role orders.
#[test]
fn crop_roles_can_be_swapped() {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let remesh = operators::resample_density(&fixed, 1.);
    let mut passed = 0;
    for fraction in [0.3, 0.5, 0.7] {
        let crop = operators::crop_by_area(&remesh, fraction, operators::CropWindow::Centered);
        for truth in [Rigid::IDENTITY, far_truth(7)] {
            let moving = operators::rigid_offset(&crop, truth);
            let accuracy = Accuracy {
                id: "ID07-forward",
                truth,
                tolerance: [0.5, 0.1],
                top1: false,
            };
            let a = observe(&moving, &fixed, &accuracy, &unsigned());
            let inverse = Accuracy {
                id: "ID07-reverse",
                truth: truth.inverse(),
                tolerance: [0.5, 0.1],
                top1: false,
            };
            let b = observe(&fixed, &moving, &inverse, &unsigned());
            let disagreement = a
                .result
                .candidates
                .iter()
                .flat_map(|a| b.result.candidates.iter().map(move |b| (a, b)))
                .map(|(a, b)| {
                    metrics::probe_error(
                        a.pose,
                        b.pose.inverse(),
                        &metrics::analytic_probes(&arch::ArchSpec::default()),
                    )[0]
                })
                .fold(f64::INFINITY, f64::min);
            println!("ID07 fraction={fraction} inverse_disagreement_mm={disagreement}");
            passed +=
                usize::from(a.passes(&accuracy) && b.passes(&inverse) && disagreement <= 0.10);
        }
    }
    assert_eq!(passed, 6, "complete role pairs {passed}/6");
}
/// ID10: smooth gum returns an uncertain finite geometric placement.
#[test]
fn smooth_crop_keeps_uncertain_pose() {
    let spec = arch::ArchSpec {
        teeth: 0,
        palate_mm: 0.,
        ..arch::ArchSpec::default()
    };
    let fixed = arch::dental_arch(&spec);
    let crop = operators::crop_by_area(
        &operators::resample_density(&fixed, 1.),
        0.3,
        operators::CropWindow::Centered,
    );
    let (information, _) = metrics::reference_information(&crop);
    println!("ID10 independent weakest eigenvalue={}", information[0]);
    assert!(information[0] < 1e-4);
    let accuracy = Accuracy {
        id: "ID10",
        truth: Rigid::IDENTITY,
        tolerance: [180., 100.],
        top1: false,
    };
    let observation = observe(&crop, &fixed, &accuracy, &unsigned());
    assert!(observation
        .result
        .candidates
        .iter()
        .all(|c| matches!(c.confidence, Confidence::Weak | Confidence::Ambiguous)));
    assert!(
        observation.result.completion == Completion::Complete
            && observation.within_time
            && observation.probe <= 0.5
    );
}
/// ID11: exact 1:20 independent remesh, both roles, no density weighting.
#[test]
fn density_ratio_does_not_reweight() {
    let pair = operators::density_pair(&arch::ArchSpec::default());
    let mut passed = 0;
    let mut overlap = Vec::new();
    for roles in [(&pair[0], &pair[1]), (&pair[1], &pair[0])] {
        let accuracy = Accuracy {
            id: "ID11",
            truth: small_truth(),
            tolerance: [0.3, 0.07],
            top1: true,
        };
        let moving = operators::rigid_offset(roles.0, accuracy.truth);
        let observation = observe(&moving, roles.1, &accuracy, &unsigned());
        if let Metric::Measured(value) = observation.result.candidates[0].evidence.overlap_smaller {
            overlap.push(value);
        }
        passed += usize::from(observation.passes(&accuracy));
    }
    assert_eq!(passed, 2);
    assert!((overlap[0] - overlap[1]).abs() <= 0.03);
}
fn noisy_corpus(sigma: f64, id: &str, tolerance: [f64; 2], top1: bool, rms: f64) {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let remesh = operators::resample_density(&fixed, 1.);
    let mut passed = 0;
    for seed in 0..20 {
        println!("{id} seed={seed}");
        let noise = support::NoiseSpec {
            sigma_mm: sigma,
            seed: 20_261_002 + seed,
            correlation_mm: None,
        };
        let moving =
            operators::rigid_offset(&operators::normal_noise(&remesh, noise), small_truth());
        let accuracy = Accuracy {
            id,
            truth: small_truth(),
            tolerance,
            top1,
        };
        let observation = observe(&moving, &fixed, &accuracy, &unsigned());
        passed += usize::from(observation.passes(&accuracy) && observation.probe <= rms);
    }
    println!("{id}: complete recall={passed}/20");
    assert!(passed >= 19, "{id}: {passed}/20");
}
/// ID12: 20 independent low-noise cases; deadlines remain in denominator.
#[test]
fn low_noise_is_supported() {
    noisy_corpus(0.02, "ID12", [0.25, 0.05], true, 0.06);
}
/// ID13: 20 independent higher-noise cases.
#[test]
fn higher_noise_returns_probable() {
    noisy_corpus(0.06, "ID13", [0.5, 0.1], false, 0.12);
}
/// ID14: outliers replace 30% of measured area.
#[test]
fn outlier_area_is_trimmed() {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let moving = operators::rigid_offset(
        &operators::outliers(&operators::resample_density(&fixed, 1.), 0.3, 17),
        small_truth(),
    );
    let accuracy = Accuracy {
        id: "ID14",
        truth: small_truth(),
        tolerance: [0.5, 0.1],
        top1: false,
    };
    assert!(observe(&moving, &fixed, &accuracy, &unsigned()).passes(&accuracy));
}
fn shell_case(roles_swapped: bool, extent: bool) {
    let spec = arch::ArchSpec {
        teeth: 0,
        ..arch::ArchSpec::default()
    };
    let gum = arch::dental_arch(&spec);
    let reference = operators::resample_density(&gum, 1.);
    let mut shell = arch::prosthesis_shell(&gum, 2., &arch::ArchSpec::default());
    if extent {
        shell = operators::outer_extent(&shell, gum.area());
    }
    let truth = Rigid::new(
        DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 57.3f64.to_radians()),
        DVec3::new(7., -5., 4.),
    );
    let moving = operators::rigid_offset(&reference, truth);
    let mut passed = 0;
    for normal_policy in [NormalPolicy::Opposed, NormalPolicy::Unsigned] {
        let settings = SearchSettings {
            normal_policy,
            ..SearchSettings::default()
        };
        let accuracy = Accuracy {
            id: if extent {
                "ID18"
            } else if roles_swapped {
                "ID17"
            } else {
                "ID16"
            },
            truth: if roles_swapped {
                truth.inverse()
            } else {
                truth
            },
            tolerance: if extent { [0.75, 0.15] } else { [0.5, 0.1] },
            top1: false,
        };
        let observation = if roles_swapped {
            observe(&shell, &moving, &accuracy, &settings)
        } else {
            observe(&moving, &shell, &accuracy, &settings)
        };
        passed += usize::from(observation.passes(&accuracy));
    }
    assert_eq!(passed, 2);
}
/// ID16: edentulous gum against independently triangulated tooth-bearing shell.
#[test]
fn gum_intaglio_with_outer_teeth() {
    shell_case(false, false);
}
/// ID17: full-shell source against smaller gum target.
#[test]
fn shell_and_gum_roles_swap() {
    shell_case(true, false);
}
/// ID18: tenfold unrelated exterior.
#[test]
fn outer_extent_tenfold() {
    shell_case(false, true);
}

/// ID19: intended intaglio selected explicitly; exterior is not fit evidence.
#[test]
fn intaglio_roi_disambiguates() {
    let spec = arch::ArchSpec {
        teeth: 0,
        ..arch::ArchSpec::default()
    };
    let gum = arch::dental_arch(&spec);
    let shell = arch::prosthesis_shell(&gum, 2., &arch::ArchSpec::default());
    let noise = support::NoiseSpec {
        sigma_mm: 0.02,
        seed: 19,
        correlation_mm: None,
    };
    let moving = operators::rigid_offset(
        &operators::normal_noise(&operators::resample_density(&gum, 1.), noise),
        small_truth(),
    );
    let mask = operators::reference_roi(&shell, &[2]);
    let mut input = alignment_input(moving.soup(), shell.soup());
    input.fixed.soup.mask = Some(&mask);
    let settings = SearchSettings {
        reference_regions: occluview_align::RegionPolicy::ReferenceRoi,
        normal_policy: NormalPolicy::Opposed,
        ..SearchSettings::default()
    };
    let accuracy = Accuracy {
        id: "ID19",
        truth: small_truth(),
        tolerance: [0.3, 0.07],
        top1: true,
    };
    let observation = observe_input(&input, &moving, &accuracy, &settings);
    assert!(observation.passes(&accuracy) && observation.probe <= 0.08);
}
/// ID25 mesh: the numerical plane information keeps its three null motions.
#[test]
fn same_plane_has_free_motion() {
    let plane = arch::plane();
    let settings = SearchSettings::default();
    let accuracy = Accuracy {
        id: "ID25",
        truth: Rigid::IDENTITY,
        tolerance: [180., 100.],
        top1: true,
    };
    let observation = observe(&plane, &plane, &accuracy, &settings);
    let evidence = &observation.result.candidates[0].evidence;
    assert!(observation.within_time);
    assert!(matches!(evidence.info_eigenvalues, Metric::Measured(values) if values[0] <= 1e-8));
    assert!(evidence.weak_twists.len() >= 3);
    assert!(observation
        .result
        .candidates
        .iter()
        .all(|c| !matches!(c.confidence, Confidence::Verified | Confidence::Probable)));
    assert_eq!(observation.result.completion, Completion::Complete);
}
/// ID26 mesh: finite cylinder candidates; analytic nulls have a separate test.
#[test]
fn same_cylinder_has_free_motion() {
    let cylinder = arch::cylinder();
    let accuracy = Accuracy {
        id: "ID26",
        truth: Rigid::IDENTITY,
        tolerance: [180., 100.],
        top1: false,
    };
    let observation = observe(&cylinder, &cylinder, &accuracy, &SearchSettings::default());
    assert!(observation.within_time);
    assert!(observation.result.candidates.len() >= 2);
    assert!(observation
        .result
        .candidates
        .iter()
        .all(|c| c.confidence != Confidence::Verified));
    assert_eq!(observation.result.completion, Completion::Complete);
}
/// ID27 mesh: faceting is not counted as independent rotation evidence.
#[test]
fn same_sphere_has_free_rotations() {
    let sphere = arch::sphere();
    let accuracy = Accuracy {
        id: "ID27",
        truth: Rigid::IDENTITY,
        tolerance: [180., 100.],
        top1: false,
    };
    let observation = observe(&sphere, &sphere, &accuracy, &SearchSettings::default());
    assert!(observation.within_time);
    assert!(observation
        .result
        .candidates
        .iter()
        .all(|c| !matches!(c.confidence, Confidence::Verified | Confidence::Probable)));
    assert_eq!(observation.result.completion, Completion::Complete);
}
fn scale_corpus(factor: f64, id: &str) {
    let mut unverified = 0;
    for seed in 0..20 {
        let spec = arch::ArchSpec {
            grid: [40, 10],
            asymmetry_seed: seed,
            ..arch::ArchSpec::default()
        };
        let fixed = arch::dental_arch(&spec);
        let moving = operators::scale_geometry(&operators::resample_density(&fixed, 1.), factor);
        let accuracy = Accuracy {
            id,
            truth: Rigid::IDENTITY,
            tolerance: [180., 1e6],
            top1: false,
        };
        let observation = observe(&moving, &fixed, &accuracy, &SearchSettings::default());
        for candidate in &observation.result.candidates {
            assert!(
                (glam::DMat3::from_quat(candidate.pose.rotation).determinant() - 1.).abs() <= 1e-10
            );
            assert!(matches!(
                candidate.confidence,
                Confidence::Weak | Confidence::Ambiguous
            ));
        }
        unverified += usize::from(
            observation.within_time && observation.result.completion == Completion::Complete,
        );
    }
    println!("{id} bounded rigid cases={unverified}/20");
    assert_eq!(unverified, 20);
}
/// ID29: rigid-only outputs, twenty physical 1.10x scale conflicts.
#[test]
fn physical_scale_is_not_fitted_away_factor110() {
    scale_corpus(1.1, "ID29-110");
}
/// ID29: twenty physical 25.4x unit conflicts.
#[test]
fn physical_scale_is_not_fitted_away_factor254() {
    scale_corpus(25.4, "ID29-254");
}
/// ID30: fit the stable 80% ROI, preserving the independently known change.
#[test]
fn reference_fit_preserves_change() {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let changed = operators::change_region(&fixed, 0.2, 1.);
    let changed_crop = operators::crop_by_area(&fixed, 0.2, operators::CropWindow::OneSided);
    let cutoff = changed_crop
        .parameters
        .iter()
        .map(|uv| uv[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let mask: Vec<_> = fixed
        .parameters
        .iter()
        .map(|uv| u8::from(uv[0] <= cutoff))
        .collect();
    let moving = operators::rigid_offset(&changed, small_truth());
    let mut input = alignment_input(moving.soup(), fixed.soup());
    input.moving.soup.mask = Some(&mask);
    input.fixed.soup.mask = Some(&mask);
    let settings = SearchSettings {
        reference_regions: occluview_align::RegionPolicy::ReferenceRoi,
        ..unsigned()
    };
    let accuracy = Accuracy {
        id: "ID30",
        truth: small_truth(),
        tolerance: [0.25, 0.05],
        top1: true,
    };
    let observation = observe_input(&input, &moving, &accuracy, &settings);
    let pose = observation.result.candidates[0].pose;
    let mut differences: Vec<_> = mask
        .iter()
        .enumerate()
        .filter(|(_, value)| **value != 0)
        .map(|(i, _)| pose.apply(moving.point(i)).distance(fixed.point(i)))
        .collect();
    differences.sort_by(f64::total_cmp);
    let median = differences[differences.len() / 2];
    println!("ID30 changed-region oracle median_mm={median}");
    assert!(observation.passes(&accuracy) && median >= 0.8);
}
/// ID31: incoherent winding uses a geometric fallback without invented normals.
#[test]
fn mixed_winding_is_explained() {
    let fixed = arch::dental_arch(&arch::ArchSpec::default());
    let moving = operators::invert_winding(
        &operators::rigid_offset(&operators::resample_density(&fixed, 1.), small_truth()),
        true,
    );
    let accuracy = Accuracy {
        id: "ID31",
        truth: small_truth(),
        tolerance: [0.5, 0.1],
        top1: false,
    };
    let observation = observe(&moving, &fixed, &accuracy, &unsigned());
    assert!(observation.result.candidates.iter().any(|c| c
        .reasons
        .contains(&occluview_align::EvidenceReason::MissingNormals)));
    assert!(observation.result.candidates.iter().all(|c| !c
        .reasons
        .contains(&occluview_align::EvidenceReason::PolicyConflict)));
    assert!(observation.passes(&accuracy));
}

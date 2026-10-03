//! Synthetic result-contract gates and independent fixture invariants.
#![allow(clippy::unwrap_used, clippy::float_cmp)]
mod support;
use glam::{DAffine3, DQuat, DVec3};
use occluview_align::*;
use support::{arch, metrics, operators};

use support::alignment_input as input;
fn search(i: &AlignmentInput<'_>) -> AlignmentSearchResult {
    search_alignment(
        i,
        &SearchSettings {
            profile: SearchProfile::Local,
            ..SearchSettings::default()
        },
        &SearchControl::default(),
    )
    .unwrap()
}
fn proper(result: &AlignmentSearchResult) {
    assert!((1..=5).contains(&result.candidates.len()));
    for c in &result.candidates {
        assert!(c.pose.is_finite());
        assert!((c.pose.rotation.length() - 1.).abs() <= 1e-12);
        assert!((glam::DMat3::from_quat(c.pose.rotation).determinant() - 1.).abs() <= 1e-10);
    }
}

/// ID32: unusable finite geometry always retains a Weak correction, no fake RMS.
#[test]
fn finite_empty_or_masked_returns_weak() {
    let empty = Soup {
        positions: &[],
        indices: &[],
        mask: None,
    };
    let coincident = Soup {
        positions: &[0.; 9],
        indices: &[0, 1, 2],
        mask: None,
    };
    let collinear = Soup {
        positions: &[0., 0., 0., 1., 0., 0., 2., 0., 0.],
        indices: &[0, 1, 2],
        mask: None,
    };
    let invalid = Soup {
        positions: &[0., 0., 0., 1., 0., 0., 0., 1., 0.],
        indices: &[0, u32::MAX, 2],
        mask: None,
    };
    let masked = Soup {
        positions: &[0., 0., 0., 1., 0., 0., 0., 1., 0.],
        indices: &[0, 1, 2],
        mask: Some(&[1, 1, 1]),
    };
    for soup in [empty, coincident, collinear, invalid, masked] {
        let r = search(&input(soup, soup));
        proper(&r);
        assert_eq!(r.candidates[0].confidence, Confidence::Weak);
        assert_eq!(r.candidates[0].pose, Rigid::IDENTITY);
        assert!(matches!(
            r.candidates[0].evidence.euclidean_mm,
            Metric::Missing(_)
        ));
        assert!(matches!(
            r.candidates[0].evidence.plane_mm,
            Metric::Missing(_)
        ));
        assert!(r.candidates[0].evidence.legacy_report.is_none());
    }
}

/// ID33: every supplied numeric field, including excluded/trailing positions,
/// is checked before surface queries. A partial pass is explicit.
#[test]
#[expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "exhaustive scalar-field mutation table includes NaN and infinity conversion"
)]
fn nonfinite_input_is_the_only_numeric_error() {
    let empty = Soup {
        positions: &[],
        indices: &[],
        mask: None,
    };
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for side in 0..2 {
            for scalar in 0..4 {
                let mut positions = [0f32; 4];
                positions[scalar] = bad as f32;
                let soup = Soup {
                    positions: &positions,
                    indices: &[],
                    mask: Some(&[1]),
                };
                let i = if side == 0 {
                    input(soup, empty)
                } else {
                    input(empty, soup)
                };
                assert_eq!(
                    search_alignment(&i, &SearchSettings::default(), &SearchControl::default()),
                    Err(AlignmentInputError::NonFinite {
                        field: if side == 0 {
                            InputField::MovingPositions
                        } else {
                            InputField::FixedPositions
                        },
                        index: scalar
                    })
                );
            }
        }
        for side in 0..2 {
            for scalar in 0..12 {
                let mut a = DAffine3::IDENTITY.to_cols_array();
                a[scalar] = bad;
                let mut i = input(empty, empty);
                if side == 0 {
                    i.moving.world_from_local = DAffine3::from_cols_array(&a);
                } else {
                    i.fixed.world_from_local = DAffine3::from_cols_array(&a);
                }
                assert_eq!(
                    search_alignment(&i, &SearchSettings::default(), &SearchControl::default()),
                    Err(AlignmentInputError::NonFinite {
                        field: if side == 0 {
                            InputField::MovingAffine
                        } else {
                            InputField::FixedAffine
                        },
                        index: scalar
                    })
                );
            }
        }
        for scalar in 0..7 {
            let mut seed = Rigid::IDENTITY;
            if scalar < 4 {
                let mut q = [0., 0., 0., 1.];
                q[scalar] = bad;
                seed.rotation = DQuat::from_array(q);
            } else {
                seed.translation[scalar - 4] = bad;
            }
            let seeds = [seed];
            let mut i = input(empty, empty);
            i.seeds = &seeds;
            assert_eq!(
                search_alignment(&i, &SearchSettings::default(), &SearchControl::default()),
                Err(AlignmentInputError::NonFinite {
                    field: if scalar < 4 {
                        InputField::SeedRotation
                    } else {
                        InputField::SeedTranslation
                    },
                    index: if scalar < 4 { scalar } else { scalar - 4 }
                })
            );
        }
        for scalar in 0..12 {
            let mut pair = PointPair {
                moving_local: DVec3::ZERO,
                fixed_local: DVec3::ZERO,
                normals_local: Some([DVec3::Z; 2]),
            };
            let field = match scalar {
                0..=2 => {
                    pair.moving_local[scalar] = bad;
                    InputField::LandmarkMoving
                }
                3..=5 => {
                    pair.fixed_local[scalar - 3] = bad;
                    InputField::LandmarkFixed
                }
                _ => {
                    let mut n = [DVec3::Z; 2];
                    n[(scalar - 6) / 3][(scalar - 6) % 3] = bad;
                    pair.normals_local = Some(n);
                    InputField::LandmarkNormals
                }
            };
            let pairs = [pair];
            let mut i = input(empty, empty);
            i.landmarks = &pairs;
            assert_eq!(
                search_alignment(&i, &SearchSettings::default(), &SearchControl::default()),
                Err(AlignmentInputError::NonFinite {
                    field,
                    index: if scalar < 3 {
                        scalar
                    } else if scalar < 6 {
                        scalar - 3
                    } else {
                        scalar - 6
                    }
                })
            );
        }
        for field in [InputField::InfluenceRadius, InputField::OverlapPrior] {
            let mut s = SearchSettings::default();
            if field == InputField::InfluenceRadius {
                s.influence_radius_mm = bad;
            } else {
                s.overlap_prior = Some(bad);
            }
            assert_eq!(
                search_alignment(&input(empty, empty), &s, &SearchControl::default()),
                Err(AlignmentInputError::NonFinite { field, index: 0 })
            );
        }
    }
    let c = SearchControl::default();
    c.cancel();
    let r = search_alignment(&input(empty, empty), &SearchSettings::default(), &c).unwrap();
    assert_eq!(r.completion, Completion::Cancelled);
    assert!(matches!(
        r.input_check,
        InputCheck::Partial { checked: 0, .. }
    ));
    proper(&r);
    for q in [
        DQuat::from_array([0.; 4]),
        DQuat::from_xyzw(f64::MAX, 0., 0., f64::MAX),
        DQuat::from_xyzw(1e-300, 0., 0., 1e-300),
    ] {
        let seeds = [Rigid {
            rotation: q,
            translation: DVec3::ZERO,
        }];
        let mut i = input(empty, empty);
        i.seeds = &seeds;
        proper(&search(&i));
    }
}

#[test]
fn finite_unverified_candidates_retain_checkpoint_reasons() {
    let plane = arch::plane();
    let fixed = arch::sphere();
    let result = search(&input(plane.soup(), fixed.soup()));
    proper(&result);
    assert_eq!(result.candidates[0].confidence, Confidence::Weak);
    assert!(result.candidates[0]
        .reasons
        .contains(&EvidenceReason::UniquenessNotEstablished));
    let far = operators::rigid_offset(
        &plane,
        Rigid::new(DQuat::IDENTITY, DVec3::new(100., 0., 0.)),
    );
    let result = search(&input(far.soup(), plane.soup()));
    assert!(result
        .candidates
        .iter()
        .all(|c| c.confidence == Confidence::Weak));
    assert!(result.candidates[0]
        .reasons
        .contains(&EvidenceReason::InsufficientSupport));
    proper(&result);
}

/// Independent negative controls exercise the public geometry path. Missing
/// independent verification cannot certify any of them.
#[test]
fn negative_controls_remain_uncertified() {
    let shapes = [arch::plane(), arch::cylinder(), arch::sphere()];
    for a in 0..3 {
        for b in 0..3 {
            if a != b {
                let r = search(&input(shapes[a].soup(), shapes[b].soup()));
                proper(&r);
                assert!(r
                    .candidates
                    .iter()
                    .all(|c| c.confidence != Confidence::Verified));
            }
        }
    }
    for seed in 0..10 {
        let (a, b) = arch::negative_arch_pair(seed);
        let r = search(&input(a.soup(), b.soup()));
        proper(&r);
        assert!(r
            .candidates
            .iter()
            .all(|c| c.confidence == Confidence::Weak));
    }
}

#[test]
fn fixed_work_provenance_and_candidates_are_reproducible() {
    let plane = arch::plane();
    let request = input(plane.soup(), plane.soup());
    let mut settings = SearchSettings::for_profile(SearchProfile::Local);
    settings.top_k = usize::MAX;
    settings.influence_radius_mm = 6.;
    settings.overlap_prior = Some(-0.2);
    settings.work_budget.query_calls = 99;
    let mut previous = None;
    for _ in 0..3 {
        let mut result = search_alignment(&request, &settings, &SearchControl::default()).unwrap();
        proper(&result);
        let effective = result.provenance.effective_settings.as_ref().unwrap();
        assert_eq!(effective.top_k, 5);
        assert_eq!(effective.influence_radius_mm, 4.);
        assert_eq!(effective.overlap_prior, Some(0.01));
        assert_eq!(effective.wall_limit, std::time::Duration::from_secs(2));
        assert_eq!(effective.work_budget.query_calls, 99);
        assert_eq!(result.provenance.operation_limit, 96_000_000);
        assert_eq!(result.provenance.input_revisions, [1, 2]);
        assert_eq!(result.provenance.algorithm_version, 24);
        assert_eq!(result.completion, Completion::WorkLimit);
        assert!(result.work.query_calls <= 99);
        for family in &result.work.families {
            assert!(family.scored <= family.attempted);
        }
        result.work.elapsed = std::time::Duration::ZERO;
        if let Some(previous) = &previous {
            assert_eq!(previous, &result);
        }
        previous = Some(result);
    }
}

#[test]
fn synthetic_generator_has_measured_area_and_closed_shell() {
    let spec = arch::ArchSpec::default();
    let mesh = arch::dental_arch(&spec);
    assert!(mesh.triangles.len() / 3 < 80_000);
    assert!(mesh.area() > 100.);
    for f in [0.3, 0.5, 0.7] {
        for window in [
            operators::CropWindow::Centered,
            operators::CropWindow::OneSided,
            operators::CropWindow::TwoIslands,
        ] {
            let crop = operators::crop_by_area(&mesh, f, window);
            assert!((crop.area() / mesh.area() - f).abs() <= 0.005);
        }
    }
    let gum_spec = arch::ArchSpec {
        teeth: 0,
        ..spec.clone()
    };
    let gum = arch::dental_arch(&gum_spec);
    let shell = arch::prosthesis_shell(&gum, 2., &spec);
    let mut edges = std::collections::BTreeMap::new();
    for t in shell.triangles.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let key = if a < b { (a, b) } else { (b, a) };
            let e = edges.entry(key).or_insert((0, 0i32));
            e.0 += 1;
            e.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(
        edges.values().all(|&(count, sign)| count == 2 && sign == 0),
        "closed oriented manifold shell"
    );
    let stretched = operators::scale_geometry(&gum, 25.4);
    assert!((stretched.area() / gum.area() - 25.4f64.powi(2)).abs() < 0.001);
    for sigma_mm in [0., 0.02, 0.06] {
        let n = operators::normal_noise(
            &gum,
            support::NoiseSpec {
                sigma_mm,
                seed: 9,
                correlation_mm: None,
            },
        );
        for i in 0..gum.positions.len() / 3 {
            assert!(n.point(i).distance(gum.point(i)) <= 3. * sigma_mm + 1e-5);
        }
    }
    let inverted = operators::invert_winding(&mesh, false);
    assert_eq!(inverted.positions, mesh.positions);
    assert_eq!(inverted.triangles[1], mesh.triangles[2]);
    let probes = metrics::analytic_probes(&spec);
    let truth = Rigid::new(DQuat::from_rotation_x(0.4), DVec3::new(3., -2., 1.));
    assert!(metrics::probe_error(truth, truth, &probes)[0] < 1e-12);
}

/// Shared fixture preconditions are checked before any solver is run.
#[test]
fn synthetic_support_has_independent_truth_and_operators() {
    let spec = arch::ArchSpec::default();
    let mesh = arch::dental_arch(&spec);
    let (eigenvalues, cells) = metrics::reference_information(&mesh);
    assert!(
        eigenvalues[0] >= 0.004,
        "identifiable full-arch eigenvalues: {eigenvalues:?}"
    );
    assert!(cells >= 200);
    let independent = operators::resample_density(&mesh, 1.);
    assert_ne!(independent.positions, mesh.positions);
    assert_ne!(independent.triangles, mesh.triangles);
    assert_eq!(independent.analytic_surface_id, mesh.analytic_surface_id);
    let pair = operators::density_pair(&spec);
    assert_eq!(pair[1].triangles.len(), 20 * pair[0].triangles.len());
    let gum_spec = arch::ArchSpec {
        teeth: 0,
        ..spec.clone()
    };
    let gum = arch::dental_arch(&gum_spec);
    for fraction in [0.1, 0.3] {
        let replaced = operators::outliers(&gum, fraction, 9);
        assert!((replaced.area() / gum.area() - 1.).abs() < 1e-4);
    }
    let perturbation = support::NoiseSpec {
        sigma_mm: 0.06,
        seed: 8,
        correlation_mm: Some(1.),
    };
    let noisy = operators::normal_noise(&gum, perturbation);
    let again = operators::normal_noise(&gum, perturbation);
    assert_eq!(noisy.positions, again.positions);
    for i in 0..gum.positions.len() / 3 {
        assert!(noisy.point(i).distance(gum.point(i)) <= 0.18 + 1e-5);
    }
    let changed = operators::change_region(&gum, 0.2, 1.);
    assert!(changed
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .zip(gum.positions.as_chunks::<3>().0)
        .any(|(a, b)| (a[2] - b[2] - 1.).abs() < 1e-6));
    let repeated = operators::repeat_patch(&gum, 2, 6.);
    assert_eq!(repeated.positions.len(), 2 * gum.positions.len());
    let shell = arch::prosthesis_shell(&gum, 2., &spec);
    let mask = operators::reference_roi(&shell, &[2]);
    assert!(mask.contains(&0) && mask.contains(&1));
    let expanded = operators::outer_extent(&shell, gum.area());
    assert!(((expanded.area() - shell.area()) / gum.area() - 10.).abs() < 1e-4);
    let twin = operators::twin_surfaces(&gum);
    assert_eq!(twin.positions.len(), 2 * gum.positions.len());
    // Exact analytic normals independently retain geometric null modes.
    let plane: Vec<_> = (0..128)
        .map(|i| {
            (
                DVec3::new(f64::from(i % 16), f64::from(i / 16), 0.),
                DVec3::Z,
            )
        })
        .collect();
    let eigenvalues = metrics::information_eigenvalues(&plane);
    assert!(eigenvalues.iter().filter(|&&v| v <= 1e-8).count() >= 3);
    for theta in [-1.2, 0.1, 1.4] {
        let v = 0.7;
        let (_, dt, dv) = arch::band(&spec, theta, v);
        let h = 1e-6;
        let numeric_t =
            (arch::band(&spec, theta + h, v).0 - arch::band(&spec, theta - h, v).0) / (2. * h);
        let numeric_v =
            (arch::band(&spec, theta, v + h).0 - arch::band(&spec, theta, v - h).0) / (2. * h);
        assert!(dt.distance(numeric_t) < 1e-5);
        assert!(dv.distance(numeric_v) < 1e-5);
    }
}

#[test]
fn landmarks_and_unverified_scores_keep_finite_evidence() {
    let plane = arch::plane();
    let pairs = [
        PointPair {
            moving_local: DVec3::ZERO,
            fixed_local: DVec3::X,
            normals_local: Some([DVec3::Z; 2]),
        },
        PointPair {
            moving_local: DVec3::new(3., 0., 0.),
            fixed_local: DVec3::new(4., 0., 0.),
            normals_local: Some([DVec3::Z; 2]),
        },
    ];
    let mut request = input(plane.soup(), plane.soup());
    request.landmarks = &pairs;
    let result = search(&request);
    proper(&result);
    assert_eq!(result.candidates[0].confidence, Confidence::Weak);
    assert!(result
        .candidates
        .iter()
        .any(|c| c.seeds.contains(&SeedOrigin::Landmarks)));
    assert!(result
        .candidates
        .iter()
        .all(|c| c.evidence.legacy_report.is_none()));
    assert!(matches!(
        result.candidates[0].evidence.score,
        Metric::Measured(_)
    ));
    let gum = arch::dental_arch(&arch::ArchSpec {
        teeth: 0,
        grid: [80, 16],
        ..arch::ArchSpec::default()
    });
    let crop = operators::crop_by_area(&gum, 0.3, operators::CropWindow::OneSided);
    let result = search(&input(crop.soup(), gum.soup()));
    proper(&result);
    assert_eq!(result.candidates[0].confidence, Confidence::Weak);
    assert!(
        matches!(result.candidates[0].evidence.score, Metric::Measured(_)),
        "measured finite proposal diagnostics survive without authorization"
    );
}

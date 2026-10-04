//! What a finished fit may do to the scene, using synthetic meshes.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
use crate::align::align_worker::{AlignFailure, AlignOutcome};
use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
use crate::app::OccluViewApp;
use glam::{Affine3A, DQuat, DVec3, Vec3};
use occluview_align::*;
use occluview_core::SceneMeshId;

/// A search result whose best pose is `pose`; confidence is deliberately not
/// inferred.
pub(in crate::app) fn candidate_result(pose: Rigid) -> AlignmentSearchResult {
    let soup = Soup {
        positions: &[],
        indices: &[],
        mask: None,
    };
    let mesh = MeshInput {
        soup,
        world_from_local: glam::DAffine3::IDENTITY,
        revision: 1,
    };
    let input = AlignmentInput {
        moving: mesh,
        fixed: mesh,
        landmarks: &[],
        seeds: &[],
    };
    let mut result = search_alignment(
        &input,
        &SearchSettings::default(),
        &SearchControl::default(),
    )
    .unwrap();
    result.candidates[0].pose = pose;
    result
}

fn fixture(name: &str) -> (OccluViewApp, SceneMeshId, SceneMeshId) {
    let mut app = test_app(name);
    let mut model = named_scene("fixed", 0.);
    let fixed = model.meshes()[0].id();
    let moving = push_named_layer(&mut model, "moving", 5.);
    app.workspace.scenes[0].document.scene = Some(model.into());
    app.workspace.scenes[0].tools.align.tool.arm();
    app.workspace.scenes[0]
        .tools
        .align
        .tool
        .imply_pair(&[moving, fixed]);
    app.active_context().unwrap().align_worker_mut();
    (app, moving, fixed)
}

fn moving_transform(app: &OccluViewApp, moving: SceneMeshId) -> Affine3A {
    app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .iter()
        .find(|entry| entry.id() == moving)
        .unwrap()
        .transform
}

fn status_is(app: &OccluViewApp, key: crate::i18n::MessageId) -> bool {
    app.workspace.scenes[0].tools.align.status.as_deref() == Some(app.ui.locale.tr(key).as_str())
}

/// Best fit matching seats the scan in the best pose it found: no candidate to
/// pick, no acceptance step. The pose is composed with the submitted placement
/// once, lands as one undo step whatever its confidence, and brings the
/// heatmap up with it.
#[test]
fn a_best_fit_result_is_applied_at_once_with_its_heatmap() {
    for confidence in [
        Confidence::Weak,
        Confidence::Probable,
        Confidence::Ambiguous,
        Confidence::Verified,
    ] {
        let (mut app, moving, _) = fixture("best-fit-applied-at-once");
        let authored = Affine3A::from_mat3_translation(
            glam::Mat3::from_cols(Vec3::new(1.2, 0., 0.), Vec3::new(0.1, 0.9, 0.), Vec3::Z),
            Vec3::new(4., -2., 1.),
        );
        app.workspace.scenes[0]
            .document
            .live_scene_mut()
            .unwrap()
            .meshes_mut()[1]
            .transform = authored;
        app.workspace.scenes[0].tools.align.settings.show_deviation = false;
        let pose = Rigid::new(DQuat::from_rotation_y(0.1), DVec3::new(0.5, -0.2, 0.1));
        let expected = pose.to_affine() * authored;
        let mut result = candidate_result(pose);
        result.candidates[0].confidence = confidence;
        // A rival the search ranked lower must never be the one applied.
        let mut rival = result.candidates[0].clone();
        rival.id.proposal = 1;
        rival.pose = Rigid::new(DQuat::IDENTITY, DVec3::new(40., 0., 0.));
        result.candidates.push(rival);

        let mut scene = app.active_context().unwrap();
        scene.land_fit_for_tests(AlignOutcome::Candidates(result));

        assert_eq!(scene.document.edit_mode.undo_len(), 1);
        assert_eq!(
            scene.tools.align.fitted.as_ref().unwrap().confidence,
            confidence
        );
        assert!(scene.alignment_measurement_ready());
        assert!(
            scene.tools.align.settings.show_deviation,
            "a landed fit shows its heatmap without being asked"
        );
        assert!(scene.document.has_unsaved_mesh_edits());
        for (a, b) in moving_transform(&app, moving)
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((a - b).abs() <= 1e-5);
        }

        let mut scene = app.active_context().unwrap();
        scene.apply_history_navigation_now(false, &egui::Context::default());
        assert_eq!(moving_transform(&app, moving), authored);
        assert!(app.workspace.scenes[0].tools.align.fitted.is_none());
        assert!(!app.workspace.scenes[0].tools.align.settings.show_deviation);
    }
}

/// Perform alignment lands the point fit at once, as one undo step, and leaves
/// the heatmap to Best fit matching: clicked points seat nothing.
#[test]
fn a_point_fit_moves_the_scan_without_claiming_a_surface_fit() {
    let (mut app, moving, _) = fixture("point-fit-applied-at-once");
    let correction = Rigid::new(DQuat::from_rotation_z(0.2), DVec3::new(1., 2., -0.5));
    let mut scene = app.active_context().unwrap();
    scene.mark_fitted_for_tests();
    scene.tools.align.settings.show_deviation = true;
    scene.land_fit_for_tests(AlignOutcome::Aligned {
        correction,
        rejected: vec![2],
    });

    assert_eq!(scene.document.edit_mode.undo_len(), 1);
    assert_eq!(scene.tools.align.rejected, vec![2]);
    assert!(scene.tools.align.fitted.is_none());
    assert!(!scene.tools.align.settings.show_deviation);
    assert!(!scene.alignment_measurement_ready());
    assert_eq!(moving_transform(&app, moving), correction.to_affine());
    assert!(status_is(
        &app,
        crate::i18n::message_id!("align-status-aligned")
    ));
}

/// A result is applied only to the pair it was computed from. Each of these
/// changes lands between the submission and the result.
#[test]
fn a_result_for_a_changed_pair_is_dropped() {
    for stale in 0..6 {
        let (mut app, moving, _) = fixture("stale-fit-dropped");
        let mut scene = app.active_context().unwrap();
        scene.tools.align.pending_fit = scene.current_fit_key();
        let before = match stale {
            0 => {
                scene.document.content_revision += 1;
                Affine3A::IDENTITY
            }
            1 => {
                let moved = Affine3A::from_translation(Vec3::Y);
                scene.document.live_scene_mut().unwrap().meshes_mut()[1].transform = moved;
                moved
            }
            2 => {
                scene.document.live_scene_mut().unwrap().meshes_mut()[0].visible = false;
                Affine3A::IDENTITY
            }
            3 => {
                scene.tools.align.settings.influence_radius_mm = 0.3;
                Affine3A::IDENTITY
            }
            4 => {
                scene.tools.align.tool.swap_roles();
                Affine3A::IDENTITY
            }
            _ => {
                scene.tools.align.settings.orientation = Orientation::Ignored;
                Affine3A::IDENTITY
            }
        };
        let worker = scene.align_worker_mut();
        worker.publish_for_tests(
            worker.generation(),
            AlignOutcome::Candidates(candidate_result(Rigid::new(DQuat::IDENTITY, DVec3::X))),
        );
        scene.drain_align_worker(&egui::Context::default());

        assert_eq!(scene.document.edit_mode.undo_len(), 0, "case {stale}");
        assert!(scene.tools.align.fitted.is_none(), "case {stale}");
        assert!(scene.tools.align.pending_fit.is_none(), "case {stale}");
        assert_eq!(moving_transform(&app, moving), before, "case {stale}");
        assert!(
            status_is(&app, crate::i18n::message_id!("align-fit-outdated")),
            "case {stale}"
        );
    }
}

fn fitting_side_and_unrelated_exterior() -> DeviationMap {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for x in 0..=8u8 {
        for y in 0..=8u8 {
            positions.extend([f32::from(x), f32::from(y), 0.]);
        }
    }
    for x in 0..8u32 {
        for y in 0..8u32 {
            let a = x * 9 + y;
            indices.extend([a, a + 9, a + 1, a + 1, a + 9, a + 10]);
        }
    }
    let fixed = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    })
    .unwrap();
    let mut shell_positions = positions.clone();
    for p in shell_positions.as_chunks_mut::<3>().0 {
        p[2] = 0.2;
    }
    for p in positions.as_chunks::<3>().0 {
        shell_positions.extend([p[0], p[1], 5.]);
    }
    let mut shell_indices = indices.clone();
    for t in indices.as_chunks::<3>().0 {
        shell_indices.extend([t[0] + 81, t[2] + 81, t[1] + 81]);
    }
    let shell = Soup {
        positions: &shell_positions,
        indices: &shell_indices,
        mask: None,
    };
    deviation(
        shell,
        &fixed,
        Rigid::IDENTITY,
        &DeviationSettings {
            orientation: Orientation::Ignored,
            ..DeviationSettings::default()
        },
        &CancelFlag::new(),
    )
}

/// Unsupported distances stay grey, and every way the pair can change after a
/// landed fit takes its heatmap authority away for good: returning to the tab
/// does not bring it back.
#[test]
fn partial_fit_keeps_measurement_grey() {
    let moving = Soup {
        positions: &[0., 0., 0., 1., 0., 0., 0., 1., 0., 50., 50., 50.],
        indices: &[0, 1, 2],
        mask: None,
    };
    let fixed = SurfaceIndex::build(Soup {
        positions: &[0., 0., 0., 1., 0., 0., 0., 1., 0.],
        indices: &[0, 1, 2],
        mask: None,
    })
    .unwrap();
    let map = deviation(
        moving,
        &fixed,
        Rigid::IDENTITY,
        &DeviationSettings::default(),
        &CancelFlag::new(),
    );
    let stats = deviation_stats(&map, 0.1);
    assert_ne!(map.validity[3], Validity::Measured);
    assert!(stats.summary.is_none());
    let colors = deviation_colors(&map, &RampSettings::default());
    assert_eq!(colors[3], NO_DATA_COLOR);
    // A crop of the fitting side has enough support for real statistics. The
    // exterior stays unmeasured and cannot dilute the fitting-side residual.
    let map = fitting_side_and_unrelated_exterior();
    let stats = deviation_stats(&map, 0.1);
    assert_eq!(stats.measured, 81);
    assert_eq!(stats.unmeasured.out_of_reach, 81);
    assert_eq!(stats.measured + stats.unmeasured.total(), 162);
    assert!((stats.summary.unwrap().rms - 0.2).abs() < 1e-6);
    assert_eq!(stats.summary.unwrap().within_tolerance, 0.);
    let colors = deviation_colors(&map, &RampSettings::default());
    assert!(colors[81..].iter().all(|c| *c == NO_DATA_COLOR));
    for change in 0..4 {
        let (mut app, moving, _) = fixture("fitted-map-invalidation");
        let mut scene = app.active_context().unwrap();
        scene.land_fit_for_tests(AlignOutcome::Candidates(candidate_result(Rigid::new(
            DQuat::IDENTITY,
            DVec3::X,
        ))));
        assert!(scene.alignment_measurement_ready());
        assert!(scene.tools.align.settings.show_deviation);
        match change {
            0 => scene
                .apply_align_mask_command(crate::align::align_markings::MaskCommand::FitNowhere),
            1 => {
                scene.nudge_align_layer(moving, Affine3A::from_translation(Vec3::Y));
                scene.forget_align_fit("manual pose changed");
            }
            2 => scene.apply_history_navigation_now(false, &egui::Context::default()),
            _ => scene.invalidate_alignment_for_visibility_changes(&[moving]),
        }
        assert!(scene.tools.align.fitted.is_none());
        assert!(!scene.tools.align.settings.show_deviation);
        scene.tools.align.tab = crate::align::align_panel::AlignTab::Automatically;
        scene.settle_align_tab_change();
        assert!(!scene.alignment_measurement_ready());
    }
}

/// Leaving for Adjust pose takes the heatmap down, and coming back does not
/// put it up again: the pose may have been changed by hand in between.
#[test]
fn the_heatmap_does_not_survive_a_trip_to_adjust_pose() {
    let (mut app, _, _) = fixture("heatmap-tab-round-trip");
    let mut scene = app.active_context().unwrap();
    scene.land_fit_for_tests(AlignOutcome::Candidates(candidate_result(Rigid::new(
        DQuat::IDENTITY,
        DVec3::X,
    ))));
    assert!(scene.alignment_measurement_ready());
    assert!(
        scene.align_worker_mut().is_busy() || scene.align_worker_mut().has_pending_output(),
        "the measurement that paints the heatmap is submitted with the fit"
    );
    for tab in [
        crate::align::align_panel::AlignTab::Manually,
        crate::align::align_panel::AlignTab::Automatically,
    ] {
        scene.tools.align.tab = tab;
        scene.settle_align_tab_change();
        assert!(!scene.alignment_measurement_ready());
        assert!(!scene.tools.align.settings.show_deviation);
        assert!(!scene.align_overlay_is_up());
    }
}

/// Hiding the heatmap is a display choice: the fit it describes still stands,
/// so the map can come back without matching again.
#[test]
fn hiding_the_heatmap_keeps_the_fit_it_describes() {
    let (mut app, _, _) = fixture("heatmap-hide-keeps-fit");
    let mut scene = app.active_context().unwrap();
    scene.land_fit_for_tests(AlignOutcome::Candidates(candidate_result(Rigid::new(
        DQuat::IDENTITY,
        DVec3::X,
    ))));
    scene.tools.align.settings.show_deviation = false;
    scene.abandon_align_jobs();
    scene.clear_deviation_overlay();
    assert!(scene.alignment_measurement_ready());
    scene.tools.align.settings.show_deviation = true;
    scene.run_align_measure();
    assert!(scene.align_worker_mut().is_busy() || scene.align_worker_mut().has_pending_output());
}

/// A pose that is not a finite rigid motion never reaches the scene, and a
/// refused job says what was wrong with its input.
#[test]
fn an_invalid_result_cannot_move_the_scan() {
    let (mut app, moving, _) = fixture("invalid-fit-result");
    let mut result = candidate_result(Rigid::IDENTITY);
    result.candidates[0].pose.rotation = DQuat::from_xyzw(f64::NAN, 0., 0., 1.);
    let mut scene = app.active_context().unwrap();
    scene.land_fit_for_tests(AlignOutcome::Candidates(result));
    assert_eq!(scene.document.edit_mode.undo_len(), 0);
    assert!(scene.tools.align.fitted.is_none());

    scene.land_fit_for_tests(AlignOutcome::InvalidInput(AlignmentInputError::NonFinite {
        field: InputField::MovingPositions,
        index: 7,
    }));
    assert!(scene
        .tools
        .align
        .status
        .as_ref()
        .unwrap()
        .contains("MovingPositions"));

    scene.land_fit_for_tests(AlignOutcome::Failed {
        rejection: AlignFailure::Fit(FitRejection::TooFewPairs { have: 1, need: 2 }),
    });
    assert!(scene.tools.align.pending_fit.is_none());
    assert_eq!(moving_transform(&app, moving), Affine3A::IDENTITY);
    assert!(status_is(
        &app,
        crate::i18n::message_id!("align-reject-toofew")
    ));
}

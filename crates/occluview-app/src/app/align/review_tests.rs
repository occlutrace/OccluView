//! Numbered acceptance tests for candidate authority, using synthetic meshes.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
use crate::align::align_worker::AlignOutcome;
use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
use crate::app::OccluViewApp;
use glam::{Affine3A, DQuat, DVec3, Vec3};
use occluview_align::*;
use occluview_core::SceneMeshId;

/// Synthetic result delivery fixture; confidence is deliberately not inferred.
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
fn install(app: &mut OccluViewApp, result: AlignmentSearchResult) {
    let mut scene = app.active_context().unwrap();
    let generation = scene.align_worker_mut().generation();
    scene.tools.align.pending_review = scene.current_alignment_key();
    scene.install_alignment_review(generation, result);
    assert!(scene.tools.align.review.is_some());
}

/// ID39: 100 cycles of five candidates change render uniforms only.
#[test]
fn candidate_review_has_no_scene_side_effects() {
    let (mut app, moving, _) = fixture("candidate-review-pure");
    let mut result = candidate_result(Rigid::IDENTITY);
    let template = result.candidates[0].clone();
    result.candidates = (0..5)
        .map(|i| {
            let mut c = template.clone();
            c.id.proposal = i;
            c.pose = Rigid::new(
                DQuat::from_rotation_z(f64::from(i) * 0.02),
                DVec3::new(f64::from(i), 0., 0.),
            );
            c
        })
        .collect();
    let model = app.workspace.scenes[0].document.scene.clone().unwrap();
    let revision = app.workspace.scenes[0].document.content_revision;
    let original_positions: Vec<_> = model
        .meshes()
        .iter()
        .map(|m| {
            m.mesh
                .vertices()
                .iter()
                .map(|v| v.position)
                .collect::<Vec<_>>()
        })
        .collect();
    let original_indices: Vec<_> = model
        .meshes()
        .iter()
        .map(|m| m.mesh.indices().to_vec())
        .collect();
    install(&mut app, result);
    let mut scene = app.active_context().unwrap();
    let generation = scene.align_worker_mut().generation();
    for _ in 0..100 {
        scene.cycle_alignment_candidate(true);
        let review = scene.tools.align.review.as_ref().unwrap();
        let pose = review.candidates.candidates[review.selected].pose;
        let expected = pose.to_affine() * review.key.transforms[0];
        assert_eq!(scene.alignment_preview_transform(moving), Some(expected));
        let sources = scene.prepared_scene_sources(scene.document.scene.as_ref().unwrap());
        assert_eq!(
            sources[1].uniform.model,
            glam::Mat4::from(expected).to_cols_array()
        );
        scene.toggle_alignment_preview();
        assert!(scene.alignment_preview_transform(moving).is_none());
        scene.toggle_alignment_preview();
    }
    assert_eq!(scene.document.content_revision, revision);
    assert_eq!(scene.document.edit_mode.undo_len(), 0);
    assert!(scene.tools.align.accepted.is_none());
    assert!(scene.tools.align.stats.is_none());
    assert_eq!(scene.align_worker_mut().generation(), generation);
    for (i, entry) in scene
        .document
        .scene
        .as_ref()
        .unwrap()
        .meshes()
        .iter()
        .enumerate()
    {
        assert_eq!(entry.transform, model.meshes()[i].transform);
        assert_eq!(
            entry.mesh.geometry_id(),
            model.meshes()[i].mesh.geometry_id()
        );
        assert_eq!(
            entry
                .mesh
                .vertices()
                .iter()
                .map(|v| v.position)
                .collect::<Vec<_>>(),
            original_positions[i]
        );
        assert_eq!(entry.mesh.indices(), original_indices[i]);
    }
}

/// ID40: accepting every confidence class composes pose*A0 once, retains the
/// class, and creates exactly one undo transaction. Stale keys create none.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "four confidence classes and seven independent revision fences share one acceptance contract"
)]
fn accept_is_one_current_transaction() {
    for confidence in [
        Confidence::Weak,
        Confidence::Probable,
        Confidence::Ambiguous,
        Confidence::Verified,
    ] {
        let (mut app, moving, _) = fixture("accept-current-candidate");
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
        let pose = Rigid::new(DQuat::from_rotation_y(0.1), DVec3::new(0.5, -0.2, 0.1));
        let expected = pose.to_affine() * authored;
        let mut result = candidate_result(pose);
        result.candidates[0].confidence = confidence;
        result.candidates[0].evidence.common_area_mm2 = Metric::Measured(123.);
        install(&mut app, result);
        let mut scene = app.active_context().unwrap();
        assert!(scene.accept_alignment_candidate());
        assert!(!scene.accept_alignment_candidate());
        assert_eq!(scene.document.edit_mode.undo_len(), 1);
        assert_eq!(
            scene.tools.align.accepted.as_ref().unwrap().confidence,
            confidence
        );
        assert_eq!(
            scene
                .tools
                .align
                .accepted
                .as_ref()
                .unwrap()
                .evidence
                .common_area_mm2,
            Metric::Measured(123.)
        );
        let absolute = scene
            .document
            .scene
            .as_ref()
            .unwrap()
            .meshes()
            .iter()
            .find(|m| m.id() == moving)
            .unwrap()
            .transform;
        for (a, b) in absolute
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((a - b).abs() <= 1e-5);
        }
        assert!(scene.document.has_unsaved_mesh_edits());
        assert!(scene.alignment_preview_transform(moving).is_none());
        scene.apply_history_navigation_now(false, &egui::Context::default());
        assert_eq!(
            scene.document.scene.as_ref().unwrap().meshes()[1].transform,
            authored
        );
        assert!(scene.tools.align.accepted.is_none());
    }
    for stale in 0..7 {
        let (mut app, _, _) = fixture("accept-stale-candidate");
        install(
            &mut app,
            candidate_result(Rigid::new(DQuat::IDENTITY, DVec3::X)),
        );
        let mut scene = app.active_context().unwrap();
        match stale {
            0 => {
                scene.align_worker_mut().bump_generation();
            }
            1 => scene.document.content_revision += 1,
            2 => {
                scene.document.live_scene_mut().unwrap().meshes_mut()[1].transform =
                    Affine3A::from_translation(Vec3::Y);
            }
            3 => scene.document.live_scene_mut().unwrap().meshes_mut()[0].visible = false,
            4 => scene.tools.align.settings.influence_radius_mm = 0.3,
            5 => {
                scene.tools.align.tool.swap_roles();
            }
            _ => scene.tools.align.settings.orientation = Orientation::Ignored,
        }
        assert!(!scene.accept_alignment_candidate());
        assert_eq!(scene.document.edit_mode.undo_len(), 0);
        assert!(scene.tools.align.accepted.is_none());
        assert!(scene.tools.align.review.is_none());
        assert_eq!(
            scene.tools.align.status.as_deref(),
            Some(
                scene
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("align-review-invalidated"))
                    .as_str()
            )
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

/// ID41: unsupported distances stay grey, and real mask/manual/history paths
/// revoke review and accepted authority without reviving it on tab return.
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
        let (mut app, moving, _) = fixture("accepted-map-invalidation");
        install(
            &mut app,
            candidate_result(Rigid::new(DQuat::IDENTITY, DVec3::X)),
        );
        let mut scene = app.active_context().unwrap();
        assert!(scene.accept_alignment_candidate());
        assert!(scene.alignment_measurement_ready());
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
        assert!(scene.tools.align.accepted.is_none());
        assert!(scene.tools.align.review.is_none());
        assert!(!scene.tools.align.settings.show_deviation);
        scene.tools.align.tab = crate::align::align_panel::AlignTab::Automatically;
        scene.settle_align_tab_change();
        assert!(!scene.alignment_measurement_ready());
        assert!(!scene.align_worker_mut().is_busy());
    }
}

/// Invalid numeric payloads cannot create a preview or scene transaction.
#[test]
fn invalid_candidate_cannot_reach_preview_or_accept() {
    let (mut app, moving, _) = fixture("invalid-candidate-preview");
    let mut result = candidate_result(Rigid::IDENTITY);
    result.candidates[0].pose.rotation = DQuat::from_xyzw(f64::NAN, 0., 0., 1.);
    let mut scene = app.active_context().unwrap();
    let generation = scene.align_worker_mut().generation();
    scene.tools.align.pending_review = scene.current_alignment_key();
    scene.install_alignment_review(generation, result);
    assert!(scene.alignment_preview_transform(moving).is_none());
    assert!(!scene.accept_alignment_candidate());
    assert_eq!(scene.document.edit_mode.undo_len(), 0);
    scene.align_worker_mut().publish_for_tests(
        generation,
        AlignOutcome::InvalidInput(AlignmentInputError::NonFinite {
            field: InputField::MovingPositions,
            index: 7,
        }),
    );
    scene.drain_align_worker(&egui::Context::default());
    assert!(scene.tools.align.review.is_none());
    assert!(scene
        .tools
        .align
        .status
        .as_ref()
        .unwrap()
        .contains("MovingPositions"));
}

#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]
use super::*;
use glam::{Quat, Vec3};
use occluview_core::test_support::quad_mesh;
use occluview_core::{Mesh, SceneMesh};
use std::thread;

#[test]
fn toggling_a_tool_arms_it_and_toggling_again_disarms() {
    let mut tool = SculptTool::default();
    tool.toggle(SculptToolKind::AddRemove);
    assert_eq!(tool.armed, Some(SculptToolKind::AddRemove));
    tool.toggle(SculptToolKind::Smooth);
    assert_eq!(tool.armed, Some(SculptToolKind::Smooth));
    tool.toggle(SculptToolKind::Smooth);
    assert_eq!(tool.armed, None);
}

#[test]
fn shift_flips_add_to_remove_and_forces_smooth() {
    assert_eq!(
        SculptToolKind::AddRemove.brush_mode(false, false),
        BrushMode::Add
    );
    assert_eq!(
        SculptToolKind::AddRemove.brush_mode(true, false),
        BrushMode::Remove
    );
    assert_eq!(
        SculptToolKind::AddRemove.brush_mode(true, true),
        BrushMode::Relax
    );
    assert_eq!(
        SculptToolKind::AddRemove.brush_mode(false, true),
        BrushMode::Add
    );
    assert_eq!(
        SculptToolKind::Smooth.brush_mode(true, true),
        BrushMode::Smooth
    );
    // Shift strengthens Smooth by twice the slider, clamped to the top of the
    // range; Add/Remove follows the intensity slider with or without Shift.
    // A forced maximum flattened whole cusps from the bottom of the slider.
    assert!((SculptToolKind::Smooth.dab_strength(0.3, true) - 0.6).abs() < 1e-6);
    assert_eq!(SculptToolKind::Smooth.dab_strength(0.7, true), 1.0);
    assert_eq!(SculptToolKind::Smooth.dab_strength(1.0, false), 1.0);
    assert_eq!(SculptToolKind::AddRemove.dab_strength(0.3, false), 0.3);
    assert_eq!(SculptToolKind::AddRemove.dab_strength(0.3, true), 0.3);
}

#[test]
fn sculpt_controls_use_the_donor_catalog_defaults_and_physical_ranges() {
    assert_eq!(SculptTip::Ball.default_radius_mm(), 0.75);
    assert_eq!(SculptTip::Knife.default_radius_mm(), 0.5);
    assert_eq!(SculptTip::Cylinder.default_radius_mm(), 0.5);
    assert_eq!(SculptTip::Ball.radius_range_mm(), (0.25, 4.0));
    assert_eq!(SculptTip::Knife.radius_range_mm(), (0.25, 2.5));
    assert_eq!(SculptTip::Cylinder.radius_range_mm(), (0.25, 2.0));
    assert_eq!(SculptToolKind::AddRemove.default_strength(), 0.35);
    assert_eq!(SculptToolKind::Smooth.default_strength(), 0.15);
    assert_eq!(SculptToolKind::AddRemove.strength_range(), (0.05, 1.0));
    assert_eq!(SculptToolKind::Smooth.strength_range(), (0.01, 1.0));
}

#[test]
fn brush_wheel_detents_follow_donor_ratios_and_slider_quantization() {
    assert_eq!(SculptToolKind::AddRemove.step_strength(0.35, 1.0), 0.45);
    assert_eq!(SculptToolKind::AddRemove.step_strength(0.35, -1.0), 0.25);
    assert_eq!(SculptToolKind::Smooth.step_strength(0.15, 1.0), 0.2);
    assert_eq!(SculptToolKind::Smooth.step_strength(0.15, -1.0), 0.12);
    assert_eq!(SculptTip::Ball.step_radius_mm(0.75, 1.0), 0.9);
    assert_eq!(SculptTip::Ball.step_radius_mm(0.75, -1.0), 0.65);
    assert_eq!(SculptTip::Knife.step_radius_mm(2.5, 1.0), 2.5);
    assert_eq!(SculptTip::Cylinder.step_radius_mm(0.25, -1.0), 0.25);
}

#[test]
fn mean_uniform_scale_reads_a_rigid_transform_as_one() {
    let rigid =
        Affine3A::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(3.0, -2.0, 9.0));
    assert!((mean_uniform_scale(&rigid) - 1.0).abs() < 1e-5);
}

#[test]
fn mean_uniform_scale_survives_a_degenerate_transform() {
    assert_eq!(mean_uniform_scale(&Affine3A::from_scale(Vec3::ZERO)), 1.0);
}

#[test]
fn sculpt_accepts_only_a_positive_orthogonal_uniform_scale() {
    let rigid =
        Affine3A::from_rotation_translation(Quat::from_rotation_z(0.4), Vec3::new(1.0, 2.0, 3.0));
    assert!(uniform_scene_scale(&rigid).is_some());
    assert!(uniform_scene_scale(&Affine3A::from_scale(Vec3::new(1.0, 2.0, 1.0))).is_none());
    assert!(uniform_scene_scale(&Affine3A::from_scale(Vec3::new(-1.0, -1.0, -1.0))).is_none());
}

#[test]
fn persistent_session_accepts_a_second_stroke_after_first_commit() {
    let mesh = Mesh::new(
        Some("sculpt-test".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh");
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let mut session = SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    let first = session.apply_dab(stroke, BrushMode::Add);
    assert!(
        !first.touched.is_empty() || first.topology_delta.is_some(),
        "the first dab must reach the surface"
    );
    session.dirty_stroke = false;
    let second = session.apply_dab(stroke, BrushMode::Add);
    assert!(
        !second.touched.is_empty() || second.topology_delta.is_some(),
        "the persistent session must accept a second stroke"
    );
}

#[test]
fn poisoned_shadow_is_a_terminal_dab_failure() {
    let mesh = Mesh::new(
        Some("poisoned-shadow".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh");
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let shadow = Arc::new(RwLock::new(mesh.vertices().to_vec()));
    let poison_target = Arc::clone(&shadow);
    let poison = thread::spawn(move || {
        let _guard = poison_target.write().expect("shadow lock");
        panic!("test poison");
    });
    assert!(poison.join().is_err());

    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let mut session = SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow,
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };

    let outcome = session.apply_dab(stroke, BrushMode::Add);
    assert_eq!(outcome.failure, Some(DabFailure::ShadowPoisoned));
    assert!(outcome.touched.is_empty());
    assert!(!session.dirty_stroke);
}

#[test]
fn invalid_shadow_mapping_fails_before_partial_publish() {
    let mesh = Mesh::new(
        Some("invalid-shadow-mapping".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh");
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let original = mesh.vertices().to_vec();
    let shadow = Arc::new(RwLock::new(original.clone()));
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let mut session = SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::clone(&shadow),
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };

    let failure = session
        .patch_shadow(&[0, original.len()], &[], None)
        .expect_err("an out-of-range kernel id must be terminal");
    assert_eq!(
        failure,
        DabFailure::InvalidVertexIndex {
            vertex_id: original.len(),
            vertex_count: original.len(),
        }
    );
    assert_eq!(*shadow.read().expect("shadow read"), original);
}

/// A brush session is prepared off the UI thread, so for a frame or two there
/// is no worker yet — and the mesh edit that a worker would gate on must also
/// wait for the preparation. Reporting quiet there lets a
/// Done/undo/structural edit invalidate the session being built.
#[test]
fn sculpt_preparation_counts_as_busy_before_the_worker_exists() {
    let mut tool = SculptTool::default();
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(
        quad_mesh(Some("prepare-busy")).expect("test mesh"),
    ));
    let scene = Arc::new(scene);
    let layer_id = scene.meshes()[index].id();
    let topology_id = scene.meshes()[index].mesh.topology_id();

    tool.queue_preparation(Arc::clone(&scene), index);

    assert!(
        tool.worker.is_none(),
        "the worker lands only after preparation"
    );
    assert!(
        tool.pending_matches(layer_id, topology_id),
        "the preparation must be in flight"
    );
    assert!(
        tool.is_busy(),
        "a mesh edit must wait for the session that is still being prepared"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let result = tool
        .pending
        .as_ref()
        .expect("preparation worker")
        .receiver
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .expect("preparation never landed");
    let pending = tool.pending.take().expect("preparation worker");
    let _ = pending.thread.join();
    let session = result.expect("the preparation worker succeeds");
    assert_eq!(session.layer_id, layer_id);
    assert!(
        !tool.is_busy(),
        "a landed, idle session is not work the guards have to wait for"
    );
}

/// The stroke's undo baseline is snapshotted cold. It is stored and usually
/// dropped — most strokes are never undone — and the full form's caches land on
/// the first dab of every stroke, where the operator is waiting.
#[test]
fn the_stroke_baseline_is_snapshotted_cold() {
    let mesh = quad_mesh(Some("cold-baseline")).expect("test mesh");
    mesh.warm_bvh();
    let original = mesh.vertices().to_vec();
    let topology_id = mesh.topology_id();
    let topology = PreparedSceneTopology::from_mesh(&mesh);
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let mut session = SculptSession {
        layer_id,
        topology_id,
        session: brush,
        base_mesh: Arc::new(mesh),
        shadow: Arc::new(RwLock::new(original.clone())),
        topology,
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };

    let outcome = session.apply_dab(
        BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        },
        BrushMode::Add,
    );
    assert!(
        !outcome.touched.is_empty() || outcome.topology_delta.is_some(),
        "the dab has to change geometry, or there is no baseline to snapshot"
    );

    let baseline = session
        .stroke_start_mesh
        .as_ref()
        .expect("the first dab snapshots the undo baseline");
    assert_eq!(
        baseline.vertices(),
        original.as_slice(),
        "the baseline is the pre-stroke geometry"
    );
    assert_eq!(baseline.topology_id(), topology_id);
    assert!(
        !baseline.bvh_is_ready(),
        "the baseline must not refit a picking tree on the operator's first dab"
    );
    assert!(
        !baseline.bbox_is_cached(),
        "nor rebuild the bounding box for a mesh that is most likely dropped"
    );
}

#[test]
fn shadow_shape_mismatch_is_not_treated_as_an_empty_dab() {
    let mesh = Mesh::new(
        Some("shadow-shape-mismatch".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh");
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
    let mut session = SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::new(RwLock::new(Vec::new())),
        topology: PreparedSceneTopology::from_mesh(&mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };

    let failure = session
        .patch_shadow(&[0], &[], None)
        .expect_err("a shadow with the wrong shape must be terminal");
    assert_eq!(
        failure,
        DabFailure::ShadowShapeMismatch {
            shadow_count: 0,
            live_count: mesh.vertices().len(),
        }
    );
}

/// Disarming abandons a preparation that is still in flight, and the abandoned
/// worker never installs its session. A preparation that landed after the tool
/// was disarmed would attach a worker with no armed brush and warm a picking
/// tree for a scan that is not being sculpted.
#[test]
fn an_abandoned_preparation_never_installs_its_session() {
    let mut tool = SculptTool::default();
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(
        quad_mesh(Some("abandoned-preparation")).expect("test mesh"),
    ));
    let scene = Arc::new(scene);
    let layer_id = scene.meshes()[index].id();
    let topology_id = scene.meshes()[index].mesh.topology_id();

    assert!(
        !tool.queue_preparation(Arc::clone(&scene), index),
        "a fresh preparation reports that it has not landed yet"
    );
    assert!(
        tool.pending_matches(layer_id, topology_id),
        "the preparation has to be in flight, or this proves nothing"
    );

    tool.disarm();

    assert!(
        !tool.pending_matches(layer_id, topology_id),
        "disarming must abandon the in-flight preparation"
    );
    assert!(
        !tool.is_busy(),
        "an abandoned preparation must not keep the tool reading as busy"
    );
    assert!(
        tool.poll_preparation().is_none(),
        "an abandoned preparation must never be collected as a session"
    );
    assert!(
        tool.worker.is_none(),
        "so no worker is installed for a brush the operator has put down"
    );
}

/// A regular 1 mm grid over the given extent, in the test mesh's own layout.
// A fixture: the grid indices and coordinates are far below any precision limit.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn flat_grid(side: usize) -> Mesh {
    let mut vertices = Vec::with_capacity(side * side);
    for j in 0..side {
        for i in 0..side {
            vertices.push(Vertex::at(Vec3::new(i as f32, j as f32, 0.0)));
        }
    }
    let mut indices = Vec::new();
    let idx = |i: usize, j: usize| (j * side + i) as u32;
    for j in 0..side - 1 {
        for i in 0..side - 1 {
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            indices.extend_from_slice(&[idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Mesh::new(Some("flat-grid".to_string()), vertices, indices).expect("grid mesh")
}

fn session_over(mesh: &Mesh) -> (SculptSession, Arc<RwLock<Vec<Vertex>>>) {
    let layer_id = SceneMesh::new(mesh.clone()).id();
    let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(mesh)).expect("prepare");
    let shadow = Arc::new(RwLock::new(mesh.vertices().to_vec()));
    let session = SculptSession {
        layer_id,
        topology_id: mesh.topology_id(),
        session: brush,
        base_mesh: Arc::new(mesh.clone()),
        shadow: Arc::clone(&shadow),
        topology: PreparedSceneTopology::from_mesh(mesh),
        world_to_local: Affine3A::IDENTITY,
        local_per_world: 1.0,
        dirty_stroke: false,
        topology_dirty_stroke: false,
        stroke_start_mesh: None,
    };
    (session, shadow)
}

/// How far the vertices the dab actually moved spread along x and along y
/// from the dab centre. Read from the live shadow rather than the touched list,
/// because a dab that also retessellated the patch reports a topology delta.
fn moved_spread(shadow: &Arc<RwLock<Vec<Vertex>>>, original: &[Vertex]) -> (f32, f32) {
    let shadow = shadow.read().expect("shadow lock");
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    for (live, before) in shadow.iter().zip(original) {
        if live.position == before.position {
            continue;
        }
        x = x.max((live.position[0] - 4.0).abs());
        y = y.max((live.position[1] - 4.0).abs());
    }
    (x, y)
}

/// The tip and the stroke bearing reach the kernel: a knife dab cuts further
/// along its bearing than across it, while a ball dab spreads evenly.
#[test]
fn the_knife_tip_cuts_along_its_bearing() {
    let mesh = flat_grid(9);
    let stroke = BrushStroke {
        center: [4.0, 4.0, 0.0],
        radius_mm: 3.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    let original = mesh.vertices().to_vec();
    let (mut ball_session, ball_shadow) = session_over(&mesh);
    let _ = ball_session.apply_dab(stroke, BrushMode::Add);
    let (mut knife_session, knife_shadow) = session_over(&mesh);
    let _ = knife_session.apply_dab_tipped(
        stroke,
        BrushMode::Add,
        SculptTip::Knife,
        Some([1.0, 0.0, 0.0]),
    );

    let (ball_x, ball_y) = moved_spread(&ball_shadow, &original);
    let (knife_x, knife_y) = moved_spread(&knife_shadow, &original);
    assert!(
        ball_x > 0.0 && knife_x > 0.0,
        "both dabs must move something"
    );
    assert!(
        knife_x > knife_y * 1.5,
        "the knife must reach along its bearing: x={knife_x} y={knife_y}"
    );
    assert!(
        (ball_x - ball_y).abs() <= 1.0,
        "the ball must spread evenly: x={ball_x} y={ball_y}"
    );
}

/// One hold interval in milliseconds, which is what the scheduler reports.
fn hold_ms() -> f32 {
    HOLD_DAB_INTERVAL_SEC * 1000.0
}

/// Share of a full dose one hold interval stands for.
#[allow(
    clippy::cast_possible_truncation,
    reason = "The fixed 120 ms dose is exactly representable as f32."
)]
fn share_of_full_dose() -> f32 {
    HOLD_DAB_INTERVAL_SEC * 1000.0 / occluview_sculpt::DWELL_FULL_DOSE_MS as f32
}

/// The dwell a caller reports doses the dab, so four hold intervals deposit
/// what one full dab does. A caller that stamped a full dose per frame instead
/// would quadruple the material a held brush lays down.
#[test]
fn four_hold_dabs_deposit_one_full_dose() {
    let mesh = flat_grid(9);
    let stroke = BrushStroke {
        center: [4.0, 4.0, 0.0],
        radius_mm: 3.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    let lifted = |doses: &[DabDose]| -> Vec<f32> {
        let (mut session, shadow) = session_over(&mesh);
        for dose in doses {
            let outcome = session
                .apply_dab_cancellable(
                    stroke,
                    BrushMode::Add,
                    &AtomicBool::new(false),
                    SculptTip::Ball,
                    None,
                    *dose,
                )
                .expect("an uncancelled dab returns an outcome");
            assert!(outcome.failure.is_none(), "the dab completes");
        }
        let lifted: Vec<f32> = shadow
            .read()
            .expect("shadow lock")
            .iter()
            .map(|vertex| vertex.position[2])
            .collect();
        lifted
    };

    let one_full = lifted(&[DabDose::FULL]);
    let four_holds = lifted(&[
        DabDose::dwell(hold_ms()),
        DabDose::dwell(hold_ms()),
        DabDose::dwell(hold_ms()),
        DabDose::dwell(hold_ms()),
    ]);
    let peak = |heights: &[f32]| heights.iter().copied().fold(0.0f32, f32::max);
    let full_peak = peak(&one_full);
    let holds_peak = peak(&four_holds);
    assert!(full_peak > 0.0, "the full-dose dab must lift the surface");
    // Four hold dabs land in four separate kernel steps, each with its own
    // auto-smoothing and face guard, so the total is close to one full dose
    // rather than bit-identical to it. A caller that stamped a full dose per
    // frame lands near four times this, which the bound refuses.
    let ratio = holds_peak / full_peak;
    assert!(
        (ratio - 1.0).abs() < 0.2,
        "four 30 ms hold dabs must deposit about one full dose: full={full_peak} holds={holds_peak} ratio={ratio}"
    );
}

/// A hold dab that carries no dwell would still stand for a full dose; this
/// pins the dose the kernel actually receives, not the scheduler's plan.
#[test]
fn a_hold_dab_doses_less_than_a_travelled_dab() {
    let mesh = flat_grid(9);
    let stroke = BrushStroke {
        center: [4.0, 4.0, 0.0],
        radius_mm: 3.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    let lift = |dose: DabDose| -> f32 {
        let (mut session, shadow) = session_over(&mesh);
        let _ = session
            .apply_dab_cancellable(
                stroke,
                BrushMode::Add,
                &AtomicBool::new(false),
                SculptTip::Ball,
                None,
                dose,
            )
            .expect("an uncancelled dab returns an outcome");
        let peak: f32 = shadow
            .read()
            .expect("shadow lock")
            .iter()
            .map(|vertex| vertex.position[2])
            .fold(0.0f32, f32::max);
        peak
    };
    let full = lift(DabDose::FULL);
    let quarter = lift(DabDose::dwell(hold_ms()));
    assert!(full > 0.0 && quarter > 0.0);
    assert!(
        (quarter / full - share_of_full_dose()).abs() < 0.02,
        "one hold interval must deposit its share of a full dose: full={full} hold={quarter}"
    );
}

/// The dwell a caller reports is the only thing that decides how far a dab
/// moves the surface: Add and Remove mirror each other, strength scales the
/// dose, and neither end of the controls runs away.
#[test]
fn the_dose_controls_depth_symmetry_and_strength() {
    let mesh = flat_grid(9);
    let extremes = |strength: f32, radius_mm: f32, mode: BrushMode| -> (f32, f32) {
        let (mut session, shadow) = session_over(&mesh);
        let stroke = BrushStroke {
            center: [4.0, 4.0, 0.0],
            radius_mm,
            strength,
            view_dir: [0.0, 0.0, -1.0],
        };
        let outcome = session
            .apply_dab_cancellable(
                stroke,
                mode,
                &AtomicBool::new(false),
                SculptTip::Ball,
                None,
                DabDose::FULL,
            )
            .expect("an uncancelled dab returns an outcome");
        assert!(outcome.failure.is_none(), "the dab completes");
        let shadow = shadow.read().expect("shadow lock");
        shadow.iter().fold((0.0f32, 0.0f32), |(high, low), vertex| {
            (high.max(vertex.position[2]), low.min(vertex.position[2]))
        })
    };

    let (add_high, add_low) = extremes(1.0, 3.0, BrushMode::Add);
    let (remove_high, remove_low) = extremes(1.0, 3.0, BrushMode::Remove);
    assert!(
        add_high > 0.05 && add_low > -0.005,
        "Add lifts the surface: high={add_high} low={add_low}"
    );
    assert!(
        remove_low < -0.05 && remove_high < 0.005,
        "Remove lowers the surface: high={remove_high} low={remove_low}"
    );
    assert!(
        (add_high + remove_low).abs() <= add_high * 0.05,
        "Add {add_high} and Remove {remove_low} mirror each other"
    );

    let (half_high, _) = extremes(0.5, 3.0, BrushMode::Add);
    let ratio = half_high / add_high;
    assert!(
        (ratio - 0.5).abs() < 0.1,
        "strength scales the dose: half strength lifted {ratio} of full"
    );

    for (strength, radius) in [(0.0_f32, 0.4_f32), (1.0, 12.0)] {
        let (high, low) = extremes(strength, radius, BrushMode::Add);
        assert!(high.is_finite() && low.is_finite(), "finite at the ends");
        assert!(
            high <= 1.5,
            "radius {radius} at strength {strength} lifted {high} mm in one dab"
        );
    }
}

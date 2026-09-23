#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]
use super::*;
use glam::Quat;
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
    assert_eq!(SculptToolKind::AddRemove.brush_mode(false), BrushMode::Add);
    assert_eq!(
        SculptToolKind::AddRemove.brush_mode(true),
        BrushMode::Remove
    );
    assert_eq!(SculptToolKind::Smooth.brush_mode(true), BrushMode::Smooth);
    // Shift forces Smooth to maximum regardless of the slider; Add/Remove
    // follows the intensity slider with or without Shift.
    assert_eq!(SculptToolKind::Smooth.dab_strength(0.3, true), 1.0);
    assert_eq!(SculptToolKind::Smooth.dab_strength(1.0, false), 1.0);
    assert_eq!(SculptToolKind::AddRemove.dab_strength(0.3, false), 0.3);
    assert_eq!(SculptToolKind::AddRemove.dab_strength(0.3, true), 0.3);
}

#[test]
fn shift_widens_only_the_smooth_footprint() {
    let base = size_to_radius_mm(SCULPT_SIZE_DEFAULT);
    assert_eq!(
        SculptToolKind::Smooth.dab_radius_mm(base, true),
        base * SHIFT_SMOOTH_RADIUS_BOOST
    );
    assert_eq!(SculptToolKind::Smooth.dab_radius_mm(base, false), base);
    assert_eq!(SculptToolKind::AddRemove.dab_radius_mm(base, true), base);
}

#[test]
fn size_slider_maps_monotonically_into_the_mm_range() {
    assert!(size_to_radius_mm(SCULPT_SIZE_MIN) < size_to_radius_mm(SCULPT_SIZE_MAX));
    assert!(size_to_radius_mm(SCULPT_SIZE_MIN) >= SCULPT_RADIUS_MIN_MM - 1e-4);
    assert!(size_to_radius_mm(SCULPT_SIZE_MAX) <= SCULPT_RADIUS_MAX_MM + 1e-4);
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
        stroke_start_mesh: None,
    };
    let stroke = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 2.0,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(!session.apply_dab(stroke, BrushMode::Add).touched.is_empty());
    session.dirty_stroke = false;
    assert!(!session.apply_dab(stroke, BrushMode::Add).touched.is_empty());
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
        stroke_start_mesh: None,
    };

    let failure = session
        .patch_shadow(&[0, original.len()], &[])
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
/// is no worker yet — and the mesh edit that a worker would gate on is exactly
/// the one that must wait for the preparation. Reporting quiet there lets a
/// Done/undo/structural edit invalidate the session being built.
#[test]
fn sculpt_preparation_counts_as_busy_before_the_worker_exists() {
    let mut tool = SculptTool::default();
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(quad_mesh("prepare-busy")));
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
    let session = loop {
        if let Some(result) = tool.poll_preparation() {
            break result.expect("the preparation worker succeeds");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "preparation never landed"
        );
        thread::sleep(std::time::Duration::from_millis(1));
    };
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
    let mesh = quad_mesh("cold-baseline");
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
        !outcome.touched.is_empty(),
        "the dab has to move geometry, or there is no baseline to snapshot"
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

/// A four-vertex quad: small enough to sculpt immediately, real enough that a
/// dab moves something.
fn quad_mesh(name: &str) -> Mesh {
    Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("test mesh")
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
        stroke_start_mesh: None,
    };

    let failure = session
        .patch_shadow(&[0], &[])
        .expect_err("a shadow with the wrong shape must be terminal");
    assert_eq!(
        failure,
        DabFailure::ShadowShapeMismatch {
            shadow_count: 0,
            live_count: mesh.vertices().len(),
        }
    );
}

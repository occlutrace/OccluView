#![allow(
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    reason = "exact dab-planning inputs make float equality meaningful"
)]

use super::{
    plan_dab_centers, sculpt_cursor_action, sculpt_cursor_color, sculpt_cursor_height,
    sculpt_target,
};
use crate::sculpt_kernel::BrushMode;
use crate::sculpt_tool::{HOLD_DAB_INTERVAL_SEC, MAX_DABS_PER_FRAME};
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};

#[test]
fn a_cold_mesh_can_be_resolved_for_background_preparation() -> anyhow::Result<()> {
    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::X),
            Vertex::at(Vec3::Y),
        ],
        vec![0, 1, 2],
    )?;
    assert!(!mesh.bvh_is_ready());
    let mut scene = Scene::new();
    let index = scene.add(SceneMesh::new(mesh));
    let layer_id = scene.meshes()[index].id();

    assert_eq!(
        sculpt_target(&scene, Some(layer_id)),
        Some((index, layer_id))
    );
    assert!(!scene.meshes()[index].mesh.bvh_is_ready());
    Ok(())
}

#[test]
fn sculpt_cursor_modes_have_distinct_color_and_body_profiles() {
    let add = sculpt_cursor_color(BrushMode::Add);
    let remove = sculpt_cursor_color(BrushMode::Remove);
    let relax = sculpt_cursor_color(BrushMode::Relax);
    let smooth = sculpt_cursor_color(BrushMode::Smooth);
    assert_ne!(add, remove);
    assert_ne!(add, relax);
    assert_ne!(add, smooth);
    assert_ne!(remove, relax);
    assert_ne!(remove, smooth);
    assert_ne!(relax, smooth);

    assert_eq!(sculpt_cursor_action(BrushMode::Add), [0.0, 0.0]);
    assert_eq!(sculpt_cursor_action(BrushMode::Remove), [1.0, 0.0]);
    assert_eq!(sculpt_cursor_action(BrushMode::Relax), [0.0, 1.0]);
    assert_eq!(sculpt_cursor_action(BrushMode::Smooth), [0.0, 1.0]);
    let add_height = sculpt_cursor_height(BrushMode::Add, 0.5, 2.0);
    let remove_height = sculpt_cursor_height(BrushMode::Remove, 0.5, 2.0);
    let relax_height = sculpt_cursor_height(BrushMode::Relax, 0.5, 2.0);
    let smooth_height = sculpt_cursor_height(BrushMode::Smooth, 0.5, 2.0);
    assert_eq!(add_height, remove_height);
    assert_eq!(relax_height, smooth_height);
    assert!(add_height > relax_height);
}

#[test]
fn first_dab_lands_at_the_cursor_and_arms_the_path() {
    let (centers, last, hold) = plan_dab_centers(None, Vec3::new(2.0, 0.0, 0.0), 1.0, 0.0, 0.016);
    assert_eq!(centers, vec![Vec3::new(2.0, 0.0, 0.0)]);
    assert_eq!(last, Some(Vec3::new(2.0, 0.0, 0.0)));
    assert_eq!(hold, 0.0);
}

#[test]
fn a_straight_move_spaces_dabs_evenly_by_arc_length() {
    let (centers, last, hold) =
        plan_dab_centers(Some(Vec3::ZERO), Vec3::new(3.0, 0.0, 0.0), 1.0, 0.0, 0.016);
    assert_eq!(
        centers,
        vec![
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ]
    );
    assert_eq!(last, Some(Vec3::new(3.0, 0.0, 0.0)));
    assert_eq!(hold, 0.0);
}

#[test]
fn a_huge_single_frame_jump_is_capped_without_backlog() {
    let far = (MAX_DABS_PER_FRAME + 50) as f32;
    let (centers, last, _) =
        plan_dab_centers(Some(Vec3::ZERO), Vec3::new(far, 0.0, 0.0), 1.0, 0.0, 0.016);
    assert_eq!(centers.len(), MAX_DABS_PER_FRAME);
    assert_eq!(last, Some(Vec3::new(far, 0.0, 0.0)));
    assert_eq!(centers.last(), Some(&Vec3::new(far, 0.0, 0.0)));
}

#[test]
fn a_stationary_hold_fires_dabs_on_the_time_cadence() {
    let last_dab = Some(Vec3::ZERO);
    let dt = HOLD_DAB_INTERVAL_SEC * 2.5;
    let (centers, last, hold) =
        plan_dab_centers(last_dab, Vec3::new(0.001, 0.0, 0.0), 1.0, 0.0, dt);
    assert_eq!(centers.len(), 2, "2.5 intervals of hold => 2 dabs");
    assert!(centers.iter().all(|c| *c == Vec3::new(0.001, 0.0, 0.0)));
    assert_eq!(
        last, last_dab,
        "a hold does not advance the arc-length anchor"
    );
    assert!(hold > 0.0 && hold < HOLD_DAB_INTERVAL_SEC);
}

#[test]
fn a_stalled_frame_cannot_dump_a_huge_hold_backlog() {
    let (centers, _, _) =
        plan_dab_centers(Some(Vec3::ZERO), Vec3::new(0.001, 0.0, 0.0), 1.0, 0.0, 5.0);
    assert!(centers.len() <= MAX_DABS_PER_FRAME);
    assert!(
        centers.len() <= 5,
        "clamped dt should keep the backlog small"
    );
}

//! The frames the align path hands the fit.
//!
//! Two frames are in play at once and must not be mixed. A clicked point and
//! its surface normal are stored in the layer's own coordinates; the fixed half
//! is handed to the fit in world coordinates. A normal is a covector, so its
//! world form is the inverse transpose of the instance transform: under a
//! non-uniform scale the direct vector transform points somewhere else, and the
//! two-point fit is solved from those directions.
//!
//! A `#[path]` child module of `app_align.rs`, so it drives the real click and
//! pair-assembly path instead of a copy of it.
#![allow(clippy::expect_used, clippy::float_cmp, clippy::unwrap_used)]

use super::*;
use crate::align_tool::AlignPoint;
use crate::app::app_test_support::{push_named_layer, test_app};
use crate::viewer::pick_scene_hit;
use occluview_core::{Mesh, Vertex};
use std::sync::Arc;

fn press(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }
}

fn release(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn assert_same_direction(actual: Vec3, expected: Vec3, what: &str) {
    let delta = actual.normalize_or_zero() - expected.normalize_or_zero();
    assert!(delta.length() < 1.0e-5, "{what}: got {actual:?}");
}

/// The geometric normal of triangle 0, computed from the mesh's own vertices —
/// the reference the click path has to reproduce, not a value it produced.
fn local_triangle_normal(entry: &SceneMesh) -> Vec3 {
    let indices = entry.mesh.indices();
    let vertices = entry.mesh.vertices();
    let corner = |slot: usize| Vec3::from_array(vertices[indices[slot] as usize].position);
    (corner(1) - corner(0))
        .cross(corner(2) - corner(0))
        .normalize()
}

/// A non-uniformly scaled instance is the case that separates a correct normal
/// transform from a wrong one: 45 degrees across a 2:1 stretch reaches the
/// world as a different direction, not merely a different length.
#[test]
fn fixed_pair_normals_use_the_inverse_transpose_for_scaled_instances() {
    let mut app = test_app("align-fixed-normal-frame");
    let mut scene = Scene::new();
    let moving_id = push_named_layer(&mut scene, "moving", 0.0);
    let fixed_id = push_named_layer(&mut scene, "fixed", 5.0);
    scene.meshes_mut()[1].transform = Affine3A::from_scale(Vec3::new(2.0, 1.0, 1.0));
    app.document.scene = Some(Arc::new(scene));

    app.tools.align.tool.arm();
    app.tools.align.tool.imply_pair(&[moving_id, fixed_id]);
    // Placing the first point on the moving scan settles the roles without a
    // swap, so the pair below is oriented the way it was clicked.
    app.tools.align.tool.click(AlignPoint {
        layer: moving_id,
        local: Vec3::new(1.0, 0.0, 0.0),
        normal: Vec3::Y,
    });
    app.tools.align.tool.click(AlignPoint {
        layer: fixed_id,
        local: Vec3::new(1.0, 1.0, 0.0),
        normal: Vec3::new(1.0, 1.0, 0.0),
    });

    let pairs = app.align_world_pairs();
    assert_eq!(pairs.len(), 1, "two clicks make one pair");
    let fixed_normal = pairs[0].fixed_normal.as_vec3();

    let inverse_transpose = Vec3::new(0.5, 1.0, 0.0).normalize();
    let direct_vector_transform = Vec3::new(2.0, 1.0, 0.0).normalize();
    assert_same_direction(
        fixed_normal,
        inverse_transpose,
        "the fixed normal reaches the fit as the inverse transpose of its instance",
    );
    assert!(
        (fixed_normal - direct_vector_transform).length() > 0.1,
        "a direct vector transform must not pass: got {fixed_normal:?}"
    );
    assert_same_direction(
        pairs[0].moving_normal.as_vec3(),
        Vec3::Y,
        "the moving half stays in its layer's local frame",
    );
}

/// The click stores the hit in the layer's local frame. A transform that is not
/// the identity makes the two frames disagree about the normal's direction, so
/// a click path that inverse-transforms the already-local normal lands on the
/// world direction instead and this test sees it.
#[test]
fn clicked_triangle_normals_stay_in_the_mesh_local_frame() {
    let mut app = test_app("align-click-normal-frame");
    let mesh = Mesh::new(
        Some("surface".to_string()),
        vec![
            Vertex::at(Vec3::new(-10.0, -10.0, 0.0)),
            Vertex::at(Vec3::new(10.0, -10.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 10.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("test mesh");
    // Half a turn about x: local +Z reaches the world as -Z.
    let transform = Affine3A::from_rotation_x(std::f32::consts::PI);
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh).with_transform(transform));
    let layer_id = scene.meshes()[0].id();
    let scene = Arc::new(scene);
    app.document.scene = Some(Arc::clone(&scene));
    app.render.camera = Some(crate::viewer::home_camera_for_scene(scene.as_ref()));
    app.tools.align.tool.arm();

    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
    let pointer = screen.center();
    let mut clicked = None;
    for events in [
        vec![egui::Event::PointerMoved(pointer)],
        vec![egui::Event::PointerMoved(pointer), press(pointer)],
        vec![egui::Event::PointerMoved(pointer), release(pointer)],
    ] {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            let response = ui.allocate_response(ui.available_size(), egui::Sense::click());
            if response.clicked_by(egui::PointerButton::Primary) {
                let consumed = app.handle_align_click(&response, &ctx);
                clicked = Some((
                    consumed,
                    response.rect,
                    response.interact_pointer_pos().expect("a click has a position"),
                ));
            }
        })
        // This test asserts on state, not pixels, so egui's texture deltas are
        // dropped the way `panel_shots` and the other egui-driven tests here do.
        .drop_without_applying_deltas();
    }
    let (consumed, rect, pointer) = clicked.expect("the release frame is a click on the viewport");
    assert!(consumed, "the armed tool consumes the click");

    let pending = app
        .tools
        .align
        .tool
        .pending()
        .expect("the first click places a point");
    assert_eq!(
        pending.layer, layer_id,
        "the point belongs to the clicked layer"
    );

    let local_normal = local_triangle_normal(&scene.meshes()[0]);
    assert_same_direction(
        pending.normal,
        local_normal,
        "the stored normal is the local one",
    );
    let world_normal = transform
        .transform_vector3(local_normal)
        .normalize_or_zero();
    assert!(
        pending.normal.dot(world_normal) < 0.9,
        "the stored normal must not be the world-frame one: {:?} vs {:?}",
        pending.normal,
        world_normal
    );

    let hit = pick_scene_hit(
        app.render.camera.as_ref().expect("camera"),
        rect,
        pointer,
        scene.as_ref(),
    )
    .expect("the same ray hits the surface");
    let expected_local = transform.inverse().transform_point3(hit.point);
    assert!(
        (pending.local - expected_local).length() < 1.0e-3,
        "the stored point is the hit in the layer's frame: {:?} vs {:?}",
        pending.local,
        expected_local
    );
}

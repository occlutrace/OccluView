use super::cursor::{sculpt_cursor_action, sculpt_cursor_color, sculpt_cursor_height};
use super::geometry::sculpt_target;
use super::input::{collect_sculpt_pointer_events, SculptPointerEvent};
use crate::sculpt::sculpt_kernel::BrushMode;
use eframe::egui;
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
fn sculpt_target_never_substitutes_an_unavailable_edit_layer() -> anyhow::Result<()> {
    let mesh = occluview_core::test_support::quad_mesh(None)
        .ok_or_else(|| anyhow::anyhow!("quad mesh"))?;
    let mut scene = Scene::new();
    let edited = scene.add(SceneMesh::new(mesh.clone()));
    let other = scene.add(SceneMesh::new(mesh));
    let edited_id = scene.meshes()[edited].id();
    let other_id = scene.meshes()[other].id();
    scene.meshes_mut()[edited].visible = false;

    assert_eq!(sculpt_target(&scene, Some(edited_id)), None);
    assert_eq!(sculpt_target(&scene, None), Some((other, other_id)));
    scene.remove(edited);
    assert_eq!(sculpt_target(&scene, Some(edited_id)), None);
    Ok(())
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "Operation flags and paired cursor heights are exact values."
)]
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
fn raw_pointer_moves_keep_modifier_changes_in_event_order() {
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: false,
        ..Default::default()
    };
    let events = [
        egui::Event::PointerMoved(egui::pos2(20.0, 30.0)),
        egui::Event::ModifiersChanged(ctrl),
        egui::Event::PointerMoved(egui::pos2(40.0, 50.0)),
        egui::Event::PointerButton {
            pos: egui::pos2(40.0, 50.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: ctrl,
        },
    ];
    let captured = collect_sculpt_pointer_events(&events, egui::Modifiers::NONE, ctrl);

    assert!(matches!(
        captured.as_slice(),
        [
            SculptPointerEvent::Moved(first, first_modifiers),
            SculptPointerEvent::Moved(second, second_modifiers),
            SculptPointerEvent::PrimaryButton(press, true, press_modifiers)
        ] if *first == egui::pos2(20.0, 30.0)
            && *first_modifiers == egui::Modifiers::NONE
            && *second == egui::pos2(40.0, 50.0)
            && *second_modifiers == ctrl
            && *press == egui::pos2(40.0, 50.0)
            && *press_modifiers == ctrl
    ));
}

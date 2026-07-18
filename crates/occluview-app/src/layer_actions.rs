use occluview_core::{Scene, SceneMeshId};

pub(crate) const LAYER_TINT_PRESETS: [([f32; 4], &str); 5] = [
    (occluview_core::DEFAULT_UNTEXTURED_MESH_TINT, "Stone IV"),
    ([0.74, 0.58, 0.32, 1.0], "Baked"),
    ([0.92, 0.80, 0.56, 1.0], "Plaster"),
    ([0.72, 0.75, 0.68, 1.0], "Sage"),
    ([0.82, 0.74, 0.64, 1.0], "Wax"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerContextAction {
    ToggleVisibility,
    Solo,
    ShowAll,
    ResetOpacity,
    NextTint,
    ToggleWireframe,
    ToggleShowVertexColors,
    EditMesh,
    BridgeSplit,
    DeleteSelectedFaces,
    CropToSelectedFaces,
    CutSelectionToNewLayer,
    SeparateSelectedComponents,
    CloseHoles,
    InvertNormals,
    RepairMesh,
    UndoLastMeshEdit,
    ExportLayer,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerContextRequest {
    pub(crate) index: usize,
    pub(crate) layer_id: SceneMeshId,
    pub(crate) action: LayerContextAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LayerContextApply {
    pub(crate) scene_changed: bool,
    pub(crate) structural_scene_change: bool,
    pub(crate) removed: bool,
}

pub(crate) fn apply_layer_context_action(
    scene: &mut Scene,
    request: LayerContextRequest,
) -> LayerContextApply {
    let LayerContextRequest {
        index,
        layer_id,
        action,
    } = request;
    if index >= scene.meshes().len() {
        return LayerContextApply::default();
    }
    if scene.meshes()[index].id() != layer_id {
        return LayerContextApply::default();
    }

    match action {
        LayerContextAction::ToggleVisibility => toggle_layer_visibility(scene, index),
        LayerContextAction::Solo => solo_layer(scene, index),
        LayerContextAction::ShowAll => show_all_layers(scene),
        LayerContextAction::ResetOpacity => reset_layer_opacity(scene, index),
        LayerContextAction::NextTint => advance_layer_tint(scene, index),
        LayerContextAction::ToggleWireframe => toggle_wireframe(scene, index),
        LayerContextAction::ToggleShowVertexColors => toggle_show_vertex_colors(scene, index),
        LayerContextAction::InvertNormals
        | LayerContextAction::EditMesh
        | LayerContextAction::BridgeSplit
        | LayerContextAction::DeleteSelectedFaces
        | LayerContextAction::CropToSelectedFaces
        | LayerContextAction::CutSelectionToNewLayer
        | LayerContextAction::SeparateSelectedComponents
        | LayerContextAction::CloseHoles
        | LayerContextAction::RepairMesh
        | LayerContextAction::UndoLastMeshEdit
        | LayerContextAction::ExportLayer => LayerContextApply::default(),
        LayerContextAction::Remove => {
            let removed = scene.remove(index).is_some();
            LayerContextApply {
                scene_changed: removed,
                structural_scene_change: removed,
                removed,
            }
        }
    }
}

fn toggle_layer_visibility(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    entry.visible = !entry.visible;
    LayerContextApply {
        scene_changed: true,
        ..LayerContextApply::default()
    }
}

fn solo_layer(scene: &mut Scene, index: usize) -> LayerContextApply {
    let mut scene_changed = false;
    for (entry_index, entry) in scene.meshes_mut().iter_mut().enumerate() {
        let next_visible = entry_index == index;
        if entry.visible != next_visible {
            entry.visible = next_visible;
            scene_changed = true;
        }
    }
    LayerContextApply {
        scene_changed,
        ..LayerContextApply::default()
    }
}

fn show_all_layers(scene: &mut Scene) -> LayerContextApply {
    let mut scene_changed = false;
    for entry in scene.meshes_mut() {
        if !entry.visible {
            entry.visible = true;
            scene_changed = true;
        }
    }
    LayerContextApply {
        scene_changed,
        ..LayerContextApply::default()
    }
}

fn reset_layer_opacity(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    if (entry.opacity - 1.0).abs() <= f32::EPSILON {
        return LayerContextApply::default();
    }
    entry.opacity = 1.0;
    LayerContextApply {
        scene_changed: true,
        ..LayerContextApply::default()
    }
}

fn advance_layer_tint(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    entry.tint = next_layer_tint(entry.tint);
    LayerContextApply {
        scene_changed: true,
        ..LayerContextApply::default()
    }
}

fn toggle_wireframe(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    entry.wireframe = !entry.wireframe;
    LayerContextApply {
        scene_changed: true,
        structural_scene_change: true,
        ..LayerContextApply::default()
    }
}

/// Display-only, like [`toggle_layer_visibility`]: it only changes the
/// per-mesh GPU uniform, never mesh topology, so no structural rebuild is
/// needed.
fn toggle_show_vertex_colors(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    entry.show_vertex_colors = !entry.show_vertex_colors;
    LayerContextApply {
        scene_changed: true,
        ..LayerContextApply::default()
    }
}

pub(crate) fn next_layer_tint(current: [f32; 4]) -> [f32; 4] {
    let current_index = LAYER_TINT_PRESETS
        .iter()
        .position(|(color, _)| tint_matches(*color, current))
        .unwrap_or(0);
    let next_index = (current_index + 1) % LAYER_TINT_PRESETS.len();
    LAYER_TINT_PRESETS[next_index].0
}

fn tint_matches(lhs: [f32; 4], rhs: [f32; 4]) -> bool {
    lhs.into_iter()
        .zip(rhs)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{Mesh, SceneMesh};

    fn request(scene: &Scene, index: usize, action: LayerContextAction) -> LayerContextRequest {
        LayerContextRequest {
            index,
            layer_id: scene.meshes()[index].id(),
            action,
        }
    }

    #[test]
    fn layer_context_actions_apply_to_scene_without_rebuilding_meshes() {
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(Mesh::empty()).with_opacity(0.4));
        scene.add(SceneMesh::new(Mesh::empty()));
        scene.add(SceneMesh::new(Mesh::empty()));
        scene.meshes_mut()[2].visible = false;

        let action = request(&scene, 0, LayerContextAction::ToggleVisibility);
        let toggle = apply_layer_context_action(&mut scene, action);
        assert!(toggle.scene_changed);
        assert!(!scene.meshes()[0].visible);

        let action = request(&scene, 0, LayerContextAction::Solo);
        let solo = apply_layer_context_action(&mut scene, action);
        assert!(solo.scene_changed);
        assert!(!solo.structural_scene_change);
        assert!(scene.meshes()[0].visible);
        assert!(!scene.meshes()[1].visible);
        assert!(!scene.meshes()[2].visible);

        let action = request(&scene, 0, LayerContextAction::ShowAll);
        let show_all = apply_layer_context_action(&mut scene, action);
        assert!(show_all.scene_changed);
        assert!(scene.meshes().iter().all(|entry| entry.visible));

        let action = request(&scene, 0, LayerContextAction::ResetOpacity);
        let reset = apply_layer_context_action(&mut scene, action);
        assert!(reset.scene_changed);
        assert!((scene.meshes()[0].opacity - 1.0).abs() <= f32::EPSILON);

        let before_tint = scene.meshes()[1].tint;
        let action = request(&scene, 1, LayerContextAction::NextTint);
        let tint = apply_layer_context_action(&mut scene, action);
        assert!(tint.scene_changed);
        assert!(scene.meshes()[1]
            .tint
            .iter()
            .zip(before_tint.iter())
            .any(|(left, right)| (*left - *right).abs() > f32::EPSILON));

        let action = request(&scene, 0, LayerContextAction::ToggleWireframe);
        let wire = apply_layer_context_action(&mut scene, action);
        assert!(wire.scene_changed);
        assert!(wire.structural_scene_change);
        assert!(scene.meshes()[0].wireframe);

        let action = request(&scene, 0, LayerContextAction::ToggleWireframe);
        let wire_off = apply_layer_context_action(&mut scene, action);
        assert!(wire_off.scene_changed);
        assert!(wire_off.structural_scene_change);
        assert!(!scene.meshes()[0].wireframe);

        assert!(scene.meshes()[0].show_vertex_colors);
        let action = request(&scene, 0, LayerContextAction::ToggleShowVertexColors);
        let colors_off = apply_layer_context_action(&mut scene, action);
        assert!(colors_off.scene_changed);
        assert!(
            !colors_off.structural_scene_change,
            "a display-only toggle must not force a mesh rebuild"
        );
        assert!(!scene.meshes()[0].show_vertex_colors);

        let action = request(&scene, 0, LayerContextAction::ToggleShowVertexColors);
        let colors_on = apply_layer_context_action(&mut scene, action);
        assert!(colors_on.scene_changed);
        assert!(scene.meshes()[0].show_vertex_colors);

        let action = request(&scene, 1, LayerContextAction::Remove);
        let remove = apply_layer_context_action(&mut scene, action);
        assert!(remove.scene_changed);
        assert!(remove.structural_scene_change);
        assert!(remove.removed);
        assert_eq!(scene.meshes().len(), 2);
    }

    #[test]
    fn layer_context_action_ignores_stale_layer_identity() {
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(Mesh::empty()));
        let stale_layer_id = SceneMesh::new(Mesh::empty()).id();

        let apply = apply_layer_context_action(
            &mut scene,
            LayerContextRequest {
                index: 0,
                layer_id: stale_layer_id,
                action: LayerContextAction::ToggleVisibility,
            },
        );

        assert!(!apply.scene_changed);
        assert!(scene.meshes()[0].visible);
    }
}

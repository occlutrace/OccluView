use occluview_core::{Scene, SceneMesh, SceneMeshId};

/// Model tints: the shades a single scan is read in. Muted on purpose — a
/// surface is judged by its shading, and a saturated one hides the detail the
/// shading is carrying.
pub(crate) const LAYER_TINT_PRESETS: [([f32; 4], &str); 10] = [
    (occluview_core::DEFAULT_UNTEXTURED_MESH_TINT, "Stone IV"),
    ([0.74, 0.58, 0.32, 1.0], "Baked"),
    ([0.92, 0.80, 0.56, 1.0], "Plaster"),
    ([0.72, 0.75, 0.68, 1.0], "Sage"),
    ([0.82, 0.74, 0.64, 1.0], "Wax"),
    ([0.55, 0.65, 0.85, 1.0], "Glacier"),
    ([0.85, 0.45, 0.45, 1.0], "Coral"),
    ([0.45, 0.75, 0.55, 1.0], "Mint"),
    ([0.80, 0.65, 0.85, 1.0], "Lilac"),
    ([0.85, 0.75, 0.35, 1.0], "Amber"),
];

/// Overlay tints: for telling two scans apart while they sit on top of each
/// other, which the model shades above cannot do — they are neighbours on the
/// same warm band by design, and one at 50% opacity over another reads as a
/// third shade of the same colour.
///
/// Values are linear sRGB, so they look darker here than they render.
///
/// Ordered as usable pairs, strongest first. Cobalt against Tangerine is the
/// first entry because blue against orange is the one strong opposition that
/// survives red-green colour blindness — roughly one man in twelve — where
/// Crimson against Lime does not. Slate is last and is the odd one out: a
/// neutral for the scan you want to recede behind a coloured one.
pub(crate) const LAYER_OVERLAY_TINT_PRESETS: [([f32; 4], &str); 8] = [
    ([0.03, 0.15, 0.79, 1.0], "Cobalt"),
    ([0.89, 0.24, 0.00, 1.0], "Tangerine"),
    ([0.25, 0.11, 0.87, 1.0], "Violet"),
    ([0.39, 0.64, 0.01, 1.0], "Lime"),
    ([0.01, 0.39, 0.35, 1.0], "Teal"),
    ([0.75, 0.05, 0.39, 1.0], "Magenta"),
    ([0.72, 0.02, 0.06, 1.0], "Crimson"),
    ([0.16, 0.20, 0.26, 1.0], "Slate"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum LayerContextAction {
    ToggleVisibility,
    Solo,
    ShowAll,
    ResetOpacity,
    NextTint,
    ToggleWireframe,
    ToggleShowVertexColors,
    ToggleShowTexture,
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
        LayerContextAction::ToggleShowTexture => toggle_show_texture(scene, index),
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
    if entry.mesh.texture().is_some() {
        entry.show_texture = false;
        entry.show_vertex_colors = false;
    }
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

fn toggle_show_texture(scene: &mut Scene, index: usize) -> LayerContextApply {
    let entry = &mut scene.meshes_mut()[index];
    if entry.mesh.texture().is_none() {
        return LayerContextApply::default();
    }
    entry.show_texture = !entry.show_texture;
    if entry.show_texture {
        entry.show_vertex_colors = true;
    }
    LayerContextApply {
        scene_changed: true,
        ..LayerContextApply::default()
    }
}

/// Every tint the palette offers, model shades first then overlay colours —
/// the order the popup lists them in, and the order cycling walks.
pub(crate) fn all_layer_tints() -> impl Iterator<Item = ([f32; 4], &'static str)> {
    LAYER_TINT_PRESETS
        .into_iter()
        .chain(LAYER_OVERLAY_TINT_PRESETS)
}

/// The next tint after `current`.
///
/// Walks the whole palette, overlay colours included. Cycling used to know
/// only the model shades, so stepping on from an overlay colour found nothing
/// to step on from and dropped back to Stone IV — which made the two halves of
/// the palette behave like different features.
pub(crate) fn next_layer_tint(current: [f32; 4]) -> [f32; 4] {
    let tints: Vec<([f32; 4], &str)> = all_layer_tints().collect();
    let current_index = tints
        .iter()
        .position(|(color, _)| tint_matches(*color, current))
        .unwrap_or(0);
    tints[(current_index + 1) % tints.len()].0
}

/// Whether `tint` is one of the overlay colours.
fn is_overlay_tint(tint: [f32; 4]) -> bool {
    LAYER_OVERLAY_TINT_PRESETS
        .iter()
        .any(|(color, _)| tint_matches(*color, tint))
}

/// Put a tint the operator picked onto `entry`, overriding whatever would stop
/// it being the colour they see.
///
/// The shader multiplies tint into the colour a scan already carries, so on a
/// coloured scan a tint reads as that scan's colour darkened rather than as the
/// colour chosen. A texture has always been overridden for exactly that reason.
/// An overlay colour is picked for one job — telling this scan from the one it
/// is lying on — and a muddied version of it does not do that job, so it takes
/// vertex colour with it. The model shades deliberately do not: they are warm
/// neutrals meant to sit under a scan's own colour, and throwing that colour
/// away would be a surprise rather than a choice. Both are display-only and
/// come back from the layer menu.
pub(crate) fn apply_picked_tint(entry: &mut SceneMesh, tint: [f32; 4]) {
    let picked = !tint_matches(entry.tint, tint);
    if picked {
        if entry.mesh.texture().is_some() {
            entry.show_texture = false;
            entry.show_vertex_colors = false;
        }
        if is_overlay_tint(tint) {
            entry.show_vertex_colors = false;
        }
    }
    entry.tint = tint;
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
    fn an_overlay_colour_wins_over_a_scan_that_carries_its_own_colour() {
        // The whole point of the overlay group is that the scan reads as that
        // one colour. The shader multiplies tint into whatever the scan already
        // carries, so leaving a coloured scan's own colours on would hand back
        // that scan darkened rather than the colour the operator picked.
        let mut coloured = SceneMesh::new(Mesh::empty());
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_OVERLAY_TINT_PRESETS[0].0);

        assert!(tint_matches(coloured.tint, LAYER_OVERLAY_TINT_PRESETS[0].0));
        assert!(
            !coloured.show_vertex_colors,
            "an overlay colour has to be the colour on screen"
        );
    }

    #[test]
    fn a_model_shade_leaves_a_scan_its_own_colour() {
        // The counterpart, and the reason the rule is not "any tint wins": the
        // model shades are warm neutrals meant to sit under a scan's colour.
        // Throwing that colour away would be a surprise, not a choice.
        let mut coloured = SceneMesh::new(Mesh::empty());
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_TINT_PRESETS[2].0);

        assert!(tint_matches(coloured.tint, LAYER_TINT_PRESETS[2].0));
        assert!(
            coloured.show_vertex_colors,
            "a model shade is not a request to discard scan colour"
        );
    }

    #[test]
    fn cycling_the_tint_walks_the_overlay_colours_too() {
        // Cycling used to know only the model shades, so stepping on from an
        // overlay colour found nothing and dropped back to the first entry —
        // which made half the palette unreachable by cycling.
        let last_model = LAYER_TINT_PRESETS[LAYER_TINT_PRESETS.len() - 1].0;
        assert!(tint_matches(
            next_layer_tint(last_model),
            LAYER_OVERLAY_TINT_PRESETS[0].0,
        ));

        let last_overlay = LAYER_OVERLAY_TINT_PRESETS[LAYER_OVERLAY_TINT_PRESETS.len() - 1].0;
        assert!(tint_matches(
            next_layer_tint(last_overlay),
            LAYER_TINT_PRESETS[0].0,
        ));

        // And every colour in the palette is reachable from any starting point.
        let total = LAYER_TINT_PRESETS.len() + LAYER_OVERLAY_TINT_PRESETS.len();
        let mut current = LAYER_TINT_PRESETS[0].0;
        let mut seen = vec![current];
        for _ in 1..total {
            current = next_layer_tint(current);
            assert!(
                !seen.iter().any(|tint| tint_matches(*tint, current)),
                "cycling repeated a colour before covering the palette"
            );
            seen.push(current);
        }
        assert_eq!(seen.len(), total);
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

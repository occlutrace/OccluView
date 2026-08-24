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
/// Values are in the renderer's own space: a tint is multiplied into the
/// shaded colour and nothing encodes on the way out, so what the swatch
/// shows is what the viewport draws.
///
/// Ordered as usable pairs, strongest first. Cobalt against Tangerine is the
/// first entry because blue against orange is the one strong opposition that
/// survives red-green colour blindness — roughly one man in twelve — where
/// Crimson against Lime does not. Slate is last and is the odd one out: a
/// cool near-neutral for the scan meant to recede behind a coloured one —
/// blue-leaning enough that even multiplied into the warm neutral material
/// it still reads cool.
pub(crate) const LAYER_OVERLAY_TINT_PRESETS: [([f32; 4], &str); 8] = [
    ([0.03, 0.15, 0.79, 1.0], "Cobalt"),
    ([0.89, 0.24, 0.00, 1.0], "Tangerine"),
    ([0.25, 0.11, 0.87, 1.0], "Violet"),
    ([0.39, 0.64, 0.01, 1.0], "Lime"),
    ([0.01, 0.39, 0.35, 1.0], "Teal"),
    ([0.75, 0.05, 0.39, 1.0], "Magenta"),
    ([0.72, 0.02, 0.06, 1.0], "Crimson"),
    ([0.10, 0.18, 0.28, 1.0], "Slate"),
];

/// Every action the layer context menu can raise.
///
/// Exhaustive, and held that way by
/// `every_layer_action_is_offered_by_a_surface_that_raises_it`. A variant no
/// menu can produce is a handler nobody can run, a test that proves nothing,
/// and a reader counting features the product does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerContextAction {
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

fn advance_layer_tint(scene: &mut Scene, index: usize) -> LayerContextApply {
    let Some(entry) = scene.meshes_mut().get_mut(index) else {
        return LayerContextApply::default();
    };
    // Through the one gate every picked tint goes through. Cycling used to
    // set the tint directly with its own texture-only override, so stepping
    // into an overlay colour from this path left scan colours multiplying it
    // into mud — the same colour behaving differently depending on which UI
    // path assigned it.
    apply_picked_tint(entry, next_layer_tint(entry.tint), true);
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

/// Display-only: it only changes the per-mesh GPU uniform, never mesh
/// topology, so no structural rebuild is needed.
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

/// Put a tint the operator picked onto `entry`, overriding whatever would
/// stop it being the colour they see.
///
/// `clicked` says the operator chose this tint just now, as opposed to the
/// value merely riding along on an opacity drag or a visibility toggle. The
/// overrides fire on a click even when the value has not changed: re-picking
/// the current overlay colour after re-enabling scan colours is a request to
/// see that colour again, and gating it on the value made that a silent
/// no-op with the swatch still highlighted as current.
///
/// The shader multiplies tint into whatever base the scan shows. On a scan
/// that carries its own colours, an overlay colour times those colours is
/// that scan darkened, so the override switches the colours off. On a scan
/// that carries none, the base is white and tint times white IS the swatch —
/// nothing is switched off, and the common alignment case of two plain STLs
/// renders the overlay colours exactly. The model shades never override
/// colours either way: they are warm neutrals meant to sit under a scan's
/// own colour, and throwing that colour away would be a surprise rather than
/// a choice. Every override is display-only and comes back from the layer
/// menu.
pub(crate) fn apply_picked_tint(entry: &mut SceneMesh, tint: [f32; 4], clicked: bool) {
    let picked = clicked || !tint_matches(entry.tint, tint);
    if picked {
        if entry.mesh.texture().is_some() {
            entry.show_texture = false;
            entry.show_vertex_colors = false;
        }
        if is_overlay_tint(tint) && entry.mesh.carries_color_data() {
            entry.show_vertex_colors = false;
        }
    }
    entry.tint = tint;
}

pub(crate) fn tint_matches(lhs: [f32; 4], rhs: [f32; 4]) -> bool {
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

    /// A one-triangle mesh whose vertices carry a real colour, so
    /// `carries_color_data` is true the way a colour-bearing scan's is.
    fn coloured_mesh() -> Mesh {
        let corner = |position: [f32; 3]| occluview_core::Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            color: [200, 60, 40, 255],
            uv: [0.0, 0.0],
        };
        Mesh::new(
            None,
            vec![
                corner([0.0, 0.0, 0.0]),
                corner([1.0, 0.0, 0.0]),
                corner([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2],
        )
        .unwrap_or_else(|_| Mesh::empty())
    }

    #[test]
    fn an_overlay_colour_on_a_plain_scan_keeps_the_white_base() {
        // A colourless scan's vertices are white, and white times tint IS the
        // swatch — switching colours off there would swap the exact colour
        // for the warm neutral material darkening it. The common alignment
        // case is two plain STLs, so this is the path that matters most.
        let mut plain = SceneMesh::new(Mesh::empty());
        plain.show_vertex_colors = true;

        apply_picked_tint(&mut plain, LAYER_OVERLAY_TINT_PRESETS[0].0, true);

        assert!(
            plain.show_vertex_colors,
            "a white base renders the swatch exactly; there is nothing to override"
        );
    }

    #[test]
    fn re_picking_the_current_overlay_colour_is_still_a_pick() {
        // Pick Cobalt, re-enable scan colours from the menu, pick Cobalt
        // again: the value has not changed, but the click is a request to see
        // that colour again — gating on the value made this a silent no-op
        // with the swatch highlighted as current.
        let mut coloured = SceneMesh::new(coloured_mesh());
        apply_picked_tint(&mut coloured, LAYER_OVERLAY_TINT_PRESETS[0].0, true);
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_OVERLAY_TINT_PRESETS[0].0, true);

        assert!(
            !coloured.show_vertex_colors,
            "the second click must override again"
        );
    }

    #[test]
    fn a_tint_riding_along_on_another_edit_overrides_nothing() {
        // Every row interaction carries the tint value with it; only a swatch
        // CLICK may fire the overrides, or an opacity drag on an overlay-
        // tinted scan would flip its colours off.
        let mut coloured = SceneMesh::new(coloured_mesh());
        coloured.tint = LAYER_OVERLAY_TINT_PRESETS[0].0;
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_OVERLAY_TINT_PRESETS[0].0, false);

        assert!(coloured.show_vertex_colors);
    }

    #[test]
    fn an_overlay_colour_wins_over_a_scan_that_carries_its_own_colour() {
        // The whole point of the overlay group is that the scan reads as that
        // one colour. The shader multiplies tint into whatever the scan already
        // carries, so leaving a coloured scan's own colours on would hand back
        // that scan darkened rather than the colour the operator picked.
        let mut coloured = SceneMesh::new(coloured_mesh());
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_OVERLAY_TINT_PRESETS[0].0, true);

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
        let mut coloured = SceneMesh::new(coloured_mesh());
        coloured.show_vertex_colors = true;

        apply_picked_tint(&mut coloured, LAYER_TINT_PRESETS[2].0, true);

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

    /// Every action the enum declares must be offered by something the
    /// operator can click.
    ///
    /// Four variants once drifted out of reach, each with a handler, a passing
    /// test and in two cases a drawn glyph: a reader counted twenty actions in
    /// a menu that offered sixteen.
    ///
    /// Searching the handler files too would let "has a handler" satisfy "can
    /// be raised", which the drift itself would have passed. Each action names
    /// the surface that offers it instead, so a new variant has to say where it
    /// is raised and deleting a button fails the line that claims it.
    #[test]
    fn every_layer_action_is_offered_by_a_surface_that_raises_it() {
        // Where an action is offered. `MeshEditorPanel` carries the
        // `MeshEditorAction` its button raises, because the panel speaks its
        // own vocabulary and the mapping to a layer action is a separate step.
        enum Surface {
            LayerMenu,
            LayerRow,
            MeshEditorPanel(&'static str),
        }

        let menu = include_str!("layers_overlay/menu.rs");
        let row = include_str!("layers_overlay/row.rs");
        let panel = include_str!("mesh_editor_groups.rs");
        let router = include_str!("app/app_mesh_editor.rs");

        // Crop, cut, separate, delete and close-holes are NOT in the layer
        // menu -- menu.rs asserts their absence -- because they act on a
        // selection only the Mesh Editor can make.
        let surfaces: [(LayerContextAction, &[Surface]); 16] = [
            (LayerContextAction::NextTint, &[Surface::LayerMenu]),
            (LayerContextAction::ToggleWireframe, &[Surface::LayerMenu]),
            (
                LayerContextAction::ToggleShowVertexColors,
                &[Surface::LayerMenu],
            ),
            (LayerContextAction::ToggleShowTexture, &[Surface::LayerMenu]),
            (LayerContextAction::EditMesh, &[Surface::LayerMenu]),
            (LayerContextAction::BridgeSplit, &[Surface::LayerMenu]),
            (
                LayerContextAction::DeleteSelectedFaces,
                &[Surface::MeshEditorPanel("Delete")],
            ),
            (
                LayerContextAction::CropToSelectedFaces,
                &[Surface::MeshEditorPanel("Crop")],
            ),
            (
                LayerContextAction::CutSelectionToNewLayer,
                &[Surface::MeshEditorPanel("Cut")],
            ),
            (
                LayerContextAction::SeparateSelectedComponents,
                &[Surface::MeshEditorPanel("Separate")],
            ),
            (
                LayerContextAction::CloseHoles,
                &[Surface::MeshEditorPanel("CloseHoles")],
            ),
            (LayerContextAction::InvertNormals, &[Surface::LayerMenu]),
            (LayerContextAction::RepairMesh, &[Surface::LayerMenu]),
            (
                LayerContextAction::UndoLastMeshEdit,
                &[Surface::MeshEditorPanel("Undo")],
            ),
            (LayerContextAction::ExportLayer, &[Surface::LayerMenu]),
            (
                LayerContextAction::Remove,
                &[Surface::LayerMenu, Surface::LayerRow],
            ),
        ];

        for (action, offered_by) in surfaces {
            let name = format!("LayerContextAction::{action:?}");
            for surface in offered_by {
                match surface {
                    Surface::LayerMenu => assert!(
                        menu.contains(name.as_str()),
                        "{name} is listed as a layer-menu item and the menu does not raise it"
                    ),
                    Surface::LayerRow => assert!(
                        row.contains(name.as_str()),
                        "{name} is listed as a layer-row control and the row does not raise it"
                    ),
                    Surface::MeshEditorPanel(button) => {
                        let raised = format!("MeshEditorAction::{button}");
                        assert!(
                            panel.contains(raised.as_str()),
                            "{name} is offered by the editor button {raised}, which the \
                             panel no longer draws"
                        );
                        // And routed to THIS action. Checking only that the
                        // button exists let it be re-pointed at a different
                        // layer action, leaving this one offered by nothing
                        // while the test stayed green.
                        let mapping = format!("{raised} => {name}");
                        assert!(
                            router.contains(mapping.as_str())
                                || router.contains(&format!("{raised} => {{")),
                            "{raised} is drawn, but nothing routes it to {name}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn layer_context_action_ignores_stale_layer_identity() {
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(Mesh::empty()).with_opacity(0.4));
        let stale_layer_id = SceneMesh::new(Mesh::empty()).id();
        let before_tint = scene.meshes()[0].tint;

        let apply = apply_layer_context_action(
            &mut scene,
            LayerContextRequest {
                index: 0,
                layer_id: stale_layer_id,
                action: LayerContextAction::NextTint,
            },
        );

        assert!(!apply.scene_changed);
        assert!(
            scene.meshes()[0]
                .tint
                .iter()
                .zip(before_tint.iter())
                .all(|(left, right)| (*left - *right).abs() <= f32::EPSILON),
            "a stale layer id must leave the tint alone"
        );
    }
}

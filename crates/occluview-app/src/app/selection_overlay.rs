use super::{EditModeController, GpuMeshUniform, Mat4, PreparedSceneSource};
use occluview_core::{FaceSelection, Mesh, Scene, SceneMeshId, Vertex};

const SELECTION_OVERLAY_TINT: [f32; 4] = [1.0, 0.58, 0.06, 1.0];
const SELECTION_OVERLAY_OPACITY: f32 = 0.34;
const SELECTION_OVERLAY_VERTEX_COLOR: [u8; 4] = [255, 182, 48, 255];

pub(super) struct SelectionOverlaySource {
    mesh: Mesh,
    uniform: GpuMeshUniform,
}

impl SelectionOverlaySource {
    pub(super) fn source(&self) -> PreparedSceneSource<'_> {
        PreparedSceneSource {
            mesh: &self.mesh,
            uniform: self.uniform,
            visible: true,
            wireframe: true,
        }
    }
}

/// CPU-owned overlay scene. Its sources can be uploaded as one renderer scene
/// when callers have selections for more than the currently active layer.
pub(super) struct SelectionOverlayScene {
    layers: Vec<SelectionOverlaySource>,
}

impl SelectionOverlayScene {
    pub(super) fn prepared_sources(&self) -> Vec<PreparedSceneSource<'_>> {
        self.layers
            .iter()
            .map(SelectionOverlaySource::source)
            .collect()
    }
}

pub(super) fn selection_overlay_for_scene(
    scene: &Scene,
    edit_mode: &EditModeController,
) -> Option<SelectionOverlayScene> {
    let selections = edit_mode
        .visible_selection_plan(scene)
        .into_iter()
        .map(|entry| (entry.layer_id, entry.selection));
    selection_overlay_scene_for_selections(scene, selections)
}

/// Build one CPU overlay scene from all supplied layer selections.
///
/// The function deliberately accepts selections directly instead of reaching
/// into edit-mode state. That keeps it pure and lets the current one-layer API
/// remain unchanged while a future selection API can pass every layer here.
pub(super) fn selection_overlay_scene_for_selections<I>(
    scene: &Scene,
    selections: I,
) -> Option<SelectionOverlayScene>
where
    I: IntoIterator<Item = (SceneMeshId, FaceSelection)>,
{
    let layers = selections
        .into_iter()
        .filter_map(|(layer_id, selection)| {
            selection_overlay_for_layer(scene, layer_id, &selection)
        })
        .collect::<Vec<_>>();
    (!layers.is_empty()).then_some(SelectionOverlayScene { layers })
}

fn selection_overlay_for_layer(
    scene: &Scene,
    layer_id: SceneMeshId,
    selection: &FaceSelection,
) -> Option<SelectionOverlaySource> {
    if selection.selected_count() == 0 {
        return None;
    }

    let entry = scene.meshes().iter().find(|entry| entry.id() == layer_id)?;
    if !entry.visible || entry.mesh.is_point_cloud() {
        return None;
    }
    if selection.len() != entry.mesh.triangle_count() {
        return None;
    }

    let mut vertices = Vec::with_capacity(selection.selected_count() * 3);
    let mut indices = Vec::with_capacity(selection.selected_count() * 3);
    for (triangle_index, triangle) in entry.mesh.indices().chunks_exact(3).enumerate() {
        if !selection.as_slice()[triangle_index] {
            continue;
        }
        for raw_index in triangle {
            let vertex_index = usize::try_from(*raw_index).ok()?;
            let vertex = *entry.mesh.vertices().get(vertex_index)?;
            vertices.push(selection_overlay_vertex(vertex));
            indices.push(u32::try_from(indices.len()).ok()?);
        }
    }
    if indices.is_empty() {
        return None;
    }

    let mesh = Mesh::new(Some("selection overlay".to_string()), vertices, indices).ok()?;
    Some(SelectionOverlaySource {
        mesh,
        uniform: GpuMeshUniform {
            model: Mat4::from(entry.transform).to_cols_array(),
            tint: SELECTION_OVERLAY_TINT,
            opacity: SELECTION_OVERLAY_OPACITY,
            has_texture: 0,
            show_orientation: 0,
            pad: 0,
        },
    })
}

fn selection_overlay_vertex(vertex: Vertex) -> Vertex {
    vertex.with_color(SELECTION_OVERLAY_VERTEX_COLOR)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

    use super::*;
    use crate::edit_mode::EditModeController;
    use glam::{Affine3A, Vec3};
    use occluview_core::{FaceSelection, Mesh, SceneMesh, ScenePickHit};

    fn vertex(x: f32, y: f32, z: f32) -> Vertex {
        Vertex::at(Vec3::new(x, y, z)).with_normal(Vec3::Z)
    }

    fn two_triangle_scene() -> Scene {
        let mesh = Mesh::new(
            Some("scan".into()),
            vec![
                vertex(0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(0.0, 1.0, 0.0),
                vertex(2.0, 0.0, 0.0),
                vertex(3.0, 0.0, 0.0),
                vertex(2.0, 1.0, 0.0),
            ],
            vec![0, 1, 2, 3, 4, 5],
        )
        .expect("valid mesh");
        let mut scene = Scene::new();
        scene.add(
            SceneMesh::new(mesh)
                .with_transform(Affine3A::from_translation(Vec3::new(4.0, 5.0, 6.0))),
        );
        scene
    }

    #[test]
    fn selection_overlay_source_contains_only_selected_faces() {
        let scene = two_triangle_scene();
        let layer_id = scene.meshes()[0].id();
        let mut edit_mode = EditModeController::new(4, 1_000_000);
        assert!(edit_mode.select_face_hit(
            &scene,
            ScenePickHit {
                layer_index: 0,
                layer_id,
                triangle_index: 1,
                point: Vec3::new(2.25, 0.25, 0.0),
                distance: 10.0,
            },
        ));

        let Some(overlay) = selection_overlay_for_scene(&scene, &edit_mode) else {
            panic!("selected face should create an overlay");
        };

        assert_eq!(overlay.layers.len(), 1);
        let overlay = &overlay.layers[0];
        assert_eq!(overlay.mesh.vertices().len(), 3);
        assert_eq!(overlay.mesh.indices(), &[0, 1, 2]);
        assert_eq!(overlay.mesh.vertices()[0].position, [2.0, 0.0, 0.0]);
        assert_eq!(
            overlay.uniform.model,
            Mat4::from(scene.meshes()[0].transform).to_cols_array()
        );
        assert!(overlay.uniform.opacity > 0.0 && overlay.uniform.opacity < 1.0);
        assert!(overlay.source().visible);
        assert!(overlay.source().wireframe);
    }

    #[test]
    fn selection_overlay_source_ignores_hidden_or_stale_selection() {
        let mut scene = two_triangle_scene();
        let layer_id = scene.meshes()[0].id();
        let mut edit_mode = EditModeController::new(4, 1_000_000);
        assert!(edit_mode.select_face_hit(
            &scene,
            ScenePickHit {
                layer_index: 0,
                layer_id,
                triangle_index: 0,
                point: Vec3::new(0.25, 0.25, 0.0),
                distance: 10.0,
            },
        ));
        scene.meshes_mut()[0].visible = false;

        assert!(selection_overlay_for_scene(&scene, &edit_mode).is_none());
    }

    #[test]
    fn builder_can_prepare_multiple_selection_layers_as_one_scene() {
        let mesh = Mesh::new(
            Some("scan".into()),
            vec![
                vertex(0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2],
        )
        .expect("valid mesh");
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh.clone()));
        scene.add(SceneMesh::new(mesh).with_transform(Affine3A::from_translation(Vec3::X)));
        let selections = scene
            .meshes()
            .iter()
            .map(|entry| (entry.id(), FaceSelection::new(vec![true])));

        let overlay = selection_overlay_scene_for_selections(&scene, selections)
            .expect("selected layers should create an overlay scene");

        let sources = overlay.prepared_sources();
        assert_eq!(sources.len(), 2);
        assert!(sources
            .iter()
            .all(|source| source.visible && source.wireframe));
    }
}

//! Canonical face selections for all scene layers.

use std::collections::HashMap;

use occluview_core::{Camera, FaceSelection, Scene, SceneMesh, SceneMeshId, ScenePickHit};

use super::selection::{FaceSelectionState, ScreenPolygonSelectionRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SelectionKey {
    layer_id: SceneMeshId,
    topology_id: u64,
}

/// An owned operation input for one visible, non-empty layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisibleFaceSelection {
    pub(crate) layer_id: SceneMeshId,
    pub(crate) topology_id: u64,
    pub(crate) selection: FaceSelection,
}

#[derive(Default)]
pub(super) struct FaceSelectionSet {
    entries: HashMap<SelectionKey, FaceSelectionState>,
}

impl FaceSelectionSet {
    pub(super) fn sync_to_scene(&mut self, scene: &Scene) {
        self.entries.retain(|key, selection| {
            scene.meshes().iter().any(|entry| {
                entry.id() == key.layer_id
                    && entry.mesh.topology_id() == key.topology_id
                    && entry.mesh.triangle_count() == selection.triangle_count()
            })
        });
    }

    pub(super) fn select_face_hit(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> bool {
        let Some(entry) = selectable_entry(scene, hit.layer_index, hit.layer_id) else {
            return false;
        };
        if hit.triangle_index >= entry.mesh.triangle_count() {
            return false;
        }
        let Some(selection) = self.ensure_for_entry(entry) else {
            return false;
        };
        selection.select_scene_hit(scene, hit, unmark).is_some()
    }

    pub(super) fn select_component_hit(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> bool {
        let Some(entry) = selectable_entry(scene, hit.layer_index, hit.layer_id) else {
            return false;
        };
        if hit.triangle_index >= entry.mesh.triangle_count() {
            return false;
        }
        let Some(selection) = self.ensure_for_entry(entry) else {
            return false;
        };
        selection
            .select_scene_hit_component(scene, hit, unmark)
            .is_some()
    }

    pub(super) fn select_screen_polygon(
        &mut self,
        scene: &Scene,
        camera: &Camera,
        request: ScreenPolygonSelectionRequest<'_>,
    ) -> bool {
        let mut changed = false;
        for entry in scene.meshes() {
            if !is_selectable(entry) {
                continue;
            }
            let Some(selection) = self.ensure_for_entry(entry) else {
                continue;
            };
            changed |= selection
                .select_screen_polygon(scene, camera, request.clone())
                .unwrap_or(false);
        }
        changed
    }

    pub(super) fn clear_visible(&mut self, scene: &Scene) -> bool {
        let mut changed = false;
        for entry in scene.meshes().iter().filter(|entry| is_selectable(entry)) {
            if let Some(selection) = self.selection_for_entry_mut(entry) {
                changed |= selection.clear_selection();
            }
        }
        changed
    }

    #[cfg(test)]
    pub(super) fn clear_layer(&mut self, layer_id: Option<SceneMeshId>) -> bool {
        layer_id
            .and_then(|layer_id| {
                self.entries
                    .iter_mut()
                    .find(|(key, _)| key.layer_id == layer_id)
                    .map(|(_, selection)| selection.clear_selection())
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(super) fn select_all_layer(&mut self, layer_id: Option<SceneMeshId>) -> bool {
        layer_id
            .and_then(|layer_id| {
                self.entries
                    .iter_mut()
                    .find(|(key, _)| key.layer_id == layer_id)
                    .map(|(_, selection)| selection.select_all())
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(super) fn invert_layer(&mut self, layer_id: Option<SceneMeshId>) -> bool {
        layer_id
            .and_then(|layer_id| {
                self.entries
                    .iter_mut()
                    .find(|(key, _)| key.layer_id == layer_id)
                    .map(|(_, selection)| selection.invert_selection())
            })
            .unwrap_or(false)
    }

    pub(super) fn select_all_visible(&mut self, scene: &Scene) -> bool {
        let mut changed = false;
        for entry in scene.meshes().iter().filter(|entry| is_selectable(entry)) {
            let Some(selection) = self.ensure_for_entry(entry) else {
                continue;
            };
            changed |= selection.select_all();
        }
        changed
    }

    pub(super) fn invert_visible(&mut self, scene: &Scene) -> bool {
        let mut changed = false;
        for entry in scene.meshes().iter().filter(|entry| is_selectable(entry)) {
            let Some(selection) = self.ensure_for_entry(entry) else {
                continue;
            };
            changed |= selection.invert_selection();
        }
        changed
    }

    #[cfg(test)]
    pub(super) fn total_selected_face_count(&self) -> usize {
        self.entries
            .values()
            .map(FaceSelectionState::selected_count)
            .sum()
    }

    #[cfg(test)]
    pub(super) fn total_selected_layer_count(&self) -> usize {
        self.entries
            .values()
            .filter(|selection| selection.selected_count() > 0)
            .count()
    }

    pub(super) fn visible_selections<'a>(
        &'a self,
        scene: &'a Scene,
    ) -> impl Iterator<Item = &'a FaceSelectionState> + 'a {
        self.entries.iter().filter_map(move |(key, selection)| {
            (selection.selected_count() > 0
                && scene.meshes().iter().any(|entry| {
                    is_selectable(entry)
                        && entry.id() == key.layer_id
                        && entry.mesh.topology_id() == key.topology_id
                        && entry.mesh.triangle_count() == selection.triangle_count()
                }))
            .then_some(selection)
        })
    }

    pub(super) fn visible_selected_face_count(&self, scene: &Scene) -> usize {
        self.visible_selections(scene)
            .map(FaceSelectionState::selected_count)
            .sum()
    }

    pub(super) fn visible_selected_layer_count(&self, scene: &Scene) -> usize {
        self.visible_selections(scene).count()
    }

    pub(super) fn visible_selection_plan(&self, scene: &Scene) -> Vec<VisibleFaceSelection> {
        scene
            .meshes()
            .iter()
            .filter(|entry| is_selectable(entry))
            .filter_map(|entry| {
                let key = SelectionKey {
                    layer_id: entry.id(),
                    topology_id: entry.mesh.topology_id(),
                };
                let selection = self.entries.get(&key)?;
                (selection.selected_count() > 0
                    && entry.mesh.triangle_count() == selection.triangle_count())
                .then(|| VisibleFaceSelection {
                    layer_id: key.layer_id,
                    topology_id: key.topology_id,
                    selection: selection.to_face_selection(),
                })
            })
            .collect()
    }

    pub(super) fn selection_for_layer(&self, layer_id: SceneMeshId) -> Option<FaceSelection> {
        self.entries
            .iter()
            .find(|(key, _)| key.layer_id == layer_id)
            .map(|(_, selection)| selection.to_face_selection())
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn ensure_for_entry(
        &mut self,
        entry: &SceneMesh,
    ) -> Option<&mut FaceSelectionState> {
        if !is_selectable(entry) {
            return None;
        }
        let key = SelectionKey {
            layer_id: entry.id(),
            topology_id: entry.mesh.topology_id(),
        };
        self.entries
            .retain(|existing, _| existing.layer_id != key.layer_id || *existing == key);
        match self.entries.entry(key) {
            std::collections::hash_map::Entry::Occupied(selection) => Some(selection.into_mut()),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let selection =
                    FaceSelectionState::empty_for_layer(entry.id(), entry.mesh.triangle_count())?;
                Some(slot.insert(selection))
            }
        }
    }

    fn selection_for_entry_mut(&mut self, entry: &SceneMesh) -> Option<&mut FaceSelectionState> {
        let key = SelectionKey {
            layer_id: entry.id(),
            topology_id: entry.mesh.topology_id(),
        };
        self.entries.get_mut(&key)
    }
}

fn is_selectable(entry: &SceneMesh) -> bool {
    entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
}

fn selectable_entry(
    scene: &Scene,
    layer_index: usize,
    layer_id: SceneMeshId,
) -> Option<&SceneMesh> {
    scene
        .meshes()
        .get(layer_index)
        .filter(|entry| entry.id() == layer_id && is_selectable(entry))
}

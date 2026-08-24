//! Face-selection commands.

use occluview_core::{Camera, Scene, SceneMeshId, ScenePickHit};

use super::selection::ScreenPolygonSelectionRequest;
use super::state::EditModeState;
use super::EditModeController;

impl EditModeController {
    #[cfg(test)]
    pub(crate) fn select_face_hit(&mut self, scene: &Scene, hit: ScenePickHit) -> bool {
        self.select_face_hit_with_mode(scene, hit, false)
    }

    pub(crate) fn select_face_hit_with_mode(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> bool {
        if matches!(self.state, EditModeState::Busy { .. }) {
            return false;
        }
        let changed = self.selections.select_face_hit(scene, hit, unmark);
        if changed {
            self.active_layer_id = Some(hit.layer_id);
        }
        changed
    }

    pub(crate) fn select_component_hit(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> bool {
        if matches!(self.state, EditModeState::Busy { .. }) {
            return false;
        }
        let changed = self.selections.select_component_hit(scene, hit, unmark);
        if changed {
            self.active_layer_id = Some(hit.layer_id);
        }
        changed
    }

    /// Most recently selected layer.
    pub(crate) fn selected_layer_id(&self) -> Option<SceneMeshId> {
        self.active_layer_id
    }

    /// Selected face count for the active layer.
    #[cfg(test)]
    pub(crate) fn selected_face_count(&self) -> usize {
        self.active_layer_id
            .and_then(|layer_id| self.selections.selection_for_layer(layer_id))
            .map_or(0, |selection| selection.selected_count())
    }

    pub(crate) fn selected_faces_for_layer(
        &self,
        layer_id: SceneMeshId,
    ) -> Option<occluview_core::FaceSelection> {
        self.selections.selection_for_layer(layer_id)
    }

    /// Clear the active layer selection.
    #[cfg(test)]
    pub(crate) fn clear_face_selection(&mut self) -> bool {
        self.selections.clear_layer(self.active_layer_id)
    }

    #[cfg(test)]
    pub(crate) fn select_all_faces(&mut self) -> bool {
        self.selections.select_all_layer(self.active_layer_id)
    }

    #[cfg(test)]
    pub(crate) fn invert_face_selection(&mut self) -> bool {
        self.selections.invert_layer(self.active_layer_id)
    }

    pub(crate) fn clear_visible_selections(&mut self, scene: &Scene) -> bool {
        self.selections.clear_visible(scene)
    }

    pub(crate) fn select_all_visible_selections(&mut self, scene: &Scene) -> bool {
        self.selections.select_all_visible(scene)
    }

    pub(crate) fn invert_visible_selections(&mut self, scene: &Scene) -> bool {
        self.selections.invert_visible(scene)
    }

    #[cfg(test)]
    pub(crate) fn total_selected_face_count(&self) -> usize {
        self.selections.total_selected_face_count()
    }

    #[cfg(test)]
    pub(crate) fn total_selected_layer_count(&self) -> usize {
        self.selections.total_selected_layer_count()
    }

    #[cfg(test)]
    pub(crate) fn visible_selections<'a>(
        &'a self,
        scene: &'a Scene,
    ) -> impl Iterator<Item = &'a super::selection::FaceSelectionState> + 'a {
        self.selections.visible_selections(scene)
    }

    pub(crate) fn visible_selected_face_count(&self, scene: &Scene) -> usize {
        self.selections.visible_selected_face_count(scene)
    }

    pub(crate) fn visible_selected_layer_count(&self, scene: &Scene) -> usize {
        self.selections.visible_selected_layer_count(scene)
    }

    pub(crate) fn visible_selection_plan(
        &self,
        scene: &Scene,
    ) -> Vec<super::selection_set::VisibleFaceSelection> {
        self.selections.visible_selection_plan(scene)
    }

    /// Apply one lasso/marquee polygon to every visible, editable mesh layer.
    pub(crate) fn select_faces_in_screen_polygon(
        &mut self,
        scene: &Scene,
        camera: &Camera,
        request: ScreenPolygonSelectionRequest<'_>,
    ) -> bool {
        if matches!(self.state, EditModeState::Busy { .. }) {
            return false;
        }
        self.selections
            .select_screen_polygon(scene, camera, request)
    }
}

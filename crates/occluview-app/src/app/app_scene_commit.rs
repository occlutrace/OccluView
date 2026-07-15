use super::{egui, OccluViewApp, PathBuf, Scene};
use std::collections::{BTreeSet, HashMap};

pub(super) fn reconcile_scene_paths(
    old_scene: &Scene,
    old_paths: &[PathBuf],
    new_scene: &Scene,
) -> Vec<PathBuf> {
    let mut paths_by_id = HashMap::with_capacity(old_scene.meshes().len());
    for (index, entry) in old_scene.meshes().iter().enumerate() {
        paths_by_id.insert(
            entry.id(),
            old_paths.get(index).cloned().unwrap_or_default(),
        );
    }

    new_scene
        .meshes()
        .iter()
        .map(|entry| paths_by_id.get(&entry.id()).cloned().unwrap_or_default())
        .collect()
}

impl OccluViewApp {
    fn retain_unsaved_edit_layer_ids(&mut self, scene: &Scene) {
        let retained_ids: BTreeSet<_> = scene
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();
        self.unsaved_edit_layer_ids
            .retain(|id| retained_ids.contains(id));
        self.has_unsaved_mesh_edits = !self.unsaved_edit_layer_ids.is_empty();
    }

    pub(super) fn commit_structural_scene(
        &mut self,
        previous_scene: Option<&Scene>,
        draft: Scene,
        ctx: &egui::Context,
    ) {
        if draft.meshes().is_empty() {
            self.clear_scene();
            ctx.request_repaint();
            return;
        }

        let reconciled_paths = previous_scene.map_or_else(
            || vec![PathBuf::new(); draft.meshes().len()],
            |scene| reconcile_scene_paths(scene, &self.current_paths, &draft),
        );
        self.retain_unsaved_edit_layer_ids(&draft);
        self.current_paths = reconciled_paths;
        self.set_scene(draft, false);
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use occluview_core::{Mesh, SceneMesh, Vertex};

    fn v(x: f32, y: f32, z: f32) -> Vertex {
        Vertex::at(Vec3::new(x, y, z))
    }

    fn named_layer(name: &str) -> Option<SceneMesh> {
        let mesh = Mesh::new(
            Some(name.to_string()),
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        )
        .ok()?;
        Some(SceneMesh::new(mesh))
    }

    fn scene_with_layers(layers: impl IntoIterator<Item = SceneMesh>) -> Scene {
        let mut scene = Scene::new();
        for layer in layers {
            scene.add(layer);
        }
        scene
    }

    #[test]
    fn inserting_part_in_middle_preserves_retained_ids_and_gives_new_id_empty_path() {
        let Some(lower) = named_layer("Lower") else {
            return;
        };
        let Some(upper) = named_layer("Upper") else {
            return;
        };
        let Some(prep) = named_layer("Prep") else {
            return;
        };
        let Some(part_b) = named_layer("Part B") else {
            return;
        };
        let old_scene = scene_with_layers([lower.clone(), upper.clone(), prep.clone()]);
        let new_scene = scene_with_layers([lower, part_b, upper, prep]);
        let old_paths = vec![
            PathBuf::from("/cases/lower.stl"),
            PathBuf::from("/cases/upper.stl"),
            PathBuf::from("/cases/prep.stl"),
        ];

        let reconciled = reconcile_scene_paths(&old_scene, &old_paths, &new_scene);

        assert_eq!(
            reconciled,
            vec![
                PathBuf::from("/cases/lower.stl"),
                PathBuf::new(),
                PathBuf::from("/cases/upper.stl"),
                PathBuf::from("/cases/prep.stl"),
            ]
        );
    }

    #[test]
    fn removing_and_reordering_layers_maps_paths_by_stable_id() {
        let Some(lower) = named_layer("Lower") else {
            return;
        };
        let Some(upper) = named_layer("Upper") else {
            return;
        };
        let Some(prep) = named_layer("Prep") else {
            return;
        };
        let old_scene = scene_with_layers([lower.clone(), upper, prep.clone()]);
        let new_scene = scene_with_layers([prep, lower]);
        let old_paths = vec![
            PathBuf::from("/cases/lower.stl"),
            PathBuf::from("/cases/upper.stl"),
            PathBuf::from("/cases/prep.stl"),
        ];

        let reconciled = reconcile_scene_paths(&old_scene, &old_paths, &new_scene);

        assert_eq!(
            reconciled,
            vec![
                PathBuf::from("/cases/prep.stl"),
                PathBuf::from("/cases/lower.stl"),
            ]
        );
    }

    #[test]
    fn structural_undo_and_redo_share_the_same_reconciliation_helper() {
        let Some(lower) = named_layer("Lower") else {
            return;
        };
        let Some(split_part) = named_layer("Split Part") else {
            return;
        };
        let baseline = scene_with_layers([lower.clone()]);
        let split = scene_with_layers([lower, split_part]);
        let baseline_paths = vec![PathBuf::from("/cases/lower.stl")];

        let redo_paths = reconcile_scene_paths(&baseline, &baseline_paths, &split);
        let undo_paths = reconcile_scene_paths(&split, &redo_paths, &baseline);
        let redo_again_paths = reconcile_scene_paths(&baseline, &undo_paths, &split);

        assert_eq!(
            redo_paths,
            vec![PathBuf::from("/cases/lower.stl"), PathBuf::new()]
        );
        assert_eq!(undo_paths, vec![PathBuf::from("/cases/lower.stl")]);
        assert_eq!(redo_again_paths, redo_paths);
    }

    #[test]
    fn structural_scene_callers_route_through_the_shared_commit_helper() {
        let interaction = include_str!("app_layer_interaction.rs").replace("\r\n", "\n");
        let mesh_editor = include_str!("app_mesh_editor.rs").replace("\r\n", "\n");
        let layer_edits = include_str!("app_layer_edits/mod.rs").replace("\r\n", "\n");

        assert!(
            interaction.contains("self.commit_structural_scene("),
            "layer-context structural scene commits should use the shared helper"
        );
        assert!(
            mesh_editor.contains("self.commit_structural_scene("),
            "mesh-editor structural commits should use the shared helper"
        );
        assert!(
            !layer_edits.contains("current_paths.remove("),
            "manual index-based path mutation should be gone"
        );
    }
}

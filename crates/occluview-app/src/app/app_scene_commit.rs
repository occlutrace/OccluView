use super::{egui, OccluViewApp, PathBuf, Scene};
use std::collections::{BTreeSet, HashMap};

pub(super) fn reconcile_scene_paths(
    old_scene: &Scene,
    old_paths: &[PathBuf],
    new_scene: &Scene,
) -> Vec<PathBuf> {
    let mut paths_by_id = HashMap::with_capacity(old_scene.meshes().len());
    // Keep paths by lineage root so descendants retain the source file after
    // the original layer leaves the scene.
    let mut paths_by_origin = HashMap::with_capacity(old_scene.meshes().len());
    for (index, entry) in old_scene.meshes().iter().enumerate() {
        let path = old_paths.get(index).cloned().unwrap_or_default();
        paths_by_id.insert(entry.id(), path.clone());
        if !path.as_os_str().is_empty() {
            paths_by_origin
                .entry(entry.export_source_layer_id())
                .or_insert(path);
        }
    }

    new_scene
        .meshes()
        .iter()
        .map(|entry| {
            let origin = entry.export_source_layer_id();
            // An empty slot must not hide a later lineage path.
            [entry.id(), origin]
                .into_iter()
                .find_map(|id| {
                    paths_by_id
                        .get(&id)
                        .filter(|path| !path.as_os_str().is_empty())
                })
                .or_else(|| {
                    [entry.id(), origin].into_iter().find_map(|id| {
                        paths_by_origin
                            .get(&id)
                            .filter(|path| !path.as_os_str().is_empty())
                    })
                })
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

impl OccluViewApp {
    fn retain_unsaved_edit_layer_ids(&mut self, scene: &Scene) {
        let retained_ids: BTreeSet<_> = scene
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();
        self.document
            .unsaved_edit_layer_ids
            .retain(|id| retained_ids.contains(id));
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
            |scene| reconcile_scene_paths(scene, &self.persistence.current_paths, &draft),
        );
        self.retain_unsaved_edit_layer_ids(&draft);
        self.persistence.current_paths = reconciled_paths;
        self.set_scene(draft, false);
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use glam::Vec3;
    use occluview_core::{Mesh, SceneMesh, Vertex};

    fn v(x: f32, y: f32, z: f32) -> Vertex {
        Vertex::at(Vec3::new(x, y, z))
    }

    /// Construct the fixed test mesh or fail the test.
    fn named_layer(name: &str) -> SceneMesh {
        let mesh = Mesh::new(
            Some(name.to_string()),
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        )
        .expect("the fixture's fixed triangle list is a valid mesh");
        SceneMesh::new(mesh)
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
        let lower = named_layer("Lower");
        let upper = named_layer("Upper");
        let prep = named_layer("Prep");
        let part_b = named_layer("Part B");
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
        let lower = named_layer("Lower");
        let upper = named_layer("Upper");
        let prep = named_layer("Prep");
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
        let lower = named_layer("Lower");
        let split_part = named_layer("Split Part");
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
    fn derived_layer_keeps_the_source_export_path() {
        let source = named_layer("Source");
        let derived = named_layer("Source part").with_source_layer_id(source.id());
        let old_scene = scene_with_layers([source.clone()]);
        let new_scene = scene_with_layers([source, derived]);
        let old_paths = vec![PathBuf::from("/cases/source.stl")];

        assert_eq!(
            reconcile_scene_paths(&old_scene, &old_paths, &new_scene),
            vec![
                PathBuf::from("/cases/source.stl"),
                PathBuf::from("/cases/source.stl"),
            ]
        );
    }

    /// An empty path on a live layer must not hide its lineage path.
    #[test]
    fn an_empty_slot_falls_through_to_the_lineage_root() {
        let source = named_layer("Source");
        let derived = named_layer("Source part").with_source_layer_id(source.id());
        let old_scene = scene_with_layers([source]);
        let new_scene = scene_with_layers([derived]);
        let old_paths = vec![PathBuf::from("/cases/source.stl")];

        assert_eq!(
            reconcile_scene_paths(&old_scene, &old_paths, &new_scene),
            vec![PathBuf::from("/cases/source.stl")],
            "the part must reach its ancestor's file through the origin map"
        );
    }

    /// A layer that never had a file keeps an empty path: no invented source.
    #[test]
    fn a_layer_without_any_lineage_file_stays_empty() {
        let generated = named_layer("Generated");
        let old_scene = scene_with_layers([]);
        let new_scene = scene_with_layers([generated]);

        assert_eq!(
            reconcile_scene_paths(&old_scene, &[], &new_scene),
            vec![PathBuf::new()],
            "a layer that came from nowhere must not borrow someone else's file"
        );
    }
}

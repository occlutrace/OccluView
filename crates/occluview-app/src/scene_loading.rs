use anyhow::Result;
use occluview_core::Scene;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneLoadMode {
    Replace,
    Append,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadQueueCameraReset {
    Idle,
    WhenQueueDrains,
}

pub(crate) struct SceneLoadRequest {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) source: &'static str,
    pub(crate) mode: SceneLoadMode,
}

pub(crate) struct PendingSceneLoad {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) source: &'static str,
    pub(crate) mode: SceneLoadMode,
    pub(crate) started_at: Instant,
    pub(crate) receiver: Receiver<Result<Scene>>,
}

pub(crate) fn combine_loaded_scene(
    existing_scene: Option<&Scene>,
    existing_paths: &[PathBuf],
    loaded_scene: Scene,
    loaded_paths: &[PathBuf],
) -> (Scene, Vec<PathBuf>) {
    let Some(existing_scene) = existing_scene else {
        return (loaded_scene, loaded_paths.to_vec());
    };

    let mut combined_scene = existing_scene.clone();
    combined_scene.append_scene(loaded_scene);

    let mut combined_paths = existing_paths.to_vec();
    combined_paths.extend_from_slice(loaded_paths);

    (combined_scene, combined_paths)
}

pub(crate) fn load_status_message(mode: SceneLoadMode, path_count: usize) -> String {
    let noun = if path_count == 1 { "file" } else { "files" };
    match mode {
        SceneLoadMode::Replace => format!("Opening {path_count} {noun}..."),
        SceneLoadMode::Append => format!("Adding {path_count} {noun}..."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{Mesh, SceneMesh};

    #[test]
    fn combine_loaded_scene_appends_layers_and_paths() {
        let mut existing = Scene::new();
        existing.add(SceneMesh::new(Mesh::empty()));

        let mut loaded = Scene::new();
        loaded.add(SceneMesh::new(Mesh::empty()));
        loaded.add(SceneMesh::new(Mesh::empty()));

        let existing_paths = vec![PathBuf::from(r"C:\cases\upper.stl")];
        let loaded_paths = vec![
            PathBuf::from(r"C:\cases\lower.ply"),
            PathBuf::from(r"C:\cases\bite.glb"),
        ];

        let (combined, paths) =
            combine_loaded_scene(Some(&existing), &existing_paths, loaded, &loaded_paths);

        assert_eq!(combined.meshes().len(), 3);
        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"C:\cases\upper.stl"),
                PathBuf::from(r"C:\cases\lower.ply"),
                PathBuf::from(r"C:\cases\bite.glb"),
            ]
        );
    }

    #[test]
    fn combine_loaded_scene_without_existing_scene_returns_loaded_scene() {
        let mut loaded = Scene::new();
        loaded.add(SceneMesh::new(Mesh::empty()));
        let loaded_paths = vec![PathBuf::from(r"C:\cases\upper.stl")];

        let (combined, paths) = combine_loaded_scene(None, &[], loaded, &loaded_paths);

        assert_eq!(combined.meshes().len(), 1);
        assert_eq!(paths, loaded_paths);
    }

    #[test]
    fn load_status_message_matches_mode_and_count() {
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 1),
            "Opening 1 file..."
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 2),
            "Opening 2 files..."
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Append, 1),
            "Adding 1 file..."
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Append, 3),
            "Adding 3 files..."
        );
    }
}

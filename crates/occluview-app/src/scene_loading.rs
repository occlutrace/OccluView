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

pub(crate) fn load_status_message(
    mode: SceneLoadMode,
    path_count: usize,
    locale: &crate::i18n::LocaleManager,
) -> String {
    let key = match mode {
        SceneLoadMode::Replace => "load-opening",
        SceneLoadMode::Append => "load-adding",
    };
    locale.tr_plural(key, &[], &[("count", path_count)])
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
        // Interpolated counts carry Fluent bidi isolation marks by design.
        let locale = crate::i18n::LocaleManager::for_tests();
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 1, &locale),
            "Opening \u{2068}1\u{2069} file…"
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 2, &locale),
            "Opening \u{2068}2\u{2069} files…"
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Append, 1, &locale),
            "Adding \u{2068}1\u{2069} file…"
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Append, 3, &locale),
            "Adding \u{2068}3\u{2069} files…"
        );
        let russian = {
            use crate::i18n::preference::UiLanguagePreference;
            let mut manager = crate::i18n::LocaleManager::for_tests();
            manager.set_preference(UiLanguagePreference::Explicit("ru"));
            manager
        };
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 1, &russian),
            "Открытие \u{2068}1\u{2069} файла…"
        );
        assert_eq!(
            load_status_message(SceneLoadMode::Replace, 2, &russian),
            "Открытие \u{2068}2\u{2069} файлов…"
        );
    }
}

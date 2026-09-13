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
    /// State at the authorization boundary, before this may wait in a queue.
    pub(crate) content_revision_at_request: u64,
    pub(crate) dirty_at_request: bool,
}

pub(crate) struct PendingSceneLoad {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) source: &'static str,
    pub(crate) mode: SceneLoadMode,
    pub(crate) started_at: Instant,
    pub(crate) receiver: Receiver<Result<Scene>>,
    /// A newer Replace is queued. This decoder still owns the only live load
    /// slot, but its result must not be applied when it finishes.
    pub(crate) superseded: bool,
    /// Authorization state carried through the queue and decoder.
    pub(crate) content_revision_at_request: u64,
    pub(crate) dirty_at_request: bool,
}

/// A Replace authorization cannot cover edits made while queued or decoding.
/// Reconfirm only when those edits are still at risk of being discarded.
pub(crate) fn replace_result_requires_guard(
    authorized_revision: u64,
    current_revision: u64,
    dirty_at_authorization: bool,
    dirty_now: bool,
    edit_busy_now: bool,
) -> bool {
    edit_busy_now
        || (dirty_now && (authorized_revision != current_revision || !dirty_at_authorization))
}

/// Keep one decoder alive at a time. A new Replace supersedes its result and
/// every pending request; Append follows the current request in arrival order.
pub(crate) fn queue_request_while_active(
    active: &mut PendingSceneLoad,
    queued: &mut std::collections::VecDeque<SceneLoadRequest>,
    request: SceneLoadRequest,
) {
    if request.mode == SceneLoadMode::Replace {
        active.superseded = true;
        queued.clear();
    }
    queued.push_back(request);
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
    use std::path::Path;

    #[test]
    fn repeated_replace_keeps_one_active_decoder_and_only_the_latest_request() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut active = PendingSceneLoad {
            paths: vec![PathBuf::from("first.stl")],
            source: "test",
            mode: SceneLoadMode::Replace,
            started_at: Instant::now(),
            receiver,
            superseded: false,
            content_revision_at_request: 0,
            dirty_at_request: false,
        };
        let mut queued = std::collections::VecDeque::new();
        for path in ["second.stl", "third.stl"] {
            queue_request_while_active(
                &mut active,
                &mut queued,
                SceneLoadRequest {
                    paths: vec![PathBuf::from(path)],
                    source: "test",
                    mode: SceneLoadMode::Replace,
                    content_revision_at_request: 0,
                    dirty_at_request: false,
                },
            );
        }
        assert!(active.superseded);
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued.front().map(|request| request.paths[0].as_path()),
            Some(Path::new("third.stl"))
        );
        assert!(sender.send(Ok(Scene::new())).is_ok());
        assert!(active.receiver.try_recv().is_ok());
    }

    #[test]
    fn replace_must_reconfirm_edits_made_after_authorization() {
        assert!(!replace_result_requires_guard(4, 4, false, false, false));
        assert!(replace_result_requires_guard(4, 5, false, true, false));
        assert!(!replace_result_requires_guard(4, 4, true, true, false));
        assert!(replace_result_requires_guard(4, 5, true, true, false));
        assert!(!replace_result_requires_guard(4, 5, true, false, false));
        // An in-flight sculpt job may finish after the load starts without
        // having advanced the content revision yet.
        assert!(replace_result_requires_guard(4, 4, false, false, true));
    }

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

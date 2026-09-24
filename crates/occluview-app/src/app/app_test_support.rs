#![allow(clippy::expect_used)]

use super::*;
use occluview_core::{Mesh, SceneMesh, SceneMeshId, Vertex};

pub(super) fn test_app(name: &str) -> OccluViewApp {
    let _ = name;
    std::env::set_var("OCCLUVIEW_NO_UPDATE_CHECK", "1");
    std::env::set_var(crate::app_paths::TEST_STATE_DIR_ENV, test_state_dir());
    OccluViewApp::new_for_tests(egui::Context::default())
}

fn test_state_dir() -> PathBuf {
    let root = std::env::temp_dir().join(format!("occluview-app-tests-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    root
}

pub(super) fn named_scene(name: &str, x_offset: f32) -> Scene {
    let mesh = Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(glam::Vec3::new(x_offset, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x_offset + 1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x_offset, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("test mesh");
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    scene
}

pub(super) fn push_named_layer(scene: &mut Scene, name: &str, x_offset: f32) -> SceneMeshId {
    let mesh = Mesh::new(
        Some(name.to_string()),
        vec![
            Vertex::at(glam::Vec3::new(x_offset, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x_offset + 1.0, 0.0, 0.0)),
            Vertex::at(glam::Vec3::new(x_offset, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("test mesh");
    let entry = SceneMesh::new(mesh);
    let id = entry.id();
    scene.add(entry);
    id
}

pub(super) fn scene_names(app: &OccluViewApp) -> Vec<String> {
    app.document
        .scene
        .as_ref()
        .map(|scene| {
            scene
                .meshes()
                .iter()
                .map(|entry| entry.mesh.name().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn delivered_load(
    app: &OccluViewApp,
    scene: Scene,
    mode: SceneLoadMode,
    path: &str,
) -> PendingSceneLoad {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(scene))
        .expect("the channel was just created");
    PendingSceneLoad {
        paths: vec![PathBuf::from(path)],
        source: if mode == SceneLoadMode::Append {
            "add"
        } else {
            "open"
        },
        mode,
        started_at: Instant::now(),
        receiver,
        requested_at: Instant::now(),
        superseded: false,
        content_revision_at_request: app.document.content_revision,
        dirty_at_request: false,
    }
}

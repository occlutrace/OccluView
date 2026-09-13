//! Test-only construction of a real [`OccluViewApp`].
//!
//! Document-transition tests drive real methods (`apply_scene_load_result`,
//! `apply_history_navigation_now`, `apply_layer_overlay_changes`) against a real
//! app, because the interesting failures happen in the combination of the
//! history, the document, and the load pipeline — not in any one predicate.
//!
//! The production bootstrap cannot be reused: it acquires a process-wide
//! single-instance claim, reads the operator's state directory, and starts
//! worker threads. [`test_app`] keeps the same types and skips all three, so a
//! test never touches the machine it runs on.

#![allow(clippy::expect_used)]

use super::*;
use occluview_core::{Mesh, SceneMesh, SceneMeshId, Vertex};

/// A real app with a headless egui context, no live viewport, no startup files,
/// no single-instance claim, and a per-test temporary state directory.
pub(super) fn test_app(name: &str) -> OccluViewApp {
    let state_root =
        std::env::temp_dir().join(format!("occluview-app-tests-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state_root);
    let _ = std::fs::create_dir_all(&state_root);
    std::env::set_var("OCCLUVIEW_NO_UPDATE_CHECK", "1");
    std::env::remove_var("XDG_STATE_HOME");
    std::env::remove_var("APPDATA");
    std::env::remove_var("LOCALAPPDATA");
    std::env::set_var("OCCLUVIEW_TEST_STATE_DIR", &state_root);
    let ctx = egui::Context::default();
    OccluViewApp::new_for_tests(ctx)
}

/// A scene whose single layer is named, so a test can tell two scenes apart
/// without reading geometry.
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

/// Add a second named layer to a scene, for structural history tests.
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

/// A finished decode as `process_scene_loads` sees it once the channel yields.
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
        superseded: false,
        content_revision_at_request: app.document.content_revision,
        dirty_at_request: false,
    }
}

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SCENE_MESH_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity for one mesh layer inside the app scene graph.
///
/// Scene indices are transient UI positions. This id is created with the layer
/// and survives clone/remove/append operations so edit sessions can reject
/// stale results without binding to a shifting index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneMeshId(u64);

impl SceneMeshId {
    /// Numeric id value for storage in app-level state.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

pub(super) fn next_scene_mesh_id() -> SceneMeshId {
    SceneMeshId(NEXT_SCENE_MESH_ID.fetch_add(1, Ordering::Relaxed))
}

//! Inactive face-selection masks for a multi-layer edit session.

use std::collections::HashMap;

use occluview_core::{Scene, SceneMeshId};

use super::selection::FaceSelectionState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SelectionCacheKey {
    layer_id: SceneMeshId,
    topology_id: u64,
}

#[derive(Default)]
pub(super) struct SelectionCache {
    entries: HashMap<SelectionCacheKey, FaceSelectionState>,
}

impl SelectionCache {
    pub(super) fn store(&mut self, selection: FaceSelectionState, topology_id: u64) {
        let key = SelectionCacheKey {
            layer_id: selection.layer_id(),
            topology_id,
        };
        self.entries.insert(key, selection);
    }

    pub(super) fn take(
        &mut self,
        layer_id: SceneMeshId,
        topology_id: u64,
        triangle_count: usize,
    ) -> Option<FaceSelectionState> {
        self.entries
            .remove(&SelectionCacheKey {
                layer_id,
                topology_id,
            })
            .filter(|selection| selection.triangle_count() == triangle_count)
    }

    pub(super) fn retain_live_topologies(&mut self, scene: &Scene) {
        self.entries.retain(|key, _| {
            scene.meshes().iter().any(|entry| {
                entry.id() == key.layer_id && entry.mesh.topology_id() == key.topology_id
            })
        });
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

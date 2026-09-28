use super::material::linear_srgb_from_srgb;
use super::{Scene, SceneMesh};

impl Default for Scene {
    fn default() -> Self {
        Self {
            meshes: Vec::new(),
            // OccluTrace brand dark: #0a0a0a in sRGB.
            background: linear_srgb_from_srgb([0.039, 0.039, 0.039, 1.0]),
        }
    }
}

impl Scene {
    /// Construct an empty scene with OccluTrace defaults.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a mesh entry; returns its index in the scene.
    #[inline]
    pub fn add(&mut self, entry: SceneMesh) -> usize {
        let i = self.meshes.len();
        self.meshes.push(entry);
        i
    }

    /// Insert a mesh entry at `index`, clamping out-of-range inserts to append.
    #[inline]
    pub fn insert(&mut self, index: usize, entry: SceneMesh) -> usize {
        let index = index.min(self.meshes.len());
        self.meshes.insert(index, entry);
        index
    }

    /// Append every mesh entry from `other`, preserving this scene's existing
    /// order and scene-wide settings.
    #[inline]
    pub fn append_scene(&mut self, other: Scene) {
        self.meshes.extend(other.meshes);
    }

    /// Remove a mesh entry by index, returning it if it existed.
    #[inline]
    pub fn remove(&mut self, index: usize) -> Option<SceneMesh> {
        if index < self.meshes.len() {
            Some(self.meshes.remove(index))
        } else {
            None
        }
    }

    /// All mesh entries.
    #[inline]
    #[must_use]
    pub fn meshes(&self) -> &[SceneMesh] {
        &self.meshes
    }

    /// All mesh entries, mutable.
    #[inline]
    pub fn meshes_mut(&mut self) -> &mut [SceneMesh] {
        &mut self.meshes
    }

    /// Estimate memory reserved for this scene's layers and rendering.
    ///
    /// A shared mesh referenced by multiple layers is counted for every layer.
    /// The estimate includes CPU storage, lazy picking trees, GPU buffers and
    /// textures, and the optional wireframe buffer for every mesh.
    #[must_use]
    pub fn estimated_memory_bytes(&self) -> u64 {
        let entries = self
            .meshes
            .capacity()
            .saturating_mul(size_of::<SceneMesh>());
        let entry_bytes = u64::try_from(entries).unwrap_or(u64::MAX);
        self.meshes.iter().fold(entry_bytes, |total, mesh| {
            total
                .saturating_add(mesh.estimated_memory_bytes())
                .saturating_add(mesh.mesh.estimated_gpu_memory_bytes(true))
        })
    }

    /// Number of visible meshes.
    #[inline]
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.meshes.iter().filter(|m| m.visible).count()
    }
}

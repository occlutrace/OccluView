use super::material::linear_srgb_from_srgb;
use super::{Scene, SceneMesh};
use glam::Vec3;

impl Default for Scene {
    fn default() -> Self {
        Self {
            meshes: Vec::new(),
            // OccluTrace brand dark: #0a0a0a in sRGB.
            background: linear_srgb_from_srgb([0.039, 0.039, 0.039, 1.0]),
            ambient: 0.35,
            key_light_dir: Vec3::new(0.4, 0.8, 0.5).normalize_or_zero(),
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

    /// Number of visible meshes.
    #[inline]
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.meshes.iter().filter(|m| m.visible).count()
    }
}

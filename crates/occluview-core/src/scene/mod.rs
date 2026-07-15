//! The scene graph.
//!
//! A [`Scene`] holds one or more meshes and the per-mesh transforms/materials
//! that position and present them. The dental use case is upper + lower arch as
//! a two-mesh scene, so multi-mesh is first-class, not an afterthought.
//!
//! The scene is the bridge between file loading and rendering: loaders produce
//! meshes, the app composes a scene, the renderer consumes it.

mod bounds;
mod graph;
mod id;
mod material;
mod mesh_entry;
mod picking;
mod section;

#[cfg(test)]
mod tests;

use glam::Vec3;

pub use id::SceneMeshId;
pub use material::{DEFAULT_COLORED_MESH_TINT, DEFAULT_UNTEXTURED_MESH_TINT};
pub use mesh_entry::SceneMesh;
pub use occlu_mesh_edit::{SectionPlane, SectionPolyline, SectionResult};
pub use picking::ScenePickHit;
pub use section::{LayerSection, SceneSection, SectionCache, VisibilityFilter};

/// A scene: a collection of positioned, styled meshes plus a default lighting
/// and background. The dental "upper+lower" case is a 2-mesh scene.
#[derive(Clone, Debug)]
pub struct Scene {
    meshes: Vec<SceneMesh>,
    /// Scene-wide background color (linear sRGB). OccluTrace dark by default.
    pub background: [f32; 4],
    /// Ambient light intensity (0..1). Affects flat-shaded meshes.
    pub ambient: f32,
    /// Key light direction in scene space (unit length expected).
    pub key_light_dir: Vec3,
}

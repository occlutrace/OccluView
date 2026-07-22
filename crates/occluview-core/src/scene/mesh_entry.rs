use super::id::{next_scene_mesh_id, SceneMeshId};
use super::material::default_mesh_tint;
use crate::mesh::Mesh;
use glam::Affine3A;

/// Per-instance mesh entry in a scene.
// Five INDEPENDENT display toggles (visibility, wireframe, orientation
// diagnostic, vertex-color override, texture visibility) — orthogonal settings, not a
// state machine an enum would simplify.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct SceneMesh {
    id: SceneMeshId,
    /// The underlying mesh. Shared via `Arc` once we add cross-thread sharing;
    /// owned for now to keep `core` dependency-free.
    pub mesh: Mesh,
    /// Per-instance transform (placement of this mesh in the scene).
    pub transform: Affine3A,
    /// Display tint in linear sRGB (0..1). Textured/colored meshes default to
    /// neutral white; untextured scans default to warm dental stone.
    pub tint: [f32; 4],
    /// Opacity 0..1, used for transparency / ghost arches.
    pub opacity: f32,
    /// Whether this mesh is visible.
    pub visible: bool,
    /// Whether to draw a technical wireframe overlay for this layer.
    pub wireframe: bool,
    /// Diagnostic view (exocad "Show triangle orientation"): the renderer
    /// paints back-facing fragments of this layer solid red so inverted
    /// surfaces are unmistakable before "Invert normals".
    pub show_orientation: bool,
    /// Whether the renderer shades this layer with its own vertex color /
    /// texture (`true`, default) or a flat neutral material (`false`) — a
    /// display-only toggle for colored scans; the underlying color/texture
    /// data is untouched, so edits and exports keep the real colors.
    pub show_vertex_colors: bool,
    /// Whether an attached texture is sampled. This is separate from vertex
    /// colors so Tint can show a neutral material without destroying texture
    /// data or changing export behavior.
    pub show_texture: bool,
}

impl SceneMesh {
    /// Construct an entry from a mesh, identity transform, sensible default
    /// tint, opaque, visible.
    #[inline]
    #[must_use]
    pub fn new(mesh: Mesh) -> Self {
        let tint = default_mesh_tint(&mesh);
        let show_texture = mesh.texture().is_some();
        Self {
            id: next_scene_mesh_id(),
            mesh,
            transform: Affine3A::IDENTITY,
            tint,
            opacity: 1.0,
            visible: true,
            wireframe: false,
            show_orientation: false,
            show_vertex_colors: true,
            show_texture,
        }
    }

    /// Stable identity for this scene layer.
    #[inline]
    #[must_use]
    pub fn id(&self) -> SceneMeshId {
        self.id
    }

    /// Set the per-instance transform.
    #[inline]
    #[must_use]
    pub fn with_transform(mut self, t: Affine3A) -> Self {
        self.transform = t;
        self
    }

    /// Set the linear-sRGB tint.
    #[inline]
    #[must_use]
    pub fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    /// Set opacity 0..1.
    #[inline]
    #[must_use]
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Enable or disable the technical wireframe overlay.
    #[inline]
    #[must_use]
    pub fn with_wireframe(mut self, wireframe: bool) -> Self {
        self.wireframe = wireframe;
        self
    }

    /// Enable or disable shading this layer with its own vertex color /
    /// texture, versus a flat neutral material.
    #[inline]
    #[must_use]
    pub fn with_show_vertex_colors(mut self, show_vertex_colors: bool) -> Self {
        self.show_vertex_colors = show_vertex_colors;
        self
    }

    /// Enable or disable sampling the mesh texture at display time.
    #[inline]
    #[must_use]
    pub fn with_show_texture(mut self, show_texture: bool) -> Self {
        self.show_texture = show_texture;
        self
    }

    /// Build a layer snapshot with the same instance identity and display
    /// settings, replacing only its geometry. This avoids cloning the current
    /// mesh when a background editor already prepared an undo mesh.
    #[inline]
    #[must_use]
    pub fn with_mesh(&self, mesh: Mesh) -> Self {
        Self {
            id: self.id,
            mesh,
            transform: self.transform,
            tint: self.tint,
            opacity: self.opacity,
            visible: self.visible,
            wireframe: self.wireframe,
            show_orientation: self.show_orientation,
            show_vertex_colors: self.show_vertex_colors,
            show_texture: self.show_texture,
        }
    }
}

impl Default for SceneMesh {
    fn default() -> Self {
        Self::new(Mesh::empty())
    }
}

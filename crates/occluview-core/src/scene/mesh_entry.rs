use super::id::{next_scene_mesh_id, SceneMeshId};
use super::material::default_mesh_tint;
use crate::mesh::Mesh;
use crate::units::UnitInterpretation;
use glam::Affine3A;
use std::sync::Arc;

/// Per-instance mesh entry in a scene.
// Five INDEPENDENT display toggles (visibility, wireframe, orientation
// diagnostic, vertex-color override, texture visibility) — orthogonal settings, not a
// state machine an enum would simplify.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct SceneMesh {
    id: SceneMeshId,
    /// The geometry this entry places.
    ///
    /// Geometry is shared across scene copies and background readers; cloning
    /// a scene copies layer metadata, not vertex or texture buffers.
    pub mesh: Arc<Mesh>,
    /// Per-instance transform (placement of this mesh in the scene).
    pub transform: Affine3A,
    /// Display tint (0..1) in the renderer's own space: it is multiplied into
    /// the shaded colour and nothing encodes on the way out, so the number
    /// here is the number that reaches the screen. Textured/colored meshes
    /// default to neutral white; untextured scans default to warm dental stone.
    pub tint: [f32; 4],
    /// Opacity 0..1, used for transparency / ghost arches.
    pub opacity: f32,
    /// Whether this mesh is visible.
    pub visible: bool,
    /// Whether to draw a technical wireframe overlay for this layer.
    pub wireframe: bool,
    /// Diagnostic view (the dental CAD "Show triangle orientation"): the
    /// renderer paints back-facing fragments of this layer solid red so
    /// inverted surfaces are unmistakable before "Invert normals".
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
    /// Optional per-vertex deviation colors, one entry per mesh vertex.
    ///
    /// A **display overlay**, not mesh data: the renderer paints these instead
    /// of the scan's own colors, unlit, while `mesh` keeps every original
    /// color and texture. Hiding the map is therefore free, and an export is
    /// unaffected by whether a map happens to be on screen.
    deviation: Option<Arc<Vec<[u8; 4]>>>,
    /// Stable identity of the imported layer this entry was derived from.
    /// `None` means that this entry is itself a source layer. Structural
    /// operations use this identity to preserve export provenance when a
    /// source is split into one or more scene entries.
    source_layer_id: Option<SceneMeshId>,
    /// Import-unit metadata: what the file declared and what scale was
    /// applied to reach millimeters. Never affects rendering by itself —
    /// coordinates are normalized once, at import.
    import_units: UnitInterpretation,
}

impl SceneMesh {
    /// Construct an entry from a mesh, identity transform, sensible default
    /// tint, opaque, visible.
    #[inline]
    #[must_use]
    pub fn new(mesh: impl Into<Arc<Mesh>>) -> Self {
        let mesh = mesh.into();
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
            deviation: None,
            source_layer_id: None,
            import_units: UnitInterpretation::assumed_millimeters(),
        }
    }

    /// Record the import-unit interpretation for this layer. Layers built
    /// programmatically (tests, synthetic scenes) keep the assumed-mm
    /// default; file loaders overwrite it with the format policy.
    #[inline]
    #[must_use]
    pub fn with_import_units(mut self, units: UnitInterpretation) -> Self {
        self.import_units = units;
        self
    }

    /// Import-unit metadata attached at load time.
    #[inline]
    #[must_use]
    pub fn import_units(&self) -> UnitInterpretation {
        self.import_units
    }

    /// Attach or clear the deviation color overlay.
    #[inline]
    #[must_use]
    pub fn with_deviation(mut self, deviation: Option<Arc<Vec<[u8; 4]>>>) -> Self {
        self.deviation = deviation;
        self
    }

    /// Attach or clear the deviation color overlay in place.
    ///
    /// Use this setter for repeated color-scale changes to avoid cloning the
    /// layer metadata through [`Self::with_deviation`].
    #[inline]
    pub fn set_deviation(&mut self, deviation: Option<Arc<Vec<[u8; 4]>>>) {
        self.deviation = deviation;
    }

    /// The deviation color overlay, if this layer carries one.
    #[inline]
    #[must_use]
    pub fn deviation_colors(&self) -> Option<&Arc<Vec<[u8; 4]>>> {
        self.deviation.as_ref()
    }

    /// Stable identity for this scene layer.
    #[inline]
    #[must_use]
    pub fn id(&self) -> SceneMeshId {
        self.id
    }

    /// Stable identity of the source layer from which this entry was derived,
    /// if it is a split/cut part rather than an imported top-level layer.
    #[inline]
    #[must_use]
    pub fn source_layer_id(&self) -> Option<SceneMeshId> {
        self.source_layer_id
    }

    /// Identity used to carry file/export provenance through derived layers.
    /// A top-level imported layer is its own source.
    #[inline]
    #[must_use]
    pub fn export_source_layer_id(&self) -> SceneMeshId {
        self.source_layer_id.unwrap_or(self.id)
    }

    /// Mark this entry as a derived view of an imported source layer.
    #[inline]
    #[must_use]
    pub fn with_source_layer_id(mut self, source_layer_id: SceneMeshId) -> Self {
        self.source_layer_id = Some(source_layer_id);
        self
    }

    /// Set the per-instance transform.
    #[inline]
    #[must_use]
    pub fn with_transform(mut self, t: Affine3A) -> Self {
        self.transform = t;
        self
    }

    /// Set the display tint, in the renderer's own space -- see the `tint` field.
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
    pub fn with_mesh(&self, mesh: impl Into<Arc<Mesh>>) -> Self {
        Self {
            id: self.id,
            mesh: mesh.into(),
            transform: self.transform,
            tint: self.tint,
            opacity: self.opacity,
            visible: self.visible,
            wireframe: self.wireframe,
            show_orientation: self.show_orientation,
            show_vertex_colors: self.show_vertex_colors,
            show_texture: self.show_texture,
            // Dropped deliberately: the overlay is indexed by the old
            // vertices, so carrying it onto new geometry would paint whichever
            // vertices happened to inherit those indices.
            deviation: None,
            source_layer_id: self.source_layer_id,
            // Kept deliberately: same layer, same file provenance.
            import_units: self.import_units,
        }
    }
}

impl Default for SceneMesh {
    fn default() -> Self {
        Self::new(Mesh::empty())
    }
}

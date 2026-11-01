use super::id::{next_scene_mesh_id, SceneMeshId};
use super::material::default_mesh_tint;
use crate::mesh::Mesh;
use crate::units::{SourceUnit, UnitConfidence, UnitInterpretation};
use glam::Affine3A;
use std::sync::Arc;

/// What a layer's per-vertex display overlay means.
///
/// The overlay is one RGBA array either way, and the renderer has to branch on
/// what it is: a measured map states a value the operator reads against a
/// legend, while paint states a colour drawn over the surface's own material.
/// Treating paint as a measurement would drop the tint and the texture, so
/// the marked scan would read as a pale glossy shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayKind {
    /// A measured colour map: the RGB is the reading, drawn opaque, with the
    /// tint skipped so the ramp reaches the screen at its own hue.
    #[default]
    Measured,
    /// Paint over the surface's own material: the RGB is the paint colour and
    /// the alpha is the weight, so alpha 0 leaves the scan exactly as it
    /// renders — own colour, texture and lighting included.
    Paint,
}

/// One layer's per-vertex display overlay, with what its colours mean.
#[derive(Clone, Debug)]
struct MeshOverlay {
    kind: OverlayKind,
    colors: Arc<Vec<[u8; 4]>>,
}

/// Per-instance mesh entry in a scene.
// Five independent display toggles (visibility, wireframe, orientation
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
    /// Optional per-vertex overlay colours, one entry per mesh vertex, with
    /// what they mean ([`OverlayKind`]).
    ///
    /// A **display overlay**, not mesh data: the renderer draws it over the
    /// scan while `mesh` keeps every original colour and texture. Hiding it is
    /// therefore free, and an export is unaffected by whether one happens to
    /// be on screen.
    overlay: Option<MeshOverlay>,
    /// Stable identity of the imported layer this entry was derived from.
    /// `None` means that this entry is itself a source layer. Structural
    /// operations use this identity to preserve export provenance when a
    /// source is split into one or more scene entries.
    source_layer_id: Option<SceneMeshId>,
    /// Import-unit metadata: what the file declared and what scale was
    /// applied to reach millimeters. Metadata alone never changes geometry;
    /// an explicit decision updates the layer transform and this provenance.
    import_units: UnitInterpretation,
    /// Bounds of the original file geometry, before scene placement or edits.
    import_bbox: Option<crate::Aabb>,
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
            overlay: None,
            source_layer_id: None,
            import_units: UnitInterpretation::assumed_millimeters(),
            import_bbox: None,
        }
    }

    /// Record the import-unit interpretation for this layer. Layers built
    /// programmatically (tests, synthetic scenes) keep the assumed-mm
    /// default; file loaders overwrite it with the format policy.
    #[inline]
    #[must_use]
    pub fn with_import_units(mut self, units: UnitInterpretation) -> Self {
        self.import_units = units;
        self.import_bbox = Some(self.mesh.bbox());
        self
    }

    /// Import-unit metadata attached at load time.
    #[inline]
    #[must_use]
    pub fn import_units(&self) -> UnitInterpretation {
        self.import_units
    }

    /// Original local-space import bounds, independent of other scene layers.
    #[must_use]
    pub fn import_bbox(&self) -> crate::Aabb {
        self.import_bbox.unwrap_or_else(|| self.mesh.bbox())
    }

    /// Preserve source-unit provenance when extracting a new layer.
    #[must_use]
    pub fn with_import_provenance(mut self, source: &Self) -> Self {
        self.import_units = source.import_units;
        self.import_bbox = source.import_bbox;
        self
    }

    /// Apply an explicit interpretation to imported or operator-confirmed units.
    /// Repeating the same choice is a no-op; a changed choice uses the ratio
    /// to the scale already applied, rather than scaling source units twice.
    /// Scaling acts in the mesh's local frame, preserving its authored placement.
    /// The caller records the complete layer in Undo before invoking this.
    #[must_use]
    pub fn confirm_import_units(&mut self, interpreted: SourceUnit) -> bool {
        if !matches!(self.import_units.confidence,
            UnitConfidence::Ambiguous | UnitConfidence::ConfirmedByOperator)
        {
            return false;
        }
        let scale_to_mm: f32 = match interpreted {
            SourceUnit::Meters => 1000.0,
            SourceUnit::Millimeters => 1.0,
            SourceUnit::Unitless => return false,
        };
        let previous = self.import_units.scale_to_mm;
        if self.import_units.confidence == UnitConfidence::ConfirmedByOperator
            && previous.to_bits() == scale_to_mm.to_bits()
        {
            return false;
        }
        if !previous.is_finite() || previous <= 0.0 {
            return false;
        }
        let transform = self.transform
            * Affine3A::from_scale(glam::Vec3::splat(scale_to_mm / previous));
        if !transform.is_finite() {
            return false;
        }
        // A finite matrix can still overflow finite source coordinates. Check
        // the mesh bounds before accepting an interpretation into the scene.
        if !self.mesh.vertices().is_empty() {
            let bounds = self.mesh.bbox();
            for x in [bounds.min.x, bounds.max.x] {
                for y in [bounds.min.y, bounds.max.y] {
                    for z in [bounds.min.z, bounds.max.z] {
                        if !transform.transform_point3(glam::Vec3::new(x, y, z)).is_finite() {
                            return false;
                        }
                    }
                }
            }
        }
        self.transform = transform;
        self.import_units.scale_to_mm = scale_to_mm;
        self.import_units.confidence = UnitConfidence::ConfirmedByOperator;
        true
    }

    /// Attach or clear an overlay in place, naming what its colours mean.
    ///
    /// Use this setter for repeated colour changes to avoid cloning the layer
    /// metadata through [`Self::with_overlay`].
    #[inline]
    pub fn set_overlay(&mut self, kind: OverlayKind, colors: Option<Arc<Vec<[u8; 4]>>>) {
        self.overlay = colors.map(|colors| MeshOverlay { kind, colors });
    }

    /// Attach or clear an overlay, naming what its colours mean.
    #[inline]
    #[must_use]
    pub fn with_overlay(mut self, kind: OverlayKind, colors: Option<Arc<Vec<[u8; 4]>>>) -> Self {
        self.set_overlay(kind, colors);
        self
    }

    /// Drop the display overlay, whatever kind it is.
    #[inline]
    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    /// The per-vertex overlay colours, if this layer carries an overlay.
    #[inline]
    #[must_use]
    pub fn overlay_colors(&self) -> Option<&Arc<Vec<[u8; 4]>>> {
        self.overlay.as_ref().map(|overlay| &overlay.colors)
    }

    /// Estimate the CPU bytes retained by this layer, including display paint.
    #[must_use]
    pub fn estimated_memory_bytes(&self) -> u64 {
        let overlay_bytes = self
            .overlay_colors()
            .map_or(0, |colors| colors.capacity().saturating_mul(4));
        self.mesh
            .estimated_memory_bytes()
            .saturating_add(u64::try_from(overlay_bytes).unwrap_or(u64::MAX))
    }

    /// What the overlay colours mean, or nothing when there is no overlay.
    #[inline]
    #[must_use]
    pub fn overlay_kind(&self) -> Option<OverlayKind> {
        self.overlay.as_ref().map(|overlay| overlay.kind)
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
            // Dropped: the overlay is indexed by the old vertices, so carrying
            // it onto new geometry would paint whichever vertices happened to
            // inherit those indices.
            overlay: None,
            source_layer_id: self.source_layer_id,
            // Kept: same layer, same file provenance.
            import_units: self.import_units,
            import_bbox: self.import_bbox,
        }
    }
}

impl Default for SceneMesh {
    fn default() -> Self {
        Self::new(Mesh::empty())
    }
}

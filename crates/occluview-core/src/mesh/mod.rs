//! The mesh data model.
//!
//! A [`Mesh`] is the unit of geometry that flows from a format loader, through
//! the scene graph, into the renderer. It is GPU- and I/O-agnostic on purpose:
//! `occluview-render` owns the GPU buffers, `occluview-formats` owns the
//! readers/writers.
//!
//! ## Layout
//!
//! Vertices store position, normal, and an optional RGBA color (dental color /
//! NIR scans). Indices are 32-bit triangles. Colors are optional so
//! plain STL (no color) doesn't pay for a zeroed channel.

mod bridge_split_adapter;
#[cfg(feature = "robust-csg")]
mod bridge_split_robust;
mod builder;
mod edit_adapter;
mod normals;
mod principal_axis;
mod texture;
mod vertex;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod bridge_split_tests;

#[cfg(all(test, feature = "robust-csg"))]
mod bridge_split_robust_tests;

use crate::bbox::Aabb;
use crate::error::CoreError;
use glam::Vec3;
use std::sync::atomic::{AtomicU64, Ordering};

pub use bridge_split_adapter::{
    bridge_split_mesh_in_world, bridge_split_prepared_mesh_in_world, normalize_bridge_split_input,
    prepare_bridge_split_source, CoreBridgeSplitError, CoreBridgeSplitResult,
    PreparedBridgeSplitSource,
};
pub use builder::MeshBuilder;
pub use edit_adapter::{
    component_at_triangle_in_mesh, crop_mesh_to_selected_faces, delete_selected_faces_in_mesh,
    fill_holes_in_mesh, fill_selected_holes_in_mesh, invert_mesh_orientation,
    mesh_edit_buffers_from_mesh, mesh_from_edit_buffers_like, repair_mesh_in_mesh,
    selected_connected_components_in_mesh, smooth_selected_faces_in_mesh, CoreMeshEditResult,
    CoreMeshRepairResult,
};
pub use principal_axis::PrincipalFrame;
pub use texture::MeshTexture;
pub use vertex::Vertex;

static NEXT_MESH_TOPOLOGY_ID: AtomicU64 = AtomicU64::new(1);

/// Whether a [`Mesh`] carries triangle connectivity or is just a point cloud.
///
/// Dental color scanners frequently emit PLY files with `element vertex` but
/// no `element face` — a point cloud. The renderer draws these with a
/// different pipeline (point list, not triangle list); the loader sets the
/// kind so downstream code can branch.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum MeshKind {
    /// Triangle mesh: `indices` is non-empty and indexes triangle corners.
    #[default]
    TriangleMesh,
    /// Point cloud: `indices` is empty; each vertex is drawn as a point.
    PointCloud,
}

/// A triangle mesh, the central geometry type.
#[derive(Clone, Debug)]
pub struct Mesh {
    name: Option<String>,
    vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`. Length must be a multiple of 3.
    indices: Vec<u32>,
    /// True if any vertex carries a non-default color.
    has_vertex_colors: bool,
    /// True if any vertex carries non-zero UV coordinates.
    has_uvs: bool,
    /// Whether this is a triangle mesh or a point cloud.
    kind: MeshKind,
    /// Decoded texture image, if the source file provided one (glTF
    /// `image`/`texture`). `None` for plain STL / untextured meshes.
    texture: Option<MeshTexture>,
    /// Cached bounding box, lazily computed.
    cached_bbox: Option<Aabb>,
    /// Cached principal-axis frame (centroid + orthonormal axes), computed
    /// once at construction time — see [`Mesh::principal_frame_cached`].
    cached_principal_frame: Option<PrincipalFrame>,
    /// Stable identity for GPU-uploaded geometry/texture payload.
    topology_id: u64,
}

impl Mesh {
    /// An empty mesh — no vertices, no triangles.
    ///
    /// Cheap to construct; used as a placeholder (e.g. for `Default`).
    #[must_use]
    #[inline]
    pub fn empty() -> Self {
        Self {
            name: None,
            vertices: Vec::new(),
            indices: Vec::new(),
            has_vertex_colors: false,
            has_uvs: false,
            kind: MeshKind::default(),
            texture: None,
            cached_bbox: Some(Aabb::EMPTY),
            cached_principal_frame: None,
            topology_id: next_mesh_topology_id(),
        }
    }

    /// Construct a point cloud from vertices only (no indices). Sets
    /// [`MeshKind::PointCloud`].
    #[must_use]
    pub fn point_cloud(name: Option<String>, vertices: Vec<Vertex>) -> Self {
        let has_vertex_colors = vertices.iter().any(|v| v.color != [255, 255, 255, 255]);
        let has_uvs = vertices.iter().any(|v| v.uv != [0.0, 0.0]);
        let cached_bbox = Some(Aabb::enclose_points(
            vertices.iter().map(|v| Vec3::from_array(v.position)),
        ));
        let cached_principal_frame =
            principal_axis::principal_frame(vertices.iter().map(|v| Vec3::from_array(v.position)));
        Self {
            name,
            vertices,
            indices: Vec::new(),
            has_vertex_colors,
            has_uvs,
            kind: MeshKind::PointCloud,
            texture: None,
            cached_bbox,
            cached_principal_frame,
            topology_id: next_mesh_topology_id(),
        }
    }

    /// Construct from parts. Validates the index range.
    ///
    /// # Errors
    /// - [`CoreError::IndexOutOfRange`] if any index exceeds the vertex count.
    /// - [`CoreError::IndexCountNotMultipleOfThree`] if `indices.len() % 3 != 0`.
    pub fn new(
        name: Option<String>,
        mut vertices: Vec<Vertex>,
        indices: Vec<u32>,
    ) -> Result<Self, CoreError> {
        if indices.len() % 3 != 0 {
            return Err(CoreError::IndexCountNotMultipleOfThree {
                index_count: indices.len(),
            });
        }
        let vertex_count = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        if let Some((i, bad)) = indices.iter().enumerate().find(|(_, &v)| v >= vertex_count) {
            return Err(CoreError::IndexOutOfRange {
                at_index: i,
                value: *bad,
                vertex_count,
            });
        }
        if !indices.is_empty() {
            normals::repair_missing_normals(&mut vertices, &indices);
        }
        let has_vertex_colors = vertices.iter().any(|v| v.color != [255, 255, 255, 255]);
        let has_uvs = vertices.iter().any(|v| v.uv != [0.0, 0.0]);
        let cached_bbox = Some(Aabb::enclose_points(
            vertices.iter().map(|v| Vec3::from_array(v.position)),
        ));
        let cached_principal_frame =
            principal_axis::principal_frame(vertices.iter().map(|v| Vec3::from_array(v.position)));
        Ok(Self {
            name,
            vertices,
            indices,
            has_vertex_colors,
            has_uvs,
            kind: MeshKind::TriangleMesh,
            texture: None,
            cached_bbox,
            cached_principal_frame,
            topology_id: next_mesh_topology_id(),
        })
    }

    /// Optional human-readable name (e.g. file stem, "upper arch").
    #[inline]
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// All vertices, in upload order.
    #[inline]
    #[must_use]
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    /// Triangle indices (every 3 = one triangle).
    #[inline]
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Number of triangles.
    #[inline]
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// True if any vertex carries a non-default color.
    #[inline]
    #[must_use]
    pub fn has_vertex_colors(&self) -> bool {
        self.has_vertex_colors
    }

    /// True if any vertex carries non-zero UV coordinates. Loaders set UVs
    /// from `TEXCOORD_0` (glTF), `vt` (OBJ), or `texcoord` (PLY).
    #[inline]
    #[must_use]
    pub fn has_uvs(&self) -> bool {
        self.has_uvs
    }

    /// The decoded texture image, if the source file provided one.
    #[inline]
    #[must_use]
    pub fn texture(&self) -> Option<&MeshTexture> {
        self.texture.as_ref()
    }

    /// Attach a decoded texture image (e.g. from a glTF `image`). Used by
    /// loaders after constructing the mesh.
    #[inline]
    pub fn set_texture(&mut self, texture: MeshTexture) {
        self.texture = Some(texture);
        self.topology_id = next_mesh_topology_id();
    }

    /// Stable identity for the GPU-uploaded mesh payload.
    ///
    /// Cloning a mesh preserves this value so material-only scene edits can
    /// reuse prepared GPU buffers. Constructing a new mesh or replacing its
    /// texture gets a fresh value so renderers know to rebuild uploads.
    #[inline]
    #[must_use]
    pub fn topology_id(&self) -> u64 {
        self.topology_id
    }

    /// Whether this is a triangle mesh or a point cloud.
    #[inline]
    #[must_use]
    pub fn kind(&self) -> MeshKind {
        self.kind
    }

    /// True if this geometry has no triangle connectivity (point cloud).
    #[inline]
    #[must_use]
    pub fn is_point_cloud(&self) -> bool {
        self.kind == MeshKind::PointCloud
    }

    /// Bounding box computed fresh from vertices, **without** touching the
    /// cache. Used by scene-level composition (which folds many meshes' boxes
    /// without wanting to mutate each one).
    #[must_use]
    pub fn bbox_uncached(&self) -> Aabb {
        Aabb::enclose_points(self.vertices.iter().map(|v| Vec3::from_array(v.position)))
    }

    /// Axis-aligned bounding box from the constructor-time cache.
    ///
    /// This is the read-only fast path used by scene composition. Mesh geometry
    /// is immutable after construction, so the cached box remains valid.
    #[must_use]
    pub fn bbox_cached(&self) -> Aabb {
        self.cached_bbox.unwrap_or_else(|| self.bbox_uncached())
    }

    /// Axis-aligned bounding box, computed once and cached.
    #[inline]
    #[must_use]
    pub fn bbox(&mut self) -> Aabb {
        if let Some(b) = self.cached_bbox {
            return b;
        }
        let b = self.bbox_uncached();
        self.cached_bbox = Some(b);
        b
    }

    /// True if this mesh carries scan color or texture data (a decoded
    /// texture image, or non-default per-vertex colors) — the single source
    /// of truth for whether display logic should defer to that color data
    /// instead of a neutral/default tint.
    #[inline]
    #[must_use]
    pub fn carries_color_data(&self) -> bool {
        self.texture.is_some() || self.has_vertex_colors
    }

    /// The mesh's own principal-axis frame (PCA centroid + orthonormal
    /// axes), from the constructor-time cache — a STABLE, per-mesh-constant
    /// "global shape" signal: `axes[0]` is a dental arch or bridge span's own
    /// mesiodistal direction, unaffected by cursor position or local surface
    /// bumps, and the LOCAL direction from `centroid` to any surface point,
    /// projected onto the `axes[0]`/`axes[1]` plane, rotates smoothly around
    /// the arch. `None` when the mesh has fewer than 3 distinct vertex
    /// positions (no well-defined frame — e.g. an empty mesh, or one
    /// degenerate point).
    ///
    /// Mesh geometry is immutable after construction (mirrors
    /// [`Mesh::bbox_cached`]), so the cached frame remains valid.
    #[inline]
    #[must_use]
    pub fn principal_frame_cached(&self) -> Option<PrincipalFrame> {
        self.cached_principal_frame
    }
}

pub(super) fn next_mesh_topology_id() -> u64 {
    NEXT_MESH_TOPOLOGY_ID.fetch_add(1, Ordering::Relaxed)
}

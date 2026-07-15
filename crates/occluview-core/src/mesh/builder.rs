use super::{Mesh, Vertex};
use crate::error::CoreError;

/// Builder for a [`Mesh`]. Useful when a loader streams vertices/indices.
#[derive(Default, Debug)]
pub struct MeshBuilder {
    name: Option<String>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    /// If true, `build()` produces a [`MeshKind::PointCloud`] regardless of
    /// indices. Set by loaders that know there is no face element.
    force_point_cloud: bool,
}

impl MeshBuilder {
    /// Construct an empty builder.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the result as a point cloud (no triangle connectivity).
    /// Loaders call this when the source format declares vertices but no faces.
    #[inline]
    #[must_use]
    pub const fn as_point_cloud(mut self) -> Self {
        self.force_point_cloud = true;
        self
    }

    /// Set the optional name.
    #[inline]
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Reserve space for `n` vertices and `i` indices.
    #[inline]
    #[must_use]
    pub fn reserve(mut self, vertices: usize, indices: usize) -> Self {
        self.vertices.reserve(vertices);
        self.indices.reserve(indices);
        self
    }

    /// Push a vertex; returns its index for convenience.
    #[inline]
    pub fn push_vertex(&mut self, v: Vertex) -> u32 {
        let idx = u32::try_from(self.vertices.len()).unwrap_or(u32::MAX);
        self.vertices.push(v);
        idx
    }

    /// Push a triangle by vertex indices.
    #[inline]
    pub fn push_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Finalize into a [`Mesh`], validating indices.
    ///
    /// # Errors
    /// See [`Mesh::new`].
    pub fn build(self) -> Result<Mesh, CoreError> {
        if self.force_point_cloud {
            return Ok(Mesh::point_cloud(self.name, self.vertices));
        }
        Mesh::new(self.name, self.vertices, self.indices)
    }
}

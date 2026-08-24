use super::{Mesh, Vertex};
use crate::error::CoreError;

/// Builder for a [`Mesh`]. Useful when a loader streams vertices/indices.
#[derive(Default, Debug)]
pub struct MeshBuilder {
    name: Option<String>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    /// If true, `build()` produces a [`crate::MeshKind::PointCloud`] regardless of
    /// indices. Set by loaders that know there is no face element.
    force_point_cloud: bool,
    /// Bytes of geometry this builder may hold, from the size of the file the
    /// reader was handed. `None` for callers that build a mesh from nothing.
    mesh_byte_budget: Option<u64>,
    outgrew_source: bool,
}

/// Bytes of geometry a reader may build per byte of file.
///
/// Measured across sixty real scans on this machine, the highest ratio is 3.2:
/// a dense text OBJ, whose short coordinate lines are the least efficient
/// encoding a scanner writes. Twelve leaves that nearly four times over, and
/// still refuses the shapes that end a process: a face line whose fan
/// triangulation emits a fresh vertex per token (measured at 25), an OFF
/// header whose declared count is reserved as twelve-byte elements (38), and a
/// glTF where two thousand primitives each re-emit the same accessor (six
/// thousand, which asked the allocator for 19 GB from a 3 MB file).
const MESH_BYTES_PER_INPUT_BYTE: u64 = 12;
const BYTES_PER_VERTEX: u64 = 36;
const BYTES_PER_INDEX: u64 = 4;

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

    /// Bound the geometry by the size of the file it is being read from.
    ///
    /// A reader that grows a mesh from tokens can be made to grow one far
    /// larger than its input: the allocation then fails, and an allocation
    /// failure aborts the process rather than unwinding, so no caller can
    /// catch it. With a budget set, the builder stops growing and `build`
    /// reports [`CoreError::MeshOutgrewItsSource`] instead.
    #[inline]
    #[must_use]
    pub const fn from_input_of(mut self, input_bytes: usize) -> Self {
        self.mesh_byte_budget =
            Some((input_bytes as u64).saturating_mul(MESH_BYTES_PER_INPUT_BYTE));
        self
    }

    fn within_budget(&self, extra_bytes: u64) -> bool {
        let Some(budget) = self.mesh_byte_budget else {
            return true;
        };
        self.mesh_bytes().saturating_add(extra_bytes) <= budget
    }

    fn mesh_bytes(&self) -> u64 {
        (self.vertices.len() as u64).saturating_mul(BYTES_PER_VERTEX)
            + (self.indices.len() as u64).saturating_mul(BYTES_PER_INDEX)
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
        if !self.within_budget(BYTES_PER_VERTEX) {
            self.outgrew_source = true;
            return u32::try_from(self.vertices.len().saturating_sub(1)).unwrap_or(u32::MAX);
        }
        let idx = u32::try_from(self.vertices.len()).unwrap_or(u32::MAX);
        self.vertices.push(v);
        idx
    }

    /// Push a triangle by vertex indices.
    #[inline]
    pub fn push_triangle(&mut self, a: u32, b: u32, c: u32) {
        if !self.within_budget(BYTES_PER_INDEX * 3) {
            self.outgrew_source = true;
            return;
        }
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Finalize into a [`Mesh`], validating indices.
    ///
    /// # Errors
    /// See [`Mesh::new`].
    pub fn build(self) -> Result<Mesh, CoreError> {
        if self.outgrew_source {
            return Err(CoreError::MeshOutgrewItsSource {
                mesh_bytes: self.mesh_bytes(),
                input_bytes: self.mesh_byte_budget.unwrap_or(0) / MESH_BYTES_PER_INPUT_BYTE,
            });
        }
        if self.force_point_cloud {
            return Ok(Mesh::point_cloud(self.name, self.vertices));
        }
        Mesh::new(self.name, self.vertices, self.indices)
    }

    /// Finalize into a mesh meant for an image rather than for work on it.
    ///
    /// See [`Mesh::new_for_preview`] for what is skipped and what it costs.
    ///
    /// # Errors
    /// See [`Mesh::new`].
    pub fn build_for_preview(self) -> Result<Mesh, CoreError> {
        if self.outgrew_source {
            return Err(CoreError::MeshOutgrewItsSource {
                mesh_bytes: self.mesh_bytes(),
                input_bytes: self.mesh_byte_budget.unwrap_or(0) / MESH_BYTES_PER_INPUT_BYTE,
            });
        }
        if self.force_point_cloud {
            return Ok(Mesh::point_cloud(self.name, self.vertices));
        }
        Mesh::new_for_preview(self.name, self.vertices, self.indices)
    }
}

#[cfg(test)]
mod tests {
    use super::{MeshBuilder, MESH_BYTES_PER_INPUT_BYTE};
    use crate::error::CoreError;
    use crate::mesh::Vertex;
    use glam::Vec3;

    /// A reader that grows a mesh from tokens can be handed a file that makes
    /// it grow one far larger than itself. The allocation then fails, and an
    /// allocation failure aborts the process: no caller can catch it. The
    /// builder stops instead, and says why.
    #[test]
    fn geometry_that_outgrows_its_file_is_refused_rather_than_allocated() {
        let input_bytes = 1_000;
        let mut builder = MeshBuilder::new().from_input_of(input_bytes);
        // Well past the budget: every push beyond it is dropped.
        for _ in 0..10_000 {
            let index = builder.push_vertex(Vertex::at(Vec3::ZERO));
            builder.push_triangle(index, index, index);
        }
        let error = builder.build().expect_err("the budget must be reported");
        assert!(
            matches!(error, CoreError::MeshOutgrewItsSource { .. }),
            "unexpected error: {error}"
        );
    }

    /// The budget must leave every real scan alone. Measured across sixty on
    /// this machine, the densest is 3.2 bytes of geometry per byte of file.
    #[test]
    fn the_ratio_a_real_scan_reaches_is_well_inside_the_budget() {
        // A text OBJ reaches 3.2 bytes of geometry per byte of file; the
        // budget below has to leave that room, which the pushes then prove.
        let vertices = 1_000;
        let input_bytes = vertices * 16;
        let mut builder = MeshBuilder::new().from_input_of(input_bytes);
        for _ in 0..vertices {
            let index = builder.push_vertex(Vertex::at(Vec3::ZERO));
            builder.push_triangle(index, index, index);
        }
        let mesh = builder.build().expect("a scan-shaped mesh fits");
        assert_eq!(mesh.vertices().len(), vertices);
        let realistic_ratio = 4;
        assert!(
            MESH_BYTES_PER_INPUT_BYTE > realistic_ratio,
            "the budget must sit above what a real scan reaches"
        );
    }

    /// A builder with no declared source keeps building; the app assembles
    /// meshes that came from no file at all.
    #[test]
    fn a_builder_with_no_source_has_no_budget() {
        let mut builder = MeshBuilder::new();
        for _ in 0..10_000 {
            builder.push_vertex(Vertex::at(Vec3::ZERO));
        }
        let mesh = builder.build().expect("no source, no ceiling");
        assert_eq!(mesh.vertices().len(), 10_000);
    }
}

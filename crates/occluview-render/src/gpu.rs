//! GPU mesh: vertex/index buffers uploaded from `occluview_core::Mesh`.
//!
//! The CPU `Vertex` is `#[repr(C)]` and laid out exactly as the WGSL vertex
//! shader expects (`position`/`normal`/`color`). We upload with a bytemuck
//! cast slice - no reformatting.

use crate::error::RenderError;
use occluview_core::{Mesh, Vertex};

/// A mesh resident on the GPU: vertex buffer, index buffer, and the index count
/// for the draw call.
pub struct GpuMesh {
    pub(crate) vertex_buffer: wgpu::Buffer,
    pub(crate) index_buffer: wgpu::Buffer,
    pub(crate) index_count: u32,
    pub(crate) wireframe_index_buffer: Option<wgpu::Buffer>,
    pub(crate) wireframe_index_count: u32,
    /// Vertex count (for `PointCloud` draws that don't use the index buffer).
    pub(crate) vertex_count: u32,
}

impl GpuMesh {
    /// Upload a CPU mesh to the GPU. The mesh's vertices and indices are
    /// copied into fresh `wgpu::Buffer`s with `COPY_DST` usage.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &Mesh) -> Self {
        Self::upload_with_wireframe(device, queue, mesh, false)
    }

    /// Upload a CPU mesh and optionally prepare a line-list index buffer for a
    /// technical wireframe overlay.
    pub fn upload_with_wireframe(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &Mesh,
        include_wireframe: bool,
    ) -> Self {
        let vertices = mesh.vertices();
        let indices = mesh.indices();

        let vertex_bytes: &[u8] = bytemuck::cast_slice(vertices);
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview vertex buffer"),
            size: vertex_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, vertex_bytes);

        let index_bytes: &[u8] = bytemuck::cast_slice(indices);
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview index buffer"),
            size: index_bytes.len() as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, index_bytes);

        let (wireframe_index_buffer, wireframe_index_count) =
            if include_wireframe && !mesh.is_point_cloud() && !indices.is_empty() {
                let wireframe_indices = wireframe_indices_for_triangle_mesh(indices);
                let wireframe_index_bytes: &[u8] = bytemuck::cast_slice(&wireframe_indices);
                let wireframe_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("occluview wireframe index buffer"),
                    size: wireframe_index_bytes.len() as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&wireframe_index_buffer, 0, wireframe_index_bytes);
                (
                    Some(wireframe_index_buffer),
                    u32::try_from(wireframe_indices.len()).unwrap_or(u32::MAX),
                )
            } else {
                (None, 0)
            };

        Self {
            vertex_buffer,
            index_buffer,
            // Saturate rather than truncate: a >4 billion element mesh cannot be
            // uploaded (wgpu indexes with u32) and cannot fit in memory, but a
            // silent wrap would draw garbage. Matches the wireframe count above.
            index_count: u32::try_from(indices.len()).unwrap_or(u32::MAX),
            wireframe_index_buffer,
            wireframe_index_count,
            vertex_count: u32::try_from(vertices.len()).unwrap_or(u32::MAX),
        }
    }

    /// Vertex-buffer layout describing the `Vertex` struct to wgpu.
    pub(crate) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position: vec3<f32> @ offset 0
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                // normal: vec3<f32> @ offset 12
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                // color: u8x4 @ offset 24 (UNORM -> 0..1 float in shader via /255)
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint8x4,
                    offset: 24,
                    shader_location: 2,
                },
                // uv: vec2<f32> @ offset 28
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 28,
                    shader_location: 3,
                },
            ],
        }
    }

    /// Number of indices (the draw-call count).
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.index_count
    }

    /// Minimum binding size for a uniform buffer holding a [`crate::GpuCamera`].
    /// Used by the pipeline builder; exported here for tests.
    ///
    /// [`crate::GpuCamera`] is a fixed 160-byte POD, so this never returns `None`.
    #[must_use]
    pub fn camera_uniform_size() -> u64 {
        size_of::<crate::GpuCamera>() as u64
    }

    /// Issue the draw for this mesh into `render_pass`. Triangle meshes use
    /// `draw_indexed`; point clouds use `draw` over all vertices.
    pub(crate) fn draw(&self, rpass: &mut wgpu::RenderPass<'_>, kind: occluview_core::MeshKind) {
        rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        match kind {
            occluview_core::MeshKind::TriangleMesh => {
                rpass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            occluview_core::MeshKind::PointCloud => {
                rpass.draw(0..self.vertex_count, 0..1);
            }
        }
    }

    /// Issue a line-list draw for a prepared triangle-mesh wireframe overlay.
    pub(crate) fn draw_wireframe(&self, rpass: &mut wgpu::RenderPass<'_>) {
        let Some(index_buffer) = self.wireframe_index_buffer.as_ref() else {
            return;
        };
        if self.wireframe_index_count == 0 {
            return;
        }
        rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..self.wireframe_index_count, 0, 0..1);
    }

    #[must_use]
    pub(crate) fn has_wireframe_indices(&self) -> bool {
        self.wireframe_index_buffer.is_some() && self.wireframe_index_count > 0
    }
}

fn wireframe_indices_for_triangle_mesh(indices: &[u32]) -> Vec<u32> {
    let mut lines = Vec::with_capacity(indices.len() * 2);
    for tri in indices.chunks_exact(3) {
        let a = tri[0];
        let b = tri[1];
        let c = tri[2];
        lines.extend_from_slice(&[a, b, b, c, c, a]);
    }
    lines
}

/// Build a `wgpu::BindGroupLayout` for the camera uniform (group 0, binding 0).
pub(crate) fn camera_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview camera layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Convert a `wgpu::Error` (from a device-loss callback) into our error type.
pub fn map_wgpu_error(e: wgpu::Error) -> RenderError {
    RenderError::Surface(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::wireframe_indices_for_triangle_mesh;

    #[test]
    fn wireframe_indices_expand_triangles_to_line_pairs() {
        let indices = wireframe_indices_for_triangle_mesh(&[0, 1, 2, 2, 1, 3]);

        assert_eq!(indices, vec![0, 1, 1, 2, 2, 0, 2, 1, 1, 3, 3, 2]);
    }
}

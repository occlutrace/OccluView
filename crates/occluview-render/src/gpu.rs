//! GPU mesh: vertex/index buffers uploaded from `occluview_core::Mesh`.
//!
//! The CPU `Vertex` is `#[repr(C)]` and laid out exactly as the WGSL vertex
//! shader expects (`position`/`normal`/`color`). We upload with a bytemuck
//! cast slice - no reformatting.

use crate::error::RenderError;
use occluview_core::{Mesh, Vertex};
use std::mem::size_of;

/// A mesh resident on the GPU: vertex buffer, index buffer, and the index count
/// for the draw call.
pub struct GpuMesh {
    pub(crate) vertex_buffer: wgpu::Buffer,
    pub(crate) index_buffer: wgpu::Buffer,
    pub(crate) index_count: u32,
    /// Vertex count (for `PointCloud` draws that don't use the index buffer).
    pub(crate) vertex_count: u32,
}

impl GpuMesh {
    /// Upload a CPU mesh to the GPU. The mesh's vertices and indices are
    /// copied into fresh `wgpu::Buffer`s with `COPY_DST` usage.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &Mesh) -> Self {
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

        Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            vertex_count: vertices.len() as u32,
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
    /// [`GpuCamera`] is a fixed 160-byte POD, so this never returns `None`.
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

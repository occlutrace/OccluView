//! GPU mesh: vertex/index buffers uploaded from `occluview_core::Mesh`.
//!
//! The CPU `Vertex` is `#[repr(C)]` and laid out exactly as the WGSL vertex
//! shader expects (`position`/`normal`/`color`). We upload with a bytemuck
//! cast slice - no reformatting.

use super::offscreen::{SculptBufferUpdateStats, SculptTopologyDelta};
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
    pub(crate) live_index_count: u32,
    vertex_capacity_bytes: u64,
    index_capacity_bytes: u64,
    wireframe_index_capacity_bytes: u64,
}

pub(crate) struct SculptGeometry<'a> {
    pub(crate) vertices: &'a [Vertex],
    pub(crate) indices: &'a [u32],
    pub(crate) draw_wireframe: bool,
}

struct SculptDeltaLayout {
    vertex_count: u32,
    live_index_count: u32,
    base_triangles: usize,
    live_triangles: usize,
    draw_index_count: u32,
    vertex_bytes: u64,
    index_bytes: u64,
    wireframe_bytes: u64,
    vertex_copy_bytes: u64,
    index_copy_bytes: u64,
    wireframe_copy_bytes: u64,
    wireframe_index_count: u32,
    draw_wireframe: bool,
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
        let vertex_capacity_bytes = capacity_bytes(vertex_bytes.len());
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview vertex buffer"),
            size: vertex_capacity_bytes,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        if !vertex_bytes.is_empty() {
            queue.write_buffer(&vertex_buffer, 0, vertex_bytes);
        }

        let index_bytes: &[u8] = bytemuck::cast_slice(indices);
        let index_capacity_bytes = capacity_bytes(index_bytes.len());
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview index buffer"),
            size: index_capacity_bytes,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        if !index_bytes.is_empty() {
            queue.write_buffer(&index_buffer, 0, index_bytes);
        }

        let (wireframe_index_buffer, wireframe_index_count, wireframe_index_capacity_bytes) =
            if include_wireframe && !mesh.is_point_cloud() && !indices.is_empty() {
                let wireframe_indices = wireframe_indices_for_triangle_mesh(indices);
                let wireframe_index_bytes: &[u8] = bytemuck::cast_slice(&wireframe_indices);
                let capacity = capacity_bytes(wireframe_index_bytes.len());
                let wireframe_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("occluview wireframe index buffer"),
                    size: capacity,
                    usage: wgpu::BufferUsages::INDEX
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&wireframe_index_buffer, 0, wireframe_index_bytes);
                (
                    Some(wireframe_index_buffer),
                    u32::try_from(wireframe_indices.len()).unwrap_or(u32::MAX),
                    capacity,
                )
            } else {
                (None, 0, 0)
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
            live_index_count: u32::try_from(indices.len()).unwrap_or(u32::MAX),
            vertex_capacity_bytes,
            index_capacity_bytes,
            wireframe_index_capacity_bytes,
        }
    }

    pub(crate) fn write_sculpt_delta(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        delta: &SculptTopologyDelta,
        draw_wireframe: bool,
    ) -> Option<SculptBufferUpdateStats> {
        let layout = self.sculpt_delta_layout(delta, draw_wireframe)?;
        let mut stats = self.ensure_sculpt_delta_capacity(device, queue, &layout)?;
        self.upload_sculpt_delta(queue, delta, &layout, &mut stats);
        self.vertex_count = layout.vertex_count;
        self.live_index_count = layout.live_index_count;
        self.index_count = layout.draw_index_count;
        Some(stats)
    }

    fn sculpt_delta_layout(
        &self,
        delta: &SculptTopologyDelta,
        draw_wireframe: bool,
    ) -> Option<SculptDeltaLayout> {
        let base_vertex_count = usize::try_from(self.vertex_count).ok()?;
        let base_index_count = usize::try_from(self.live_index_count).ok()?;
        if delta.base_vertex_count != base_vertex_count
            || delta.base_index_count != base_index_count
            || !delta.live_index_count.is_multiple_of(3)
            || !delta.base_index_count.is_multiple_of(3)
            || (draw_wireframe && self.wireframe_index_buffer.is_none())
        {
            return None;
        }
        let vertex_count = delta
            .base_vertex_count
            .checked_add(delta.appended_vertices.len())?;
        let vertex_count_u32 = u32::try_from(vertex_count).ok()?;
        let live_index_count = u32::try_from(delta.live_index_count).ok()?;
        let base_triangles = delta.base_index_count / 3;
        let live_triangles = delta.live_index_count / 3;
        if !delta
            .updated_vertices
            .windows(2)
            .all(|pair| pair[0].vertex < pair[1].vertex)
            || delta
                .updated_vertices
                .iter()
                .any(|update| update.vertex as usize >= delta.base_vertex_count)
            || !delta
                .face_updates
                .windows(2)
                .all(|pair| pair[0].triangle < pair[1].triangle)
            || delta.face_updates.iter().any(|update| {
                update.triangle as usize >= live_triangles
                    || update
                        .indices
                        .iter()
                        .any(|&index| index as usize >= vertex_count)
            })
        {
            return None;
        }
        for triangle in base_triangles..live_triangles {
            if delta
                .face_updates
                .binary_search_by_key(&(triangle as u32), |update| update.triangle)
                .is_err()
            {
                return None;
            }
        }
        let draw_indices = usize::try_from(self.index_count)
            .ok()?
            .max(delta.live_index_count);
        let draw_index_count = u32::try_from(draw_indices).ok()?;
        let draw_triangles = draw_indices / 3;
        let vertex_bytes = vertex_count.checked_mul(size_of::<Vertex>())? as u64;
        let index_bytes = draw_indices.checked_mul(size_of::<u32>())? as u64;
        let wireframe_bytes = draw_triangles
            .checked_mul(6)?
            .checked_mul(size_of::<u32>())? as u64;

        Some(SculptDeltaLayout {
            vertex_count: vertex_count_u32,
            live_index_count,
            base_triangles,
            live_triangles,
            draw_index_count,
            vertex_bytes,
            index_bytes,
            wireframe_bytes,
            vertex_copy_bytes: base_vertex_count.checked_mul(size_of::<Vertex>())? as u64,
            index_copy_bytes: usize::try_from(self.index_count).ok()?.checked_mul(4)? as u64,
            wireframe_copy_bytes: usize::try_from(self.wireframe_index_count)
                .ok()?
                .checked_mul(4)? as u64,
            wireframe_index_count: u32::try_from(draw_triangles.checked_mul(6)?).ok()?,
            draw_wireframe: self.wireframe_index_buffer.is_some(),
        })
    }

    fn ensure_sculpt_delta_capacity(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &SculptDeltaLayout,
    ) -> Option<SculptBufferUpdateStats> {
        let mut stats = SculptBufferUpdateStats::default();
        let (grown, copied) = self.ensure_vertex_capacity(
            device,
            queue,
            layout.vertex_bytes,
            layout.vertex_copy_bytes,
        )?;
        stats.buffers_grown += grown;
        stats.bytes_copied += copied;
        let (grown, copied) =
            self.ensure_index_capacity(device, queue, layout.index_bytes, layout.index_copy_bytes)?;
        stats.buffers_grown += grown;
        stats.bytes_copied += copied;
        if layout.draw_wireframe {
            let (grown, copied) = self.ensure_wireframe_capacity(
                device,
                queue,
                layout.wireframe_bytes,
                layout.wireframe_copy_bytes,
            )?;
            stats.buffers_grown += grown;
            stats.bytes_copied += copied;
        }
        Some(stats)
    }

    fn upload_sculpt_delta(
        &mut self,
        queue: &wgpu::Queue,
        delta: &SculptTopologyDelta,
        layout: &SculptDeltaLayout,
        stats: &mut SculptBufferUpdateStats,
    ) {
        if !delta.appended_vertices.is_empty() {
            queue.write_buffer(
                &self.vertex_buffer,
                layout.vertex_copy_bytes,
                bytemuck::cast_slice(&delta.appended_vertices),
            );
            stats.bytes_written += layout.vertex_bytes - layout.vertex_copy_bytes;
        }
        stats.bytes_written +=
            write_vertex_updates(queue, &self.vertex_buffer, &delta.updated_vertices);
        stats.bytes_written += write_face_updates(queue, &self.index_buffer, &delta.face_updates);
        if layout.live_triangles < layout.base_triangles {
            stats.bytes_written += write_degenerate_faces(
                queue,
                &self.index_buffer,
                layout.live_triangles,
                layout.base_triangles - layout.live_triangles,
            );
        }
        stats.faces_written = u32::try_from(delta.face_updates.len())
            .unwrap_or(u32::MAX)
            .saturating_add(
                u32::try_from(layout.base_triangles.saturating_sub(layout.live_triangles))
                    .unwrap_or(u32::MAX),
            );
        if let Some(buffer) = self.wireframe_index_buffer.as_ref() {
            stats.bytes_written += write_wireframe_updates(queue, buffer, &delta.face_updates);
            if layout.live_triangles < layout.base_triangles {
                stats.bytes_written += write_degenerate_wireframe_faces(
                    queue,
                    buffer,
                    layout.live_triangles,
                    layout.base_triangles - layout.live_triangles,
                );
            }
            self.wireframe_index_count = layout.wireframe_index_count;
        }
    }

    pub(crate) fn write_sculpt_geometry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        geometry: SculptGeometry<'_>,
    ) -> Option<SculptBufferUpdateStats> {
        let SculptGeometry {
            vertices,
            indices,
            draw_wireframe,
        } = geometry;
        if indices.is_empty()
            || !indices.len().is_multiple_of(3)
            || indices
                .iter()
                .any(|&index| index as usize >= vertices.len())
        {
            return None;
        }
        let vertex_count = u32::try_from(vertices.len()).ok()?;
        let index_count = u32::try_from(indices.len()).ok()?;
        let vertex_bytes = vertices.len().checked_mul(size_of::<Vertex>())? as u64;
        let index_bytes = indices.len().checked_mul(4)? as u64;
        let wireframe_indices =
            draw_wireframe.then(|| wireframe_indices_for_triangle_mesh(indices));
        let wireframe_bytes = match wireframe_indices.as_ref() {
            Some(wire) => u64::try_from(wire.len().checked_mul(4)?).ok()?,
            None => 0,
        };
        let mut stats = SculptBufferUpdateStats::default();
        let (grown, copied) = self.ensure_vertex_capacity(device, queue, vertex_bytes, 0)?;
        stats.buffers_grown += grown;
        stats.bytes_copied += copied;
        let (grown, copied) = self.ensure_index_capacity(device, queue, index_bytes, 0)?;
        stats.buffers_grown += grown;
        stats.bytes_copied += copied;
        if let Some(wireframe_indices) = wireframe_indices.as_ref() {
            let (grown, copied) =
                self.ensure_wireframe_capacity(device, queue, wireframe_bytes, 0)?;
            stats.buffers_grown += grown;
            stats.bytes_copied += copied;
            let buffer = self.wireframe_index_buffer.as_ref()?;
            if !wireframe_indices.is_empty() {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(wireframe_indices));
                stats.bytes_written += wireframe_bytes;
            }
            self.wireframe_index_count = u32::try_from(wireframe_indices.len()).ok()?;
        } else {
            self.wireframe_index_count = 0;
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(vertices));
            stats.bytes_written += vertex_bytes;
        }
        if !indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(indices));
            stats.bytes_written += index_bytes;
        }
        stats.faces_written = u32::try_from(indices.len() / 3).unwrap_or(u32::MAX);
        self.vertex_count = vertex_count;
        self.live_index_count = index_count;
        self.index_count = index_count;
        Some(stats)
    }

    fn ensure_vertex_capacity(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        required_bytes: u64,
        copy_bytes: u64,
    ) -> Option<(u8, u64)> {
        if required_bytes <= self.vertex_capacity_bytes {
            return Some((0, 0));
        }
        let (buffer, capacity, copied) = grow_buffer(
            device,
            queue,
            &self.vertex_buffer,
            self.vertex_capacity_bytes,
            required_bytes,
            copy_bytes,
            "occluview vertex buffer",
            wgpu::BufferUsages::VERTEX,
        )?;
        self.vertex_buffer = buffer;
        self.vertex_capacity_bytes = capacity;
        Some((1, copied))
    }

    fn ensure_index_capacity(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        required_bytes: u64,
        copy_bytes: u64,
    ) -> Option<(u8, u64)> {
        if required_bytes <= self.index_capacity_bytes {
            return Some((0, 0));
        }
        let (buffer, capacity, copied) = grow_buffer(
            device,
            queue,
            &self.index_buffer,
            self.index_capacity_bytes,
            required_bytes,
            copy_bytes,
            "occluview index buffer",
            wgpu::BufferUsages::INDEX,
        )?;
        self.index_buffer = buffer;
        self.index_capacity_bytes = capacity;
        Some((1, copied))
    }

    fn ensure_wireframe_capacity(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        required_bytes: u64,
        copy_bytes: u64,
    ) -> Option<(u8, u64)> {
        let buffer = self.wireframe_index_buffer.as_ref()?;
        if required_bytes <= self.wireframe_index_capacity_bytes {
            return Some((0, 0));
        }
        let (buffer, capacity, copied) = grow_buffer(
            device,
            queue,
            buffer,
            self.wireframe_index_capacity_bytes,
            required_bytes,
            copy_bytes,
            "occluview wireframe index buffer",
            wgpu::BufferUsages::INDEX,
        )?;
        self.wireframe_index_buffer = Some(buffer);
        self.wireframe_index_capacity_bytes = capacity;
        Some((1, copied))
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

fn capacity_bytes(required: usize) -> u64 {
    u64::try_from(required.max(4))
        .unwrap_or(u64::MAX)
        .checked_next_power_of_two()
        .unwrap_or(u64::MAX)
}

#[allow(clippy::too_many_arguments)]
fn grow_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    current_capacity: u64,
    required_bytes: u64,
    copy_bytes: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
) -> Option<(wgpu::Buffer, u64, u64)> {
    let mut capacity = current_capacity.max(4);
    while capacity < required_bytes {
        capacity = capacity.checked_mul(2)?;
    }
    if capacity > device.limits().max_buffer_size {
        return None;
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: capacity,
        usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let copied = copy_bytes.min(current_capacity);
    if copied > 0 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview sculpt buffer growth"),
        });
        encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, copied);
        queue.submit(Some(encoder.finish()));
    }
    Some((buffer, capacity, copied))
}

fn write_face_updates(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    updates: &[super::offscreen::SculptFaceUpdate],
) -> u64 {
    let mut bytes_written = 0u64;
    let mut start = 0;
    while start < updates.len() {
        let mut end = start + 1;
        while end < updates.len() && updates[end].triangle == updates[end - 1].triangle + 1 {
            end += 1;
        }
        let faces: Vec<_> = updates[start..end]
            .iter()
            .map(|update| update.indices)
            .collect();
        queue.write_buffer(
            buffer,
            u64::from(updates[start].triangle) * 3 * size_of::<u32>() as u64,
            bytemuck::cast_slice(&faces),
        );
        bytes_written += faces.len() as u64 * 3 * size_of::<u32>() as u64;
        start = end;
    }
    bytes_written
}

fn write_vertex_updates(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    updates: &[super::offscreen::SculptVertexUpdate],
) -> u64 {
    let mut bytes_written = 0u64;
    let mut start = 0;
    while start < updates.len() {
        let mut end = start + 1;
        while end < updates.len() && updates[end].vertex == updates[end - 1].vertex + 1 {
            end += 1;
        }
        let values: Vec<_> = updates[start..end]
            .iter()
            .map(|update| update.value)
            .collect();
        queue.write_buffer(
            buffer,
            u64::from(updates[start].vertex) * size_of::<Vertex>() as u64,
            bytemuck::cast_slice(&values),
        );
        bytes_written += values.len() as u64 * size_of::<Vertex>() as u64;
        start = end;
    }
    bytes_written
}

fn write_degenerate_faces(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    first_triangle: usize,
    triangle_count: usize,
) -> u64 {
    if triangle_count == 0 {
        return 0;
    }
    let indices = vec![0u32; triangle_count * 3];
    queue.write_buffer(
        buffer,
        (first_triangle * 3 * size_of::<u32>()) as u64,
        bytemuck::cast_slice(&indices),
    );
    indices.len() as u64 * size_of::<u32>() as u64
}

fn write_wireframe_updates(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    updates: &[super::offscreen::SculptFaceUpdate],
) -> u64 {
    let mut bytes_written = 0u64;
    let mut start = 0;
    while start < updates.len() {
        let mut end = start + 1;
        while end < updates.len() && updates[end].triangle == updates[end - 1].triangle + 1 {
            end += 1;
        }
        let lines: Vec<[u32; 6]> = updates[start..end]
            .iter()
            .map(|update| {
                let [a, b, c] = update.indices;
                [a, b, b, c, c, a]
            })
            .collect();
        queue.write_buffer(
            buffer,
            u64::from(updates[start].triangle) * 6 * size_of::<u32>() as u64,
            bytemuck::cast_slice(&lines),
        );
        bytes_written += lines.len() as u64 * 6 * size_of::<u32>() as u64;
        start = end;
    }
    bytes_written
}

fn write_degenerate_wireframe_faces(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    first_triangle: usize,
    triangle_count: usize,
) -> u64 {
    if triangle_count == 0 {
        return 0;
    }
    let indices = vec![0u32; triangle_count * 6];
    queue.write_buffer(
        buffer,
        (first_triangle * 6 * size_of::<u32>()) as u64,
        bytemuck::cast_slice(&indices),
    );
    indices.len() as u64 * size_of::<u32>() as u64
}

fn wireframe_indices_for_triangle_mesh(indices: &[u32]) -> Vec<u32> {
    let mut lines = Vec::with_capacity(indices.len() * 2);
    for tri in indices.as_chunks::<3>().0 {
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

#[cfg(test)]
mod tests {
    use super::wireframe_indices_for_triangle_mesh;

    #[test]
    fn wireframe_indices_expand_triangles_to_line_pairs() {
        let indices = wireframe_indices_for_triangle_mesh(&[0, 1, 2, 2, 1, 3]);

        assert_eq!(indices, vec![0, 1, 1, 2, 2, 0, 2, 1, 1, 3, 3, 2]);
    }
}

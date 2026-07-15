//! Render pipeline: device + shader + pipeline + per-frame camera uniform.
//!
//! [`Renderer`] owns the long-lived GPU state (bind group layout, render
//! pipeline, camera uniform buffer). Per-frame you call [`Renderer::draw`]
//! inside a render pass.

use crate::camera::GpuCamera;
use crate::clipping::ClipPlane;
use crate::gpu::{camera_bind_layout, GpuMesh};
use crate::mesh_uniform::GpuMeshUniform;
use occluview_core::Vertex;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};

/// Shared latch for the most recent wgpu uncaptured error. wgpu's default
/// uncaptured-error handler PANICS, which is a hard process abort in a release
/// build (`panic = "abort"`) — a single driver hiccup or validation slip would
/// kill the app. We install a handler that records the message here instead;
/// the app polls [`Renderer::take_gpu_error`] and surfaces it honestly.
pub(crate) type GpuErrorLatch = Arc<Mutex<Option<String>>>;

/// Record a wgpu error into the latch (called from the device error handler).
/// A poisoned latch is ignored — recording an error must never itself panic.
pub(crate) fn record_gpu_error(latch: &GpuErrorLatch, message: String) {
    tracing::error!(gpu_error = %message, "wgpu reported an uncaptured GPU error");
    if let Ok(mut slot) = latch.lock() {
        *slot = Some(message);
    }
}

/// Take and clear the latched error. Returns `None` when empty or poisoned;
/// a poisoned latch must never block the UI poll.
pub(crate) fn drain_gpu_error(latch: &GpuErrorLatch) -> Option<String> {
    latch.lock().ok().and_then(|mut slot| slot.take())
}

const SHADER_SRC: &str = include_str!("../shaders/mesh.wgsl");
const CAP_SHADER_SRC: &str = include_str!("../shaders/cap.wgsl");
const POINT_SPLAT_VERTEX_COUNT: u32 = 6;
const DEFAULT_POINT_SPLAT_VIEWPORT: [f32; 2] = [1024.0, 768.0];

#[path = "pipeline_init.rs"]
mod init;

#[path = "pipeline_ghost.rs"]
mod ghost;

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

/// Vertex layout for the cap quad: position only (vec3<f32>).
fn cap_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    }
}

fn point_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x4,
                offset: 24,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 28,
                shader_location: 3,
            },
        ],
    }
}

/// Long-lived GPU state for the OccluView mesh pipeline.
pub struct Renderer {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
    pub(crate) pipeline: wgpu::RenderPipeline,
    /// Point-list pipeline for `MeshKind::PointCloud` rendering.
    pub(crate) point_pipeline: wgpu::RenderPipeline,
    pub(crate) transparent_pipeline: wgpu::RenderPipeline,
    pub(crate) transparent_point_pipeline: wgpu::RenderPipeline,
    pub(crate) wireframe_pipeline: wgpu::RenderPipeline,
    /// Cut-view ghost pipeline: re-draws the cut-away side translucent so a
    /// cross-section never fully removes geometry from the main viewport.
    pub(crate) ghost_pipeline: wgpu::RenderPipeline,
    pub(crate) camera_layout: wgpu::BindGroupLayout,
    pub(crate) camera_buffer: wgpu::Buffer,
    /// Layout for the per-mesh uniform (group 1): model matrix + tint +
    /// opacity + `has_texture` flag.
    pub(crate) mesh_layout: wgpu::BindGroupLayout,
    /// Layout for the texture + sampler (group 2).
    pub(crate) texture_layout: wgpu::BindGroupLayout,
    /// Layout for the clip plane (group 3): `ClipPlane` uniform.
    pub(crate) clip_layout: wgpu::BindGroupLayout,
    point_splat_viewport_width_bits: AtomicU32,
    point_splat_viewport_height_bits: AtomicU32,
    /// Cached disabled clip-plane buffer + bind group. Bound at group 3 for
    /// all draws that don't actually clip (thumbnails, plain renders) so the
    /// shader's `clip.enabled == 0` branch runs. Kept alive behind `dead_code`
    /// because the bind group borrows the buffer.
    #[allow(dead_code)]
    pub(crate) clip_buffer_disabled: wgpu::Buffer,
    pub(crate) clip_bind_group_disabled: wgpu::BindGroup,
    pub(crate) depth_format: wgpu::TextureFormat,
    // --- Stencil capping pipelines ---
    /// Pass 1: back faces increment stencil (cull Front, `color_write` none).
    pub(crate) stencil_back_pipeline: wgpu::RenderPipeline,
    /// Pass 2: front faces decrement stencil (cull Back, `color_write` none).
    pub(crate) stencil_front_pipeline: wgpu::RenderPipeline,
    /// Pass 3: cap polygon, stencil test NotEqual(0), flat color.
    pub(crate) cap_pipeline: wgpu::RenderPipeline,
    /// Cap uniform layout (group 1 of cap shader): a single vec4 color.
    pub(crate) cap_uniform_layout: wgpu::BindGroupLayout,
    sample_count: u32,
    /// Most recent wgpu uncaptured error, recorded by the device error handler.
    pub(crate) gpu_error: GpuErrorLatch,
}

impl Renderer {
    /// Update the camera uniform for the next frame.
    pub fn set_camera(&self, camera: &GpuCamera) {
        let mut camera = *camera;
        camera.pad0 = f32::from_bits(self.point_splat_viewport_width_bits.load(Ordering::Relaxed));
        camera.pad1 = f32::from_bits(
            self.point_splat_viewport_height_bits
                .load(Ordering::Relaxed),
        );
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera));
    }

    /// Set the viewport used to keep point-cloud splats bounded in pixel units.
    /// Takes effect on the next camera upload.
    pub fn set_point_splat_viewport(&self, width_px: u32, height_px: u32) {
        let width = (width_px as f32).max(1.0);
        let height = (height_px as f32).max(1.0);
        self.point_splat_viewport_width_bits
            .store(width.to_bits(), Ordering::Relaxed);
        self.point_splat_viewport_height_bits
            .store(height.to_bits(), Ordering::Relaxed);
    }

    /// Build the per-frame bind group binding the camera uniform at group 0.
    pub fn camera_bind_group(&self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview camera bind group"),
            layout: &self.camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.camera_buffer.as_entire_binding(),
            }],
        })
    }

    /// Create a uniform buffer + bind group for a per-mesh [`GpuMeshUniform`]
    /// (group 1). Callers write the uniform into the returned buffer via
    /// `queue.write_buffer` before each frame.
    pub fn mesh_uniform_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview mesh uniform"),
            size: size_of::<GpuMeshUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Build the per-mesh bind group binding a uniform buffer at group 1.
    pub fn mesh_bind_group(&self, uniform_buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview mesh bind group"),
            layout: &self.mesh_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        })
    }

    /// The texture bind group layout (group 2): a `texture_2d<f32>` at binding
    /// 0 and a `sampler` at binding 1. Exposed so callers can build bind groups
    /// against their own uploaded textures.
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_layout
    }

    /// The per-mesh uniform bind group layout (group 1). Exposed so callers
    /// can build per-mesh bind groups for multi-mesh scenes.
    pub fn mesh_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.mesh_layout
    }

    /// Create a uniform buffer + bind group for a [`ClipPlane`] (group 3).
    /// Caller writes the plane into the returned buffer before each frame.
    pub fn clip_uniform_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview clip plane uniform"),
            size: size_of::<ClipPlane>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Build the clip-plane bind group (group 3) bound to `uniform_buffer`.
    pub fn clip_bind_group(&self, uniform_buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview clip bind group"),
            layout: &self.clip_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        })
    }

    /// The cached disabled-clip bind group — bound at group 3 for draws that
    /// don't clip (thumbnails, plain renders). Use this instead of building a
    /// fresh group when `clip.enabled == 0`.
    pub fn disabled_clip_bind_group(&self) -> &wgpu::BindGroup {
        &self.clip_bind_group_disabled
    }

    /// Issue the draw for one mesh inside a render pass. Caller has already
    /// begun the pass against a color+depth view, set the camera, and will
    /// submit the encoder. Picks the triangle or point pipeline by `kind`.
    ///
    /// `mesh_bg` is the per-mesh uniform bind group (group 1); `texture_bg`
    /// is the texture+sampler bind group (group 2). For untextured meshes,
    /// pass a 1×1 white fallback texture bind group. `clip_bg` (group 3) is
    /// the clip-plane bind group — pass `disabled_clip_bind_group()` for no
    /// clipping.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: occluview_core::MeshKind,
    ) {
        self.draw_inner(
            rpass, camera_bg, mesh_bg, texture_bg, clip_bg, mesh, kind, false,
        );
    }

    /// Issue a transparent draw after opaque geometry has populated depth.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_transparent(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: occluview_core::MeshKind,
    ) {
        self.draw_inner(
            rpass, camera_bg, mesh_bg, texture_bg, clip_bg, mesh, kind, true,
        );
    }

    /// Issue a technical wireframe overlay draw for a triangle mesh.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_wireframe(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
    ) {
        rpass.set_pipeline(&self.wireframe_pipeline);
        rpass.set_bind_group(0, camera_bg, &[]);
        rpass.set_bind_group(1, mesh_bg, &[]);
        rpass.set_bind_group(2, texture_bg, &[]);
        rpass.set_bind_group(3, clip_bg, &[]);
        mesh.draw_wireframe(rpass);
    }

    /// Draw one triangle mesh as a translucent ghost of the cut-away side.
    ///
    /// Call this *after* the opaque draw has populated depth: the ghost is
    /// depth-tested but does not write depth, alpha-blended, and uses the
    /// inverted clip test baked into `fs_ghost`. `clip_bg` is the *same*
    /// clip-plane bind group as the opaque pass — the shader inverts the test
    /// internally, so no second uniform is needed.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ghost(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
    ) {
        rpass.set_pipeline(&self.ghost_pipeline);
        rpass.set_bind_group(0, camera_bg, &[]);
        rpass.set_bind_group(1, mesh_bg, &[]);
        rpass.set_bind_group(2, texture_bg, &[]);
        rpass.set_bind_group(3, clip_bg, &[]);
        mesh.draw(rpass, occluview_core::MeshKind::TriangleMesh);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_inner(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: occluview_core::MeshKind,
        transparent: bool,
    ) {
        let pipe = match (kind, transparent) {
            (occluview_core::MeshKind::TriangleMesh, false) => &self.pipeline,
            (occluview_core::MeshKind::PointCloud, false) => &self.point_pipeline,
            (occluview_core::MeshKind::TriangleMesh, true) => &self.transparent_pipeline,
            (occluview_core::MeshKind::PointCloud, true) => &self.transparent_point_pipeline,
        };
        rpass.set_pipeline(pipe);
        rpass.set_bind_group(0, camera_bg, &[]);
        rpass.set_bind_group(1, mesh_bg, &[]);
        rpass.set_bind_group(2, texture_bg, &[]);
        rpass.set_bind_group(3, clip_bg, &[]);
        match kind {
            occluview_core::MeshKind::TriangleMesh => mesh.draw(rpass, kind),
            occluview_core::MeshKind::PointCloud => {
                rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                rpass.draw(0..POINT_SPLAT_VERTEX_COUNT, 0..mesh.vertex_count);
            }
        }
    }

    /// Access the device (for buffer/texture creation by callers).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Access the queue (for buffer writes by callers).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Take the most recent wgpu uncaptured error, if any, clearing it.
    ///
    /// The app polls this once per frame and surfaces the message rather than
    /// letting wgpu's default panic-and-abort handler take the process down.
    /// Returns `None` if the latch is poisoned (a worker panicked mid-record);
    /// a poisoned latch never blocks the UI.
    pub fn take_gpu_error(&self) -> Option<String> {
        drain_gpu_error(&self.gpu_error)
    }

    /// Depth texture format used by this pipeline.
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// Render-pass sample count expected by this renderer's pipelines.
    #[must_use]
    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

fn multisample_state(sample_count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: sample_count.max(1),
        mask: !0,
        alpha_to_coverage_enabled: false,
    }
}

/// Bind group layout for the per-mesh uniform (group 1): one uniform buffer
/// visible to both vertex and fragment stages.
fn mesh_uniform_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview mesh uniform layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<GpuMeshUniform>() as u64),
            },
            count: None,
        }],
    })
}

/// Bind group layout for the clip plane (group 3): one uniform buffer
/// holding a [`ClipPlane`], visible to the fragment stage (where discard
/// happens) and vertex stage (future: vertex-side clip distances).
fn clip_plane_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview clip plane layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<ClipPlane>() as u64),
            },
            count: None,
        }],
    })
}

/// Bind group layout for the texture + sampler (group 2): a
/// `texture_2d<f32>` at binding 0 (fragment), a filtering sampler at binding
/// 1 (fragment).
fn texture_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview texture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

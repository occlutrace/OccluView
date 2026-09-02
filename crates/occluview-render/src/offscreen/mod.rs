//! Offscreen render-to-texture: used by the thumbnail worker and golden-image
//! tests. One render target + depth, one draw, read back as RGBA8.

use crate::error::RenderError;
use crate::gpu::GpuMesh;
use crate::mesh_uniform::GpuMeshUniform;
use crate::pipeline::Renderer;
use crate::texture::GpuTexture;
use occluview_core::{Mesh, MeshKind};
use std::time::{Duration, Instant};

mod helpers;
mod prepared_scene;
mod scene_render;
mod single_mesh;

/// Adapter-selection policy for one headless offscreen renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterPolicy {
    /// Verify a hardware device with a known-pixel probe, then fall back if it
    /// cannot provide a complete frame inside the caller's deadline.
    HardwareThenFallback,
    /// Use only the deterministic software adapter.
    FallbackOnly,
}

/// The adapter class that produced an offscreen renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterResult {
    /// A verified hardware adapter supplied the renderer.
    Hardware,
    /// The deterministic software fallback supplied the renderer.
    Fallback,
}

pub(crate) const fn adapter_result_for_device_type(device_type: wgpu::DeviceType) -> AdapterResult {
    match device_type {
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::DiscreteGpu => AdapterResult::Hardware,
        wgpu::DeviceType::Other | wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Cpu => {
            AdapterResult::Fallback
        }
    }
}

#[cfg(test)]
mod adapter_policy_tests {
    use super::*;

    #[test]
    fn cpu_and_virtual_adapters_are_not_accepted_as_verified_hardware() {
        assert_eq!(
            adapter_result_for_device_type(wgpu::DeviceType::Cpu),
            AdapterResult::Fallback
        );
        assert_eq!(
            adapter_result_for_device_type(wgpu::DeviceType::VirtualGpu),
            AdapterResult::Fallback
        );
    }

    #[test]
    fn fallback_policy_records_the_renderer_class_without_driver_metadata() {
        let result = pollster::block_on(Offscreen::new_with_adapter_policy(
            AdapterPolicy::FallbackOnly,
            RenderDeadline::after(Duration::from_secs(5)),
        ));
        let error = result.as_ref().err();
        assert!(
            error.is_none(),
            "the GPU test environment provides a fallback adapter: {error:?}"
        );
        let Some(offscreen) = result.ok() else { return };

        assert_eq!(offscreen.adapter_result(), AdapterResult::Fallback);
    }

    #[test]
    fn unbounded_deadline_has_no_poll_timeout() {
        assert!(matches!(
            RenderDeadline::unbounded().poll_timeout(),
            Ok(None)
        ));
    }
}

/// Absolute deadline supplied by the caller that owns an offscreen render.
///
/// Renderer consumers have different liveness contracts: an Explorer
/// thumbnail request has a small end-to-end budget, while a preview pane and
/// a desktop export have their own bounded operation. Keeping this deadline
/// explicit prevents one consumer from silently imposing its timeout on all
/// others.
#[derive(Clone, Copy, Debug)]
pub struct RenderDeadline {
    deadline: Instant,
    requested_timeout: Duration,
    unbounded: bool,
}

impl RenderDeadline {
    /// Create a deadline relative to the current instant.
    #[must_use]
    pub fn after(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            requested_timeout: timeout,
            unbounded: false,
        }
    }

    /// Wrap an existing absolute deadline.
    #[must_use]
    pub const fn at(deadline: Instant) -> Self {
        Self {
            deadline,
            requested_timeout: Duration::ZERO,
            unbounded: false,
        }
    }

    /// Create an operation with no elapsed-time budget.
    ///
    /// A Preview Pane first frame is a synchronous COM contract: its caller
    /// must receive either the painted bitmap or the render error, never a
    /// timer-derived substitute. This variant keeps that contract explicit
    /// while thumbnail and batch callers retain their bounded API.
    #[must_use]
    pub(crate) fn unbounded() -> Self {
        Self {
            deadline: Instant::now(),
            requested_timeout: Duration::ZERO,
            unbounded: true,
        }
    }

    /// Return the absolute instant at which this render request expires.
    ///
    /// Shell stream copies use this to share the render request's original
    /// budget instead of starting a separate I/O timeout.
    #[must_use]
    pub const fn expires_at(self) -> Instant {
        self.deadline
    }

    /// Return the remaining budget, or a structured timeout once it expires.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::ReadbackTimeout`] when this request's absolute
    /// deadline has elapsed.
    pub fn remaining(self) -> Result<Duration, RenderError> {
        if self.unbounded {
            Ok(Duration::MAX)
        } else {
            self.deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| self.timeout_error())
        }
    }

    pub(crate) fn poll_timeout(self) -> Result<Option<Duration>, RenderError> {
        if self.unbounded {
            Ok(None)
        } else {
            self.deadline
                .checked_duration_since(Instant::now())
                .map(Some)
                .ok_or_else(|| self.timeout_error())
        }
    }

    pub(crate) fn timeout_error(self) -> RenderError {
        RenderError::ReadbackTimeout {
            timeout: self.requested_timeout,
        }
    }
}

use helpers::make_fallback_texture_bind_group;

/// One entry in a multi-mesh offscreen scene draw: the mesh, its per-mesh
/// uniform, and an optional texture.
pub struct SceneDrawEntry<'a> {
    /// The CPU mesh to upload + draw.
    pub mesh: &'a Mesh,
    /// Per-mesh uniform (model, tint, opacity, `has_texture` flag).
    pub uniform: &'a GpuMeshUniform,
    /// Texture to sample; if `None`, the fallback 1×1 white texture is used.
    pub texture: Option<&'a GpuTexture>,
}

/// CPU-side source for a prepared multi-mesh scene.
pub struct PreparedSceneSource<'a> {
    /// The CPU mesh to upload once into GPU buffers.
    pub mesh: &'a Mesh,
    /// Initial per-mesh uniform.
    pub uniform: GpuMeshUniform,
    /// Whether this layer should draw.
    pub visible: bool,
    /// Whether to draw a technical wireframe overlay for this layer.
    pub wireframe: bool,
}

/// Per-frame material/visibility update for a prepared scene.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSceneUpdate {
    /// Topology identity expected for this prepared entry.
    pub topology: PreparedSceneTopology,
    /// Updated per-mesh uniform.
    pub uniform: GpuMeshUniform,
    /// Whether this layer should draw.
    pub visible: bool,
    /// Whether to draw a technical wireframe overlay for this layer.
    pub wireframe: bool,
}

/// GPU-uploaded topology identity for one prepared scene entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedSceneTopology {
    mesh_topology_id: u64,
    kind: MeshKind,
    vertex_count: usize,
    index_count: usize,
    has_texture: bool,
}

impl PreparedSceneTopology {
    /// Build a topology token from the CPU mesh payload.
    #[must_use]
    pub fn from_mesh(mesh: &Mesh) -> Self {
        Self {
            mesh_topology_id: mesh.topology_id(),
            kind: mesh.kind(),
            vertex_count: mesh.vertices().len(),
            index_count: mesh.indices().len(),
            has_texture: mesh.texture().is_some(),
        }
    }
}

/// A multi-mesh scene uploaded once to GPU memory.
pub struct PreparedScene {
    entries: Vec<PreparedSceneEntry>,
}

struct PreparedSceneEntry {
    mesh: GpuMesh,
    uniform_buffer: wgpu::Buffer,
    mesh_bind_group: wgpu::BindGroup,
    texture: Option<GpuTexture>,
    kind: MeshKind,
    topology: PreparedSceneTopology,
    opacity: f32,
    visible: bool,
    wireframe: bool,
}

/// Parameters for an offscreen render.
#[derive(Clone, Copy, Debug)]
pub struct ThumbnailSpec {
    /// Square output dimension in pixels.
    pub size_px: u16,
    /// Background color (linear RGBA). Default is transparent.
    pub background: [f64; 4],
}

impl Default for ThumbnailSpec {
    fn default() -> Self {
        Self {
            size_px: 256,
            background: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Parameters for an interactive rectangular viewport render.
#[derive(Clone, Copy, Debug)]
pub struct ViewportSpec {
    /// Output dimensions in pixels: `[width, height]`.
    pub size_px: [u16; 2],
    /// Background color (linear RGBA).
    pub background: [f64; 4],
}

/// Inputs for a prepared-scene render with one hard clipping plane.
#[derive(Clone, Copy)]
pub struct PreparedSceneClipRequest<'a> {
    /// GPU scene prepared by this [`Offscreen`] instance.
    pub scene: &'a PreparedScene,
    /// Frame camera.
    pub camera: &'a crate::camera::GpuCamera,
    /// Hard clipping plane for the frame.
    pub clip: &'a crate::clipping::ClipPlane,
    /// Square target size and background.
    pub spec: ThumbnailSpec,
    /// Caller-owned budget for the entire render and readback.
    pub deadline: RenderDeadline,
}

/// Inputs for a prepared scene rendered into an app viewport.
#[derive(Clone, Copy)]
pub struct PreparedViewportRequest<'a> {
    /// GPU scene prepared by this [`Offscreen`] instance.
    pub scene: &'a PreparedScene,
    /// Optional prepared selection overlay.
    pub overlay: Option<&'a PreparedScene>,
    /// Frame camera.
    pub camera: &'a crate::camera::GpuCamera,
    /// Rectangular target size and background.
    pub spec: ViewportSpec,
    /// Caller-owned budget for the entire render and readback.
    pub deadline: RenderDeadline,
}

/// Inputs for a prepared viewport render that also draws the cut-away ghost.
#[derive(Clone, Copy)]
pub struct PreparedViewportClipRequest<'a> {
    /// GPU scene prepared by this [`Offscreen`] instance.
    pub scene: &'a PreparedScene,
    /// Optional prepared selection overlay.
    pub overlay: Option<&'a PreparedScene>,
    /// Frame camera.
    pub camera: &'a crate::camera::GpuCamera,
    /// Clip plane shared with the interactive viewport.
    pub clip: &'a crate::clipping::ClipPlane,
    /// Rectangular target size and background.
    pub spec: ViewportSpec,
    /// Whether the cut-away ghost pass may draw (the operator's preference;
    /// the clip plane itself stays authoritative for when a section exists).
    pub show_ghost: bool,
    /// Caller-owned budget for the entire render and readback.
    pub deadline: RenderDeadline,
}

/// Inputs for rendering one mesh with a hard clipping plane.
#[derive(Clone, Copy)]
pub struct ClippedMeshRequest<'a> {
    /// CPU mesh uploaded for this frame.
    pub mesh: &'a Mesh,
    /// Frame camera.
    pub camera: &'a crate::camera::GpuCamera,
    /// Hard clipping plane for the frame.
    pub clip: &'a crate::clipping::ClipPlane,
    /// Square target size and background.
    pub spec: ThumbnailSpec,
    /// Caller-owned budget for the entire render and readback.
    pub deadline: RenderDeadline,
}

/// Inputs for rendering one mesh with a solid cross-section cut.
#[derive(Clone, Copy)]
pub struct CutMeshRequest<'a> {
    /// CPU mesh uploaded for this frame.
    pub mesh: &'a Mesh,
    /// Frame camera.
    pub camera: &'a crate::camera::GpuCamera,
    /// Cut plane and cap policy.
    pub cut: &'a crate::clipping::CutViewSpec,
    /// Half extent of the generated cap quad in mesh coordinates.
    pub half_extent: f32,
    /// Square target size and background.
    pub spec: ThumbnailSpec,
    /// Caller-owned budget for the entire render and readback.
    pub deadline: RenderDeadline,
}

/// Offscreen renderer. Wraps a headless [`Renderer`].
pub struct Offscreen {
    renderer: Renderer,
    adapter_result: AdapterResult,
    /// Cached identity mesh bind group (group 1). The thumbnail path renders
    /// one mesh at the origin, so the model matrix is identity. The uniform
    /// buffer behind it is owned by the bind group and never read back.
    mesh_bind_group: wgpu::BindGroup,
    /// Cached 1x1 white fallback texture + bind group (group 2). The thumbnail
    /// path uses vertex colors (no texture), but the pipeline requires a bound
    /// group-2 resource.
    texture_bind_group: wgpu::BindGroup,
}

impl Offscreen {
    /// Create a headless renderer at any reasonable output format.
    ///
    /// # Errors
    /// Returns [`RenderError::NoAdapter`] if no GPU/adapter is available
    /// (including under WARP-less sandboxes).
    #[allow(clippy::unused_async)]
    pub async fn new() -> Result<Self, RenderError> {
        let renderer = Renderer::new_headless(wgpu::TextureFormat::Rgba8Unorm).await?;
        Ok(Self::from_renderer(renderer, AdapterResult::Fallback))
    }

    /// Create and verify the preferred adapter without a caller timeout.
    ///
    /// This is the synchronous Preview Pane path. It deliberately shares the
    /// same hardware-then-fallback policy as bounded callers, but never turns
    /// an unfinished first frame into a timer-derived result.
    ///
    /// # Errors
    ///
    /// Returns `RenderError::NoAdapter` when neither a verified hardware adapter
    /// nor the fallback adapter can be created.
    pub async fn new_prefer_hardware() -> Result<Self, RenderError> {
        Self::new_with_adapter_policy(
            AdapterPolicy::HardwareThenFallback,
            RenderDeadline::unbounded(),
        )
        .await
    }

    /// Create an offscreen renderer with a verified, caller-selected policy.
    ///
    /// # Errors
    /// Returns the fallback error when no verified adapter can produce a
    /// renderer inside `deadline`.
    pub async fn new_with_adapter_policy(
        policy: AdapterPolicy,
        deadline: RenderDeadline,
    ) -> Result<Self, RenderError> {
        match policy {
            AdapterPolicy::FallbackOnly => {
                Self::new_on_adapter(AdapterResult::Fallback, deadline).await
            }
            AdapterPolicy::HardwareThenFallback => {
                if let Ok(hardware) = Self::new_on_adapter(AdapterResult::Hardware, deadline).await
                {
                    if hardware.can_draw_with_deadline(deadline).await
                        && deadline.remaining().is_ok()
                    {
                        return Ok(hardware);
                    }
                }
                tracing::warn!(
                    "hardware offscreen renderer could not provide a verified frame; using fallback"
                );
                Self::new_on_adapter(AdapterResult::Fallback, deadline).await
            }
        }
    }

    async fn new_on_adapter(
        adapter_result: AdapterResult,
        deadline: RenderDeadline,
    ) -> Result<Self, RenderError> {
        let _ = deadline.remaining()?;
        let (renderer, actual_result) = Renderer::new_headless_on_adapter(
            wgpu::TextureFormat::Rgba8Unorm,
            matches!(adapter_result, AdapterResult::Fallback),
        )
        .await?;
        if actual_result != adapter_result {
            return Err(RenderError::NoAdapter);
        }
        let _ = deadline.remaining()?;
        Ok(Self::from_renderer(renderer, adapter_result))
    }

    fn from_renderer(renderer: Renderer, adapter_result: AdapterResult) -> Self {
        let device = renderer.device();
        let queue = renderer.queue();

        let mesh_uniform_buffer = renderer.mesh_uniform_buffer();
        queue.write_buffer(
            &mesh_uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuMeshUniform::identity()),
        );
        let mesh_bind_group = renderer.mesh_bind_group(&mesh_uniform_buffer);
        let texture_bind_group = make_fallback_texture_bind_group(device, queue, &renderer);

        Self {
            renderer,
            adapter_result,
            mesh_bind_group,
            texture_bind_group,
        }
    }

    /// Draw one known triangle and report whether any of it arrived.
    ///
    /// An adapter can accept every command and then produce an empty target:
    /// a virtual display driver, a headless server's stub, a runner's nominal
    /// GPU. Nothing reports an error, so the only way to tell is to draw
    /// something and look. Callers that prefer hardware use this to demote a
    /// device that cannot draw, before it is handed a scan and answers with a
    /// blank picture.
    #[must_use]
    pub async fn can_draw_with_deadline(&self, deadline: RenderDeadline) -> bool {
        use glam::Vec3;
        use occluview_core::{MeshBuilder, Vertex};

        let mut builder = MeshBuilder::new();
        let a = builder.push_vertex(Vertex::at(Vec3::new(-0.8, -0.8, 0.0)).with_normal(Vec3::Z));
        let b = builder.push_vertex(Vertex::at(Vec3::new(0.8, -0.8, 0.0)).with_normal(Vec3::Z));
        let c = builder.push_vertex(Vertex::at(Vec3::new(0.0, 0.8, 0.0)).with_normal(Vec3::Z));
        builder.push_triangle(a, b, c);
        let Ok(mesh) = builder.build() else {
            return false;
        };
        let camera = crate::GpuCamera::new(
            glam::Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y),
            glam::Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 3.0),
        );
        let spec = ThumbnailSpec {
            size_px: 16,
            ..ThumbnailSpec::default()
        };
        self.render_with_deadline(&mesh, &camera, spec, deadline)
            .await
            .is_ok_and(|pixels| pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0))
    }

    /// Draw the adapter verification frame without a caller timeout.
    #[must_use]
    pub async fn can_draw(&self) -> bool {
        self.can_draw_with_deadline(RenderDeadline::unbounded())
            .await
    }

    /// Access the underlying renderer (for callers that need device/queue).
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Return the verified adapter class without exposing host driver details.
    #[must_use]
    pub const fn adapter_result(&self) -> AdapterResult {
        self.adapter_result
    }

    /// Upload a multi-mesh scene once so camera-only redraws can reuse GPU
    /// buffers instead of re-uploading vertices, indices, and textures.
    #[must_use]
    pub fn prepare_scene(&self, sources: &[PreparedSceneSource<'_>]) -> PreparedScene {
        PreparedScene::upload(&self.renderer, sources)
    }

    /// The per-mesh uniform bind group layout (group 1). Exposed so callers
    /// can build per-mesh bind groups for multi-mesh scenes.
    pub fn mesh_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.renderer.mesh_bind_group_layout()
    }
}

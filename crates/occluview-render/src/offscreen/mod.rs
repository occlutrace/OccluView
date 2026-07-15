//! Offscreen render-to-texture: used by the thumbnail worker and golden-image
//! tests. One render target + depth, one draw, read back as RGBA8.

use crate::error::RenderError;
use crate::gpu::GpuMesh;
use crate::mesh_uniform::GpuMeshUniform;
use crate::pipeline::Renderer;
use crate::texture::GpuTexture;
use occluview_core::{Mesh, MeshKind};

mod helpers;
mod prepared_scene;
mod scene_render;
mod single_mesh;

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

/// Offscreen renderer. Wraps a headless [`Renderer`].
pub struct Offscreen {
    renderer: Renderer,
    /// Cached identity mesh uniform + bind group (group 1). The thumbnail path
    /// renders one mesh at the origin, so the model matrix is identity.
    mesh_uniform_buffer: wgpu::Buffer,
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
        Ok(Self::from_renderer(renderer))
    }

    /// Create a headless renderer for the interactive desktop viewer, preferring
    /// a hardware adapter before falling back.
    ///
    /// # Errors
    /// Returns [`RenderError::NoAdapter`] if no compatible adapter is available.
    #[allow(clippy::unused_async)]
    pub async fn new_prefer_hardware() -> Result<Self, RenderError> {
        let renderer =
            Renderer::new_headless_prefer_hardware(wgpu::TextureFormat::Rgba8Unorm).await?;
        Ok(Self::from_renderer(renderer))
    }

    fn from_renderer(renderer: Renderer) -> Self {
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
            mesh_uniform_buffer,
            mesh_bind_group,
            texture_bind_group,
        }
    }

    /// Access the underlying renderer (for callers that need device/queue).
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Upload a multi-mesh scene once so camera-only redraws can reuse GPU
    /// buffers instead of re-uploading vertices, indices, and textures.
    #[must_use]
    pub fn prepare_scene(&self, sources: &[PreparedSceneSource<'_>]) -> PreparedScene {
        PreparedScene::upload(&self.renderer, sources)
    }

    /// Access the cached fallback texture bind group (group 2). Useful for
    /// multi-mesh draws where some meshes are untextured.
    pub fn fallback_texture_bind_group(&self) -> &wgpu::BindGroup {
        &self.texture_bind_group
    }

    /// Access the cached identity mesh uniform buffer. Useful for building
    /// additional bind groups.
    pub fn identity_uniform_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_uniform_buffer
    }

    /// The per-mesh uniform bind group layout (group 1). Exposed so callers
    /// can build per-mesh bind groups for multi-mesh scenes.
    pub fn mesh_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.renderer.mesh_bind_group_layout()
    }
}

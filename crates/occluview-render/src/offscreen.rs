//! Offscreen render-to-texture: used by the thumbnail worker and golden-image
//! tests. One render target + depth, one draw, read back as RGBA8.

use crate::camera::GpuCamera;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use crate::mesh_uniform::GpuMeshUniform;
use crate::pipeline::Renderer;
use occluview_core::Mesh;

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
            background: [0.039, 0.039, 0.039, 1.0], // OccluTrace dark, opaque
        }
    }
}

/// Offscreen renderer. Wraps a headless [`Renderer`].
pub struct Offscreen {
    renderer: Renderer,
    /// Cached identity mesh uniform + bind group (group 1). The thumbnail path
    /// renders one mesh at the origin, so the model matrix is identity.
    mesh_uniform_buffer: wgpu::Buffer,
    mesh_bind_group: wgpu::BindGroup,
    /// Cached 1×1 white fallback texture + bind group (group 2). The thumbnail
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
        let device = renderer.device();
        let queue = renderer.queue();

        // Identity mesh uniform (group 1).
        let mesh_uniform_buffer = renderer.mesh_uniform_buffer();
        queue.write_buffer(
            &mesh_uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuMeshUniform::identity()),
        );
        let mesh_bind_group = renderer.mesh_bind_group(&mesh_uniform_buffer);

        // 1×1 white fallback texture + sampler (group 2).
        let texture_bind_group = make_fallback_texture_bind_group(device, queue, &renderer);

        Ok(Self {
            renderer,
            mesh_uniform_buffer,
            mesh_bind_group,
            texture_bind_group,
        })
    }

    /// Render `mesh` with the given camera into an RGBA8 buffer.
    ///
    /// Returns a flat `Vec<u8>` of length `size_px * size_px * 4` in row-major
    /// order, top-to-bottom (after the y-flip wgpu requires for offscreen).
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async)]
    pub async fn render(
        &self,
        mesh: &Mesh,
        camera: &GpuCamera,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        let gpu_mesh = GpuMesh::upload(device, queue, mesh);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview offscreen encoder"),
        });
        let targets = RenderTargets {
            color: &color_view,
            depth: &depth_view,
        };
        self.encode_pass(
            &mut encoder,
            &targets,
            &camera_bg,
            &self.mesh_bind_group,
            &self.texture_bind_group,
            &gpu_mesh,
            mesh.kind(),
            spec.background,
        );

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview offscreen readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            extent(size),
        );
        queue.submit(std::iter::once(encoder.finish()));

        Ok(self.read_back(&output_buffer, padded, spec.size_px))
    }

    /// Access the underlying renderer (for callers that need device/queue).
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
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

    /// Begin the offscreen render pass against `targets` and draw `mesh`.
    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: occluview_core::MeshKind,
        background: [f64; 4],
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview offscreen pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: targets.color,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background[0],
                        g: background[1],
                        b: background[2],
                        a: background[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.renderer
            .draw(&mut rpass, camera_bg, mesh_bg, texture_bg, mesh, kind);
    }

    fn read_back(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: u16,
    ) -> Vec<u8> {
        let slice = output_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.renderer.device().poll(wgpu::Maintain::Wait);

        let row_bytes = usize::from(size_px) * 4;
        let pixels = {
            let data = slice.get_mapped_range();
            let mut out = Vec::with_capacity(row_bytes * usize::from(size_px));
            for row in 0..usize::from(size_px) {
                let start = row * padded_bytes_per_row as usize;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
            out
        };
        output_buffer.unmap();

        // wgpu renders bottom-to-top; flip to top-to-bottom for consumers
        // (PNG encoders, HBITMAP interop).
        let mut flipped = Vec::with_capacity(pixels.len());
        for row in (0..usize::from(size_px)).rev() {
            flipped.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
        }
        flipped
    }
}

/// Color + depth views grouped so `encode_pass` takes one argument.
struct RenderTargets<'a> {
    color: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
}

/// Build a 1×1 white `Rgba8Unorm` texture + linear sampler + bind group for
/// group 2. Used as the bound-texture fallback for untextured meshes (the
/// shader's `has_texture=0` branch never samples it, but the pipeline layout
/// requires the binding).
fn make_fallback_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Renderer,
) -> wgpu::BindGroup {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview fallback white texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255, 255, 255, 255],
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("occluview fallback sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("occluview fallback texture bind group"),
        layout: renderer.texture_layout(),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&tex_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

fn make_color_target(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen color"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn make_depth_target(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen depth"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn extent(size: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    }
}

/// wgpu requires buffer rows to be aligned to 256 bytes. RGBA8 = 4 bytes/pixel.
fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(256) * 256
}

//! Offscreen render-to-texture: used by the thumbnail worker and golden-image
//! tests. One render target + depth, one draw, read back as RGBA8.

use crate::camera::GpuCamera;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
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
        Ok(Self { renderer })
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
        let bind_group = self.renderer.camera_bind_group();

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
            &bind_group,
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

    /// Begin the offscreen render pass against `targets` and draw `mesh`.
    /// The five arguments are distinct concepts (encoder, target views, bind
    /// group, mesh, clear color) that don't share a natural grouping beyond
    /// `RenderTargets`, hence the local allow.
    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets<'_>,
        bind_group: &wgpu::BindGroup,
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
        self.renderer.draw(&mut rpass, bind_group, mesh, kind);
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

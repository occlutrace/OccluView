use super::{Offscreen, Renderer};
use crate::error::RenderError;
use std::sync::mpsc;
use wgpu::TextureView;

pub(super) struct RenderTargets<'a> {
    pub(super) color: &'a TextureView,
    pub(super) depth: &'a TextureView,
}

pub(super) fn make_fallback_texture_bind_group(
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

pub(super) fn make_color_target(device: &wgpu::Device, size: u32) -> (wgpu::Texture, TextureView) {
    make_color_target_extent(device, size, size)
}

pub(super) fn make_color_target_extent(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen color"),
        size: extent_rect(width, height),
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

pub(super) fn make_depth_target(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, TextureView) {
    make_depth_target_extent(device, size, size, format)
}

pub(super) fn make_depth_target_extent(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen depth"),
        size: extent_rect(width, height),
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

pub(super) fn extent(size: u32) -> wgpu::Extent3d {
    extent_rect(size, size)
}

pub(super) fn extent_rect(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

pub(super) fn is_transparent(opacity: f32) -> bool {
    opacity < 0.999
}

pub(super) fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(256) * 256
}

impl Offscreen {
    pub(super) fn read_back(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: u16,
    ) -> Result<Vec<u8>, RenderError> {
        self.read_back_extent(output_buffer, padded_bytes_per_row, [size_px, size_px])
    }

    pub(super) fn read_back_extent(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: [u16; 2],
    ) -> Result<Vec<u8>, RenderError> {
        let [width_px, height_px] = size_px;
        let slice = output_buffer.slice(..);
        let (map_tx, map_rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = map_tx.send(result.map_err(|error| error.to_string()));
        });
        let _ = self.renderer.device().poll(wgpu::Maintain::Wait);
        map_rx
            .recv()
            .map_err(|error| {
                RenderError::Surface(format!("offscreen readback callback dropped: {error}"))
            })?
            .map_err(|error| RenderError::Surface(format!("offscreen readback failed: {error}")))?;

        let row_bytes = usize::from(width_px) * 4;
        let row_count = usize::from(height_px);
        let pixels = {
            let data = slice.get_mapped_range();
            let mut out = Vec::with_capacity(row_bytes * row_count);
            for row in 0..row_count {
                let start = row * padded_bytes_per_row as usize;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
            out
        };
        output_buffer.unmap();

        let mut flipped = Vec::with_capacity(pixels.len());
        for row in (0..row_count).rev() {
            flipped.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
        }
        Ok(flipped)
    }
}

//! GPU texture upload: decodes a CPU-side [`MeshTexture`] into a `wgpu::Texture`
//! + view + sampler + bind group, ready to bind at group 2.

use crate::pipeline::Renderer;
use occluview_core::MeshTexture;

/// A texture resident on the GPU: the `wgpu::Texture`, its view, a sampler,
/// and the bind group (group 2) that binds them at bindings 0 and 1.
pub struct GpuTexture {
    /// Owns the GPU memory; kept alive so the view and sampler stay valid.
    #[allow(dead_code)]
    pub(crate) texture: wgpu::Texture,
    /// Kept alive so the bind group's view binding remains valid.
    #[allow(dead_code)]
    pub(crate) view: wgpu::TextureView,
    /// Kept alive so the bind group's sampler binding remains valid.
    #[allow(dead_code)]
    pub(crate) sampler: wgpu::Sampler,
    /// Bind group (group 2): binding 0 = view, binding 1 = sampler.
    pub bind_group: wgpu::BindGroup,
}

impl GpuTexture {
    /// Build a 1x1 white texture used when a mesh has no material texture.
    ///
    /// The mesh shader still requires a bound group-2 texture/sampler even
    /// when `has_texture == 0`. This is the live path's fallback; the
    /// offscreen path needs only the bind group and builds an identical 1x1
    /// white one in `offscreen::helpers::make_fallback_texture_bind_group`.
    #[must_use]
    pub fn fallback(renderer: &Renderer, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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
                texture: &texture,
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("occluview fallback sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview fallback texture bind group"),
            layout: renderer.texture_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            texture,
            view,
            sampler,
            bind_group,
        }
    }

    /// Upload a CPU-side [`MeshTexture`] to the GPU and build the group-2 bind
    /// group. Uses linear filtering and clamp-to-edge wrapping — the sane
    /// defaults for dental mesh textures.
    #[must_use]
    pub fn upload(
        renderer: &Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex: &MeshTexture,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("occluview mesh texture"),
            size: wgpu::Extent3d {
                width: tex.width,
                height: tex.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // `Rgba8Unorm`, NOT `Rgba8UnormSrgb`.
            //
            // The whole pipeline writes into an `Rgba8Unorm` target and encodes
            // nothing on the way out, so whatever the shader returns is what
            // the display treats as sRGB. Vertex colours reach the shader as
            // `byte / 255`, i.e. already in that space. An sRGB-typed texture
            // would make `textureSample` decode to linear, and that value would
            // then be written out as if it were sRGB -- the same nominal colour
            // arriving darker through a texture than through a vertex.
            //
            // Same triangle, same light: sRGB 128 rendered as 70 from a
            // texture against 129 from a vertex colour, and sRGB 200 as 159
            // against 198. That is the flagship formats (HPS and GLB, colour in
            // a texture) and the open ones (PLY and OBJ, colour in vertices) on
            // two different scales, in a viewer whose job includes judging
            // colour.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &tex.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(tex.width * 4),
                rows_per_image: Some(tex.height),
            },
            wgpu::Extent3d {
                width: tex.width,
                height: tex.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("occluview mesh sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview mesh texture bind group"),
            layout: renderer.texture_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            texture,
            view,
            sampler,
            bind_group,
        }
    }
}

#[cfg(test)]
mod tests {
    /// The part of this file above the test module.
    ///
    /// Searching the whole of it matches the needle written in the assertion
    /// itself, so the guard would pass on its own text and the production line
    /// it names could be deleted with nothing going red.
    fn production_source() -> &'static str {
        let source = include_str!("texture.rs");
        source
            .split_once("#[cfg(test)]\nmod tests")
            .map_or(source, |(production, _)| production)
    }

    #[test]
    fn mesh_texture_sampler_clamps_uv_edges() {
        let source = production_source();
        let start = source.find("label: Some(\"occluview mesh sampler\")");
        assert!(start.is_some(), "missing mesh sampler");
        let Some(start) = start else {
            return;
        };
        let end = source[start..].find("mipmap_filter: wgpu::FilterMode::Nearest");
        assert!(end.is_some(), "missing mesh sampler mipmap filter");
        let Some(end) = end else {
            return;
        };
        let sampler = &source[start..start + end];

        assert!(
            sampler.contains("address_mode_u: wgpu::AddressMode::ClampToEdge")
                && sampler.contains("address_mode_v: wgpu::AddressMode::ClampToEdge")
                && sampler.contains("address_mode_w: wgpu::AddressMode::ClampToEdge"),
            "scan textures should clamp at UV borders instead of wrapping unrelated texture pixels"
        );
        assert!(
            !sampler.contains("address_mode_u: wgpu::AddressMode::Repeat"),
            "Repeat sampling causes HPS edge/packed-UV color artifacts"
        );
    }
}

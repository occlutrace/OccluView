//! The group-2 material for a layer that paints an occlusal contact field.
//!
//! The field is a signed distance per vertex (positive gap, zero touch,
//! negative penetration), packed one `f32` bit pattern per texel into an
//! `Rgba8Unorm` texture that the vertex stage loads by vertex index. Group 2 is
//! shared with the scan's own material, so this module also owns the *inert*
//! field texel every other group-2 bind group has to bind: the layout has three
//! bindings and wgpu requires all of them, whether or not the layer paints
//! contacts.
//!
//! # Why the ramp is not here
//!
//! There is no ramp texture. The colour ramp lives in the per-mesh uniform as a
//! stop table ([`crate::GpuMeshUniform::set_contact_paint`]) and is evaluated in
//! the fragment shader, which makes the one control the operator drives — the
//! depth that reads as fully loaded — a uniform write rather than a texture
//! upload and a bind-group rebuild. Nothing about a contact reading changes per
//! frame except that number.

use crate::pipeline::Renderer;
use crate::texture::GpuTexture;

/// Bytes per texel of the packed field texture (`Rgba8Unorm`).
const FIELD_TEXEL_BYTES: usize = 4;

/// The signed field value used for a vertex with no opposing surface.
///
/// The kernel's own sentinel is `+inf`, which must not reach a shader: an
/// infinite value in an interpolated varying produces `0 * inf = NaN` in the
/// fragment stage, and a NaN alpha composites as black or as nothing depending
/// on the driver. Half a millimetre is far past every law's paint edge (the
/// widest is the clinical `0.2 mm`), so a sentinel vertex is unpainted exactly
/// as an unreachable one is, through ordinary arithmetic.
pub const FIELD_FAR_SENTINEL_MM: f32 = 0.5;

/// A packed signed contact field, ready to upload: `field.width * field.height`
/// texels of four bytes, each holding the little-endian bit pattern of one
/// `f32` in millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactFieldTexels {
    /// `width * height * 4` bytes, row-major, one little-endian `f32` per texel.
    pub rgba: Vec<u8>,
    /// Texels per row. The shader turns a vertex index into `(index % width,
    /// index / width)`, so this is the row stride, not the vertex count.
    pub width: u32,
    /// Texel rows. `width * height` may exceed the vertex count; the extra
    /// texels are padding and are never addressed.
    pub height: u32,
}

impl ContactFieldTexels {
    /// Build a field from its packed bytes.
    ///
    /// Returns `None` when a dimension is zero or `rgba` is not exactly
    /// `width * height * 4` bytes. A short buffer is not repaired: every texel
    /// after it would decode as some other vertex's distance, painting a
    /// plausible but wrong reading, and a wrong measurement is worse than an
    /// absent one.
    #[must_use]
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = usize::try_from(width).ok()? * usize::try_from(height).ok()?;
        let expected = expected.checked_mul(FIELD_TEXEL_BYTES)?;
        if rgba.len() != expected {
            return None;
        }
        Some(Self {
            rgba,
            width,
            height,
        })
    }

    /// Pack a signed field, one `f32` per vertex, into a texture no wider than
    /// `max_width`.
    ///
    /// Rows are padded to `max_width` with the far sentinel, because the shader
    /// addresses the texture as `(index % width, index / width)` and a ragged
    /// last row would otherwise shift every vertex after it. A non-finite value
    /// is packed as the sentinel rather than as its own bit pattern: see
    /// [`FIELD_FAR_SENTINEL_MM`].
    #[must_use]
    pub fn from_values(values: &[f32], max_width: u32) -> Option<Self> {
        if values.is_empty() || max_width == 0 {
            return None;
        }
        let width = max_width.min(u32::try_from(values.len()).ok()?);
        let height = u32::try_from(values.len()).ok()?.div_ceil(width);
        let texels = usize::try_from(width).ok()? * usize::try_from(height).ok()?;
        let mut rgba = Vec::with_capacity(texels * FIELD_TEXEL_BYTES);
        for index in 0..texels {
            let value = values.get(index).copied().unwrap_or(FIELD_FAR_SENTINEL_MM);
            let value = if value.is_finite() {
                value
            } else {
                FIELD_FAR_SENTINEL_MM
            };
            rgba.extend_from_slice(&value.to_le_bytes());
        }
        Self::new(rgba, width, height)
    }

    /// The 1×1 field that carries the sentinel for a layer with no contacts.
    ///
    /// Bound wherever a group-2 bind group exists without a field, so the
    /// shader's `textureLoad` — which never runs for such a layer, because
    /// `contact_map == 0` — would still find a defined texel.
    #[must_use]
    pub fn inert() -> Self {
        Self {
            rgba: FIELD_FAR_SENTINEL_MM.to_le_bytes().to_vec(),
            width: 1,
            height: 1,
        }
    }
}

/// The group-2 material for one layer painting a contact field: the packed
/// field, a base texture at binding 0, and a sampler.
///
/// The base is the layer's own atlas when it has one — `upload` takes it as
/// `base`, and both offscreen call sites pass the layer's texture. The shader
/// samples that base for the surface colour and mixes the contact ramp over it
/// (`mesh.wgsl`, the contact branch), so a textured scan keeps its texture
/// under the reading. Binding a white 1×1 here instead would replace the whole
/// layer with a pale shell and leave `has_texture`/`show_texture` claiming
/// otherwise.
///
/// The white 1×1 is only for a layer with no atlas of its own, which is why the
/// field below is `Option`.
pub struct GpuContactMaterial {
    /// The white 1×1 base this material built for a layer with no texture of
    /// its own, kept alive because the bind group's binding 0 points at it.
    /// `None` when the layer's own texture is bound instead — that texture
    /// stays alive in the prepared entry that uploaded it.
    #[allow(dead_code)]
    base_owned: Option<WhiteBase>,
    #[allow(dead_code)]
    field_texture: wgpu::Texture,
    #[allow(dead_code)]
    field_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

/// A 1×1 white base texture with the view and sampler a bind group needs.
struct WhiteBase {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    /// The bind group's binding 0 points here; kept alive with the texture.
    #[allow(dead_code)]
    view: wgpu::TextureView,
    /// The bind group's binding 1 points here; kept alive with the texture.
    #[allow(dead_code)]
    sampler: wgpu::Sampler,
}

impl GpuContactMaterial {
    /// Upload one packed field and build the group-2 bind group for it.
    ///
    /// `field.rgba` is written verbatim: it already holds the bytes the shader
    /// decodes, so an upload is one copy with no per-texel work on the way.
    ///
    /// `base` is the layer's own uploaded texture, when it has one. It is bound
    /// at binding 0 unchanged, so the only thing this material replaces is the
    /// packed field the vertex stage reads.
    #[must_use]
    pub fn upload(
        renderer: &Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field: &ContactFieldTexels,
        base: Option<&GpuTexture>,
    ) -> Self {
        if let Some(texture) = base {
            let (field_texture, field_view, bind_group) = build_bind_group(
                renderer,
                device,
                queue,
                field,
                texture.view(),
                texture.sampler(),
            );
            return Self {
                base_owned: None,
                field_texture,
                field_view,
                bind_group,
            };
        }
        // A layer with no texture of its own still needs a complete group: a
        // white 1×1 leaves the shader's `has_texture == 0` branch in charge,
        // exactly as it was without a field.
        let (texture, view, sampler) = white_base(device, queue);
        let (field_texture, field_view, bind_group) =
            build_bind_group(renderer, device, queue, field, &view, &sampler);
        Self {
            base_owned: Some(WhiteBase {
                texture,
                view,
                sampler,
            }),
            field_texture,
            field_view,
            bind_group,
        }
    }

    /// The group-2 bind group to bind for this layer.
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

/// Build the group-2 bind group: the base the shader samples for the scan's own
/// colour, the sampler, and the packed field the vertex stage loads by index.
// One group, three bindings and the device they are made on: bundling them into
// a struct would only move the same six values one line up.
#[allow(clippy::too_many_arguments)]
fn build_bind_group(
    renderer: &Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    field: &ContactFieldTexels,
    base_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let (field_texture, field_view) = field_texture(device, queue, field);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("occluview contact field bind group"),
        layout: renderer.texture_layout(),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(base_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&field_view),
            },
        ],
    });
    (field_texture, field_view, bind_group)
}

/// The white 1×1 base texture and sampler every group-2 bind group carries.
///
/// Shared with [`crate::GpuTexture`] and the offscreen fallback so the three
/// places that build this layout cannot drift on the sampler's addressing: a
/// field is addressed by texel, and a repeating sampler on a 1×1 base texture
/// would wrap a scan's UVs into a single white pixel's worth of variation.
pub(crate) fn white_base(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview white base texture"),
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
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
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
        label: Some("occluview contact sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}

/// A 1×1 field holding the sentinel, for a bind group with no contacts.
///
/// Wgpu requires every binding in a layout to be present, so the two places
/// that build a group-2 bind group without a field ([`crate::GpuTexture`] and
/// the offscreen fallback) bind this. The texture is intentionally not sampled
/// with linear filtering across rows: the field texture is loaded by exact
/// texel index, so no filter ever touches it.
pub(crate) fn inert_field_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    field_texture(device, queue, &ContactFieldTexels::inert())
}

/// Upload `field` as an `Rgba8Unorm` texture, one texel per four bytes.
fn field_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    field: &ContactFieldTexels,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview contact field texture"),
        size: wgpu::Extent3d {
            width: field.width,
            height: field.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // `Rgba8Unorm`, not `Rgba8UnormSrgb`: the texels are not colours, they
        // are the bytes of an `f32`, and any format conversion would corrupt
        // them. It must also stay *filterable* — wgpu rejects an
        // `unfilterable-float` texture in a bind group that holds a filtering
        // sampler, which group 2 does.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &field.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(field.width * FIELD_TEXEL_BYTES as u32),
            rows_per_image: Some(field.height),
        },
        wgpu::Extent3d {
            width: field.width,
            height: field.height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]

    use super::*;

    /// The shader decodes an `f32` out of four bytes, so the CPU side of that
    /// contract has to be exact — not "close enough to see".
    #[test]
    fn a_packed_value_survives_the_byte_pattern_round_trip() {
        let values = [
            -0.22_f32,
            0.0,
            0.015,
            FIELD_FAR_SENTINEL_MM,
            f32::MIN,
            f32::MAX,
        ];
        let field = ContactFieldTexels::from_values(&values, 16).expect("a packed field");

        // A field is never wider than it is long: the row count follows the
        // vertex count, so a short field is one row rather than a wide one with
        // a row of unused padding behind it.
        assert_eq!((field.width, field.height), (6, 1));
        for (index, expected) in values.iter().enumerate() {
            let at = index * 4;
            let bytes = &field.rgba[at..at + 4];
            let decoded = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            assert_eq!(
                decoded.to_bits(),
                expected.to_bits(),
                "texel {index} must decode to the exact value it was packed from"
            );
        }
    }

    /// The last row is padded rather than ragged: the shader computes a texel
    /// coordinate from a vertex index, so a short row would shift every vertex
    /// after the first row and paint the wrong reading.
    #[test]
    fn a_ragged_last_row_is_padded_with_the_sentinel() {
        let field = ContactFieldTexels::from_values(&[-0.1, -0.2, -0.3, -0.4, -0.5], 4)
            .expect("a packed field");

        assert_eq!((field.width, field.height), (4, 2));
        assert_eq!(field.rgba.len(), 8 * 4);
        // Five values, eight texels: the last two are one row of padding.
        for index in 5..8 {
            let at = index * 4;
            let decoded = f32::from_le_bytes([
                field.rgba[at],
                field.rgba[at + 1],
                field.rgba[at + 2],
                field.rgba[at + 3],
            ]);
            assert_eq!(decoded, FIELD_FAR_SENTINEL_MM, "texel {index} is padding");
        }
    }

    /// A `+inf` sentinel would produce `0 * inf = NaN` in the interpolated
    /// varying, and a NaN alpha composites as black or as nothing depending on
    /// the driver. A finite far value paints nothing just as well.
    #[test]
    fn a_non_finite_field_value_is_packed_as_the_far_sentinel() {
        let field = ContactFieldTexels::from_values(&[f32::INFINITY, f32::NAN, 0.02], 3)
            .expect("a packed field");

        for index in [0usize, 1] {
            let at = index * 4;
            let decoded = f32::from_le_bytes([
                field.rgba[at],
                field.rgba[at + 1],
                field.rgba[at + 2],
                field.rgba[at + 3],
            ]);
            assert_eq!(decoded, FIELD_FAR_SENTINEL_MM);
        }
    }

    #[test]
    fn a_field_that_does_not_match_its_dimensions_is_refused() {
        assert!(ContactFieldTexels::new(vec![0; 7], 2, 1).is_none());
        assert!(ContactFieldTexels::new(vec![0; 8], 2, 1).is_some());
        assert!(ContactFieldTexels::new(Vec::new(), 0, 1).is_none());
        assert!(ContactFieldTexels::from_values(&[], 4).is_none());
    }

    /// The sentinel must be past every law's paint edge, or an unreachable
    /// vertex would show up as a contact on a bare surface. Pinned at compile
    /// time: the fact is about the constant, not about a run.
    const WIDEST_LAW_PAINT_FAR_MM: f32 = 0.2;
    const _: () = assert!(FIELD_FAR_SENTINEL_MM > WIDEST_LAW_PAINT_FAR_MM);

    #[test]
    fn the_sentinel_survives_its_own_byte_pattern() {
        assert_eq!(
            f32::from_le_bytes(FIELD_FAR_SENTINEL_MM.to_le_bytes()),
            FIELD_FAR_SENTINEL_MM
        );
    }
}

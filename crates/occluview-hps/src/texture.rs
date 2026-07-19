use crate::{
    base64, crypto,
    parser::{encrypted_element_uses_scrambled_key, original_size_attr},
    xml, DecodedTexture, HpsError,
};
use image::GenericImageView;
use std::io::Cursor;
use std::mem::size_of;
use zeroize::Zeroizing;

const MAX_TEXTURE_DIMENSION_PX: u32 = 8_192;
const MAX_TEXTURE_RGBA_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct SurfaceTexture {
    pub(super) corner_uvs: Option<Vec<Option<[f32; 2]>>>,
    pub(super) texture: Option<DecodedTexture>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RawTextureLayout {
    Rgb,
    Bgr,
    Rgba,
    Bgra,
    Argb,
    Abgr,
}

pub(super) fn parse_texture_data(
    text: &str,
    schema: &str,
    key: Option<&[u8]>,
    vertex_count: usize,
    indices: &[u32],
) -> Result<Option<SurfaceTexture>, HpsError> {
    let corner_uvs = parse_texture_coordinates(text, schema, key, vertex_count, indices)?;
    let texture = parse_texture_image(text)?;
    if corner_uvs.is_none() && texture.is_none() {
        return Ok(None);
    }
    Ok(Some(SurfaceTexture {
        corner_uvs,
        texture,
    }))
}

fn parse_texture_coordinates(
    text: &str,
    schema: &str,
    key: Option<&[u8]>,
    vertex_count: usize,
    indices: &[u32],
) -> Result<Option<Vec<Option<[f32; 2]>>>, HpsError> {
    let Some(element) = xml::find_elements(text, "PerVertexTextureCoord")
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    let mut raw = Zeroizing::new(base64::decode(element.body)?);
    if schema == "CE" {
        let key = key.ok_or(HpsError::KeyMissing)?;
        raw = crypto::decrypt_hps_data(
            &raw,
            key,
            original_size_attr(element.open_tag)?,
            encrypted_element_uses_scrambled_key(element.open_tag)?,
        )?;
    }

    decode_per_vertex_texture_coordinates(&raw, vertex_count, indices).map(Some)
}

fn parse_texture_image(text: &str) -> Result<Option<DecodedTexture>, HpsError> {
    let Some(element) = xml::find_elements(text, "TextureImage").into_iter().next() else {
        return Ok(None);
    };
    let bytes = base64::decode(element.body)?;
    let texture = if let Some(texture) = parse_raw_texture_image(element.open_tag, &bytes)? {
        texture
    } else {
        decode_embedded_raster(&bytes)?
    };
    Ok(Some(correct_channel_order_for_dental(texture)?))
}

/// A dental scan surface is always physically WARM: enamel and stone are
/// cream/white, gingiva is pink-to-red, so over any real texture `R >= B` on
/// average. This holds regardless of how the texture was decoded — the
/// embedded-raster path (`decode_embedded_raster`) trusts the container's own
/// declared color space with no channel-order ambiguity to resolve, yet a
/// real exporter-authored 3Shape/HPS JPEG atlas can still have its own
/// chroma channels swapped at the SOURCE (Cb/Cr transposed before
/// compression) — a standards-compliant decode of a mis-authored file still
/// comes out blue. Detecting the physical prior and undoing a channel swap
/// catches that case independent of its root cause, on top of the raw-path's
/// own DirectX-format-aware layout guess.
///
/// Faint natural blue casts (a cool composite light, a bluish stone shade)
/// stay well under the threshold and are never touched; only an
/// implausible, whole-texture blue bias trips it.
fn correct_channel_order_for_dental(texture: DecodedTexture) -> Result<DecodedTexture, HpsError> {
    let (width, height, mut rgba) = texture.into_parts();
    if texture_is_implausibly_blue(&rgba) {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    // Dimensions and byte length are unchanged (only individual channel BYTES
    // moved within each already-present pixel), so this can never fail — the
    // original texture's own construction already proved them sound. Routed
    // through the fallible constructor anyway (propagated with `?` by the
    // caller) rather than unwrapped, matching the crate-wide ban on panics.
    DecodedTexture::new(width, height, rgba)
}

/// Strided, deterministic sample of up to ~4096 pixels (skipping fully
/// transparent and near-gray pixels, which carry no reliable hue signal).
/// Requires the blue bias to be near-UNIFORM across the sampled pixels, not
/// just present in the aggregate mean: a genuine chroma swap at the source
/// flips R/B for every pixel alike, so virtually every non-gray sample reads
/// blue-biased once swapped. A real blue dental material (anti-glare spray,
/// bite-registration silicone) instead covers only PART of the scan, so most
/// of the sampled surface stays warm even where the material itself reads
/// strongly blue — the aggregate mean alone cannot tell these two cases
/// apart (a strong localized patch can drag the mean past the margin on its
/// own), but the per-pixel PROPORTION can, since only a real whole-texture
/// swap makes nearly every sample agree. Matches the calibration measured
/// against a real 3Shape/HPS dental scan JPEG atlas with swapped chroma
/// (mean R 107 / mean B 150 on the swapped file, uniformly across the
/// texture; mean R 150 / mean B 107 once corrected).
fn texture_is_implausibly_blue(rgba: &[u8]) -> bool {
    const SAMPLE_BUDGET: usize = 4096;
    const MIN_ALPHA: u8 = 8;
    const NEAR_GRAY_DELTA: i32 = 16;
    /// Fraction of sampled (non-transparent, non-near-gray) pixels that must
    /// INDIVIDUALLY read blue-biased before this is treated as a uniform
    /// source-level channel swap rather than a localized blue material.
    const BLUE_BIASED_PIXEL_FRACTION: f64 = 0.9;

    let pixel_count = rgba.len() / 4;
    if pixel_count == 0 {
        return false;
    }
    let stride = (pixel_count / SAMPLE_BUDGET).max(1);

    let mut red_sum: u64 = 0;
    let mut blue_sum: u64 = 0;
    let mut blue_biased: u64 = 0;
    let mut sampled: u64 = 0;
    for pixel in rgba.chunks_exact(4).step_by(stride) {
        let [r, _g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if a < MIN_ALPHA {
            continue;
        }
        let delta = i32::from(b) - i32::from(r);
        if delta.abs() < NEAR_GRAY_DELTA {
            continue;
        }
        red_sum += u64::from(r);
        blue_sum += u64::from(b);
        if delta > 0 {
            blue_biased += 1;
        }
        sampled += 1;
    }
    if sampled == 0 {
        return false;
    }
    let red_mean = red_sum / sampled;
    let blue_mean = blue_sum / sampled;
    let margin = (red_mean / 4).max(24);
    let mean_is_blue_biased = blue_mean > red_mean + margin;
    #[allow(clippy::cast_precision_loss)]
    let blue_biased_fraction = blue_biased as f64 / sampled as f64;
    mean_is_blue_biased && blue_biased_fraction >= BLUE_BIASED_PIXEL_FRACTION
}

fn decode_embedded_raster(bytes: &[u8]) -> Result<DecodedTexture, HpsError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| texture_malformed(format!("texture format detection failed: {error}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_DIMENSION_PX);
    limits.max_image_height = Some(MAX_TEXTURE_DIMENSION_PX);
    limits.max_alloc = Some(MAX_TEXTURE_RGBA_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| texture_malformed(format!("texture image decode failed: {error}")))?;
    let (width, height) = decoded.dimensions();
    validate_texture_dimensions(width, height)?;
    DecodedTexture::new(width, height, decoded.to_rgba8().into_raw())
}

fn validate_texture_dimensions(width: u32, height: u32) -> Result<(), HpsError> {
    if width == 0 || height == 0 {
        return Err(texture_malformed("texture dimensions must be non-zero"));
    }
    if width > MAX_TEXTURE_DIMENSION_PX || height > MAX_TEXTURE_DIMENSION_PX {
        return Err(texture_malformed(format!(
            "texture dimensions {width}x{height} exceed {MAX_TEXTURE_DIMENSION_PX}px limit"
        )));
    }
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(HpsError::ResourceLimit {
            resource: "texture RGBA size",
            limit: MAX_TEXTURE_RGBA_BYTES,
        })?;
    if rgba_bytes > MAX_TEXTURE_RGBA_BYTES {
        return Err(HpsError::ResourceLimit {
            resource: "texture RGBA size",
            limit: MAX_TEXTURE_RGBA_BYTES,
        });
    }
    Ok(())
}

fn texture_malformed(reason: impl Into<String>) -> HpsError {
    HpsError::TextureMalformed {
        reason: reason.into(),
    }
}

fn parse_raw_texture_image(
    open_tag: &str,
    bytes: &[u8],
) -> Result<Option<DecodedTexture>, HpsError> {
    let Some(width) = optional_u32_attr(open_tag, "Width")? else {
        return Ok(None);
    };
    let Some(height) = optional_u32_attr(open_tag, "Height")? else {
        return Ok(None);
    };
    let Some(bytes_per_pixel) = optional_u32_attr(open_tag, "BytesPerPixel")? else {
        return Ok(None);
    };

    // Width/Height/BytesPerPixel describe the raw HPS representation, but HPS
    // exporters also store JPEG/PNG payloads with the same metadata. Compare
    // the body length before applying raw-pixel limits; otherwise a valid
    // compressed 8192x4096 atlas is rejected as if it already contained 128 MiB
    // of RGBA pixels.
    let Some(raw_len) = raw_texture_len(width, height, bytes_per_pixel) else {
        return Ok(None);
    };
    if u64::try_from(bytes.len()).ok() != Some(raw_len) {
        return Ok(None);
    }
    validate_texture_dimensions(width, height)?;

    // Deterministic decode (no pixel-content guessing): honor an explicit,
    // unambiguous pixel-format declaration when present; otherwise fall back to
    // the HPS/DirectX default. HPS raw textures are DirectX surfaces
    // (D3DFMT_A8R8G8B8 / D3DFMT_R8G8B8), whose little-endian MEMORY byte order
    // is B,G,R(,A) — so the correct default is BGRA / BGR (swap R<->B).
    let layout =
        raw_texture_layout(open_tag)?.unwrap_or_else(|| default_raw_layout(bytes_per_pixel));
    let Some(rgba) = decode_raw_texture_pixels(bytes, bytes_per_pixel, layout) else {
        return Ok(None);
    };
    DecodedTexture::new(width, height, rgba).map(Some)
}

/// The default channel layout for a format-less raw HPS texture.
///
/// HPS emits DirectX surfaces (`D3DFMT_A8R8G8B8` for 32-bit, `D3DFMT_R8G8B8`
/// for 24-bit). A D3DFMT name lists channels from the high byte of a 32-bit pixel
/// to the low byte, so the little-endian MEMORY byte order is the reverse:
/// `B,G,R,A` for `A8R8G8B8` and `B,G,R` for `R8G8B8`. Decoding as BGRA/BGR (swap
/// R<->B) is the verified-correct behavior; treating the bytes as RGBA turns warm
/// dental whites (R>=G>B) into cool blue (B>R).
fn default_raw_layout(bytes_per_pixel: u32) -> RawTextureLayout {
    match bytes_per_pixel {
        3 => RawTextureLayout::Bgr,
        _ => RawTextureLayout::Bgra,
    }
}

fn raw_texture_layout(open_tag: &str) -> Result<Option<RawTextureLayout>, HpsError> {
    const FORMAT_ATTRS: [&str; 8] = [
        "PixelFormat",
        "pixelFormat",
        "Format",
        "format",
        "ChannelOrder",
        "channelOrder",
        "ColorFormat",
        "colorFormat",
    ];

    for attr in FORMAT_ATTRS {
        let Some(value) = xml::attr_value(open_tag, attr)? else {
            continue;
        };
        let normalized: String = value
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect();
        // DirectX D3DFMT names (digit forms) list channels high-byte-first, so
        // the little-endian MEMORY byte order is the REVERSE. Map them to the
        // memory layout the bytes actually arrive in — the historic mismap of
        // A8R8G8B8 -> literal ARGB is exactly what rendered white scans blue.
        if normalized.contains("a8r8g8b8") || normalized.contains("x8r8g8b8") {
            return Ok(Some(RawTextureLayout::Bgra)); // 0xAARRGGBB -> [B,G,R,A]
        }
        if normalized.contains("a8b8g8r8") || normalized.contains("x8b8g8r8") {
            return Ok(Some(RawTextureLayout::Rgba)); // 0xAABBGGRR -> [R,G,B,A]
        }
        // DXGI-style names (digit forms) already list MEMORY byte order.
        if normalized.contains("b8g8r8a8") {
            return Ok(Some(RawTextureLayout::Bgra));
        }
        if normalized.contains("r8g8b8a8") {
            return Ok(Some(RawTextureLayout::Rgba));
        }
        if normalized.contains("b8g8r8") {
            return Ok(Some(RawTextureLayout::Bgr));
        }
        if normalized.contains("r8g8b8") {
            return Ok(Some(RawTextureLayout::Rgb));
        }
        // Bare tokens state the MEMORY byte order literally.
        if normalized.contains("abgr") {
            return Ok(Some(RawTextureLayout::Abgr));
        }
        if normalized.contains("argb") {
            return Ok(Some(RawTextureLayout::Argb));
        }
        if normalized.contains("bgra") {
            return Ok(Some(RawTextureLayout::Bgra));
        }
        if normalized.contains("rgba") {
            return Ok(Some(RawTextureLayout::Rgba));
        }
        if normalized.contains("bgr") {
            return Ok(Some(RawTextureLayout::Bgr));
        }
        if normalized.contains("rgb") {
            return Ok(Some(RawTextureLayout::Rgb));
        }
    }

    Ok(None)
}

fn decode_raw_texture_pixels(
    bytes: &[u8],
    bytes_per_pixel: u32,
    layout: RawTextureLayout,
) -> Option<Vec<u8>> {
    let mut rgba = Vec::with_capacity((bytes.len() / bytes_per_pixel as usize) * 4);
    for pixel in bytes.chunks_exact(bytes_per_pixel as usize) {
        rgba.extend_from_slice(&decode_raw_texture_pixel(pixel, bytes_per_pixel, layout)?);
    }
    Some(rgba)
}

fn decode_raw_texture_pixel(
    pixel: &[u8],
    bytes_per_pixel: u32,
    layout: RawTextureLayout,
) -> Option<[u8; 4]> {
    match (bytes_per_pixel, layout) {
        (3, RawTextureLayout::Rgb) => Some([pixel[0], pixel[1], pixel[2], 255]),
        (3, RawTextureLayout::Bgr) => Some([pixel[2], pixel[1], pixel[0], 255]),
        (4, RawTextureLayout::Rgb | RawTextureLayout::Rgba) => {
            Some([pixel[0], pixel[1], pixel[2], pixel[3]])
        }
        (4, RawTextureLayout::Bgr | RawTextureLayout::Bgra) => {
            Some([pixel[2], pixel[1], pixel[0], pixel[3]])
        }
        (4, RawTextureLayout::Argb) => Some([pixel[1], pixel[2], pixel[3], pixel[0]]),
        (4, RawTextureLayout::Abgr) => Some([pixel[3], pixel[2], pixel[1], pixel[0]]),
        _ => None,
    }
}

fn optional_u32_attr(open_tag: &str, attr: &str) -> Result<Option<u32>, HpsError> {
    xml::attr_value(open_tag, attr)?
        .map(|value| xml::parse_u32_attr(value, attr))
        .transpose()
}

fn raw_texture_len(width: u32, height: u32, bytes_per_pixel: u32) -> Option<u64> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .map(u64::from)
}

fn decode_per_vertex_texture_coordinates(
    bytes: &[u8],
    vertex_count: usize,
    indices: &[u32],
) -> Result<Vec<Option<[f32; 2]>>, HpsError> {
    let corners_by_vertex = build_vertex_corner_map(indices, vertex_count)?;
    let mut corner_uvs = vec![None; indices.len()];
    let mut offset = 0;

    for corners in corners_by_vertex.iter().take(vertex_count) {
        let flag = read_u8(bytes, &mut offset)?;
        let degree = corners.len();
        if flag == 0 {
            if degree != 0 {
                return Err(texture_malformed(
                    "texture coordinate stream degree mismatch",
                ));
            }
            continue;
        }

        let uv_count = if flag == 1 {
            1
        } else if usize::from(flag) == degree {
            degree
        } else {
            return Err(texture_malformed("texture coordinate stream flag mismatch"));
        };

        let mut decoded = Vec::with_capacity(uv_count);
        for _ in 0..uv_count {
            decoded.push(decode_packed_texture_coordinate(read_u32_le(
                bytes,
                &mut offset,
            )?));
        }

        if degree == 0 {
            continue;
        }
        if flag == 1 {
            for &corner in corners {
                corner_uvs[corner] = decoded[0];
            }
        } else {
            for (corner_ordinal, &corner) in corners.iter().enumerate() {
                corner_uvs[corner] = decoded[corner_ordinal];
            }
        }
    }

    Ok(corner_uvs)
}

fn build_vertex_corner_map(
    indices: &[u32],
    vertex_count: usize,
) -> Result<Vec<Vec<usize>>, HpsError> {
    let mut corners_by_vertex = vec![Vec::new(); vertex_count];
    for (corner, &vertex_index) in indices.iter().enumerate() {
        let vertex_index = vertex_index as usize;
        let Some(corners) = corners_by_vertex.get_mut(vertex_index) else {
            return Err(texture_malformed(
                "texture coordinate vertex index out of bounds",
            ));
        };
        corners.push(corner);
    }
    Ok(corners_by_vertex)
}

fn read_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, HpsError> {
    let Some(&value) = bytes.get(*offset) else {
        return Err(texture_malformed("texture coordinate stream truncated"));
    };
    *offset += 1;
    Ok(value)
}

fn read_u32_le(bytes: &[u8], offset: &mut usize) -> Result<u32, HpsError> {
    let end = offset
        .checked_add(size_of::<u32>())
        .ok_or_else(|| texture_malformed("texture coordinate stream offset overflow"))?;
    let chunk = bytes
        .get(*offset..end)
        .ok_or_else(|| texture_malformed("texture coordinate stream truncated"))?;
    *offset = end;
    Ok(u32::from_le_bytes(chunk.try_into().map_err(|_| {
        texture_malformed("bad texture coordinate")
    })?))
}

fn decode_packed_texture_coordinate(packed: u32) -> Option<[f32; 2]> {
    if packed == u32::MAX {
        return None;
    }
    Some([
        decode_packed_texture_component((packed & 0xffff) as u16),
        decode_packed_texture_component(((packed >> 16) & 0xffff) as u16),
    ])
}

fn decode_packed_texture_component(component: u16) -> f32 {
    let value = f32::from(component & 0x7fff);
    if component & 0x8000 == 0 {
        value / 32767.0
    } else {
        value * (512.0 / 32767.0) - 256.0
    }
}

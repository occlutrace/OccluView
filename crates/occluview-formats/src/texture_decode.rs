//! Bounded decoding for embedded PNG/JPEG textures.
//!
//! Format readers run both in the interactive app and the Explorer thumbnail
//! host. A compressed raster can claim a huge decoded surface, so dimensions
//! and RGBA allocation are validated before it becomes a mesh texture.

use crate::error::FormatError;
use image::GenericImageView;
use occluview_core::MeshTexture;
use std::io::Cursor;

/// A texture may be wide enough for scanner exports without letting one image
/// dominate the viewer or the Windows thumbnail provider's address space.
pub(crate) const MAX_TEXTURE_DIMENSION_PX: u32 = 8_192;
/// Maximum resulting RGBA buffer (4K square is exactly 64 MiB).
pub(crate) const MAX_TEXTURE_RGBA_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn decode_embedded_raster(
    bytes: &[u8],
    format: &'static str,
) -> Result<MeshTexture, FormatError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| {
            texture_error(format, format!("texture format detection failed: {error}"))
        })?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_DIMENSION_PX);
    limits.max_image_height = Some(MAX_TEXTURE_DIMENSION_PX);
    limits.max_alloc = Some(MAX_TEXTURE_RGBA_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| texture_error(format, format!("texture image decode failed: {error}")))?;
    let (width, height) = decoded.dimensions();
    validate_texture_dimensions(width, height, format)?;
    Ok(MeshTexture::new(
        width,
        height,
        decoded.to_rgba8().into_raw(),
    ))
}

pub(crate) fn validate_texture_dimensions(
    width: u32,
    height: u32,
    format: &'static str,
) -> Result<(), FormatError> {
    if width == 0 || height == 0 {
        return Err(texture_error(format, "texture dimensions must be non-zero"));
    }
    if width > MAX_TEXTURE_DIMENSION_PX || height > MAX_TEXTURE_DIMENSION_PX {
        return Err(texture_error(
            format,
            format!(
                "texture dimensions {width}x{height} exceed {MAX_TEXTURE_DIMENSION_PX}px limit"
            ),
        ));
    }
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| texture_error(format, "texture RGBA size overflow"))?;
    if rgba_bytes > MAX_TEXTURE_RGBA_BYTES {
        return Err(texture_error(
            format,
            format!("texture RGBA size {rgba_bytes} exceeds {MAX_TEXTURE_RGBA_BYTES} byte limit"),
        ));
    }
    Ok(())
}

fn texture_error(format: &'static str, reason: impl Into<String>) -> FormatError {
    FormatError::Malformed {
        format,
        offset: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_accept_a_4k_square_but_reject_larger_rgba_surfaces() {
        assert!(validate_texture_dimensions(4_096, 4_096, "test").is_ok());
        assert!(validate_texture_dimensions(8_192, 8_192, "test").is_err());
        assert!(validate_texture_dimensions(0, 256, "test").is_err());
    }

    #[test]
    fn dimensions_reject_a_single_axis_over_the_strict_limit() {
        assert!(validate_texture_dimensions(MAX_TEXTURE_DIMENSION_PX + 1, 1, "test").is_err());
        assert!(validate_texture_dimensions(1, MAX_TEXTURE_DIMENSION_PX + 1, "test").is_err());
    }
}

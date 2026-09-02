//! Bounded decoding for embedded PNG/JPEG textures.
//!
//! Format readers run both in the interactive app and the Explorer thumbnail
//! host. A compressed raster can claim a huge decoded surface, so dimensions
//! and RGBA allocation are validated before it becomes a mesh texture.

use crate::error::FormatError;
use image::GenericImageView;
use occluview_core::MeshTexture;
use std::io::Cursor;

// One definition of the texture budget for every reader in the workspace,
// owned by the crate that reads the format most exposed to it. See
// `occluview_hps::MAX_TEXTURE_DIMENSION_PX` for why there is exactly one.
pub(crate) use occluview_hps::{MAX_TEXTURE_DIMENSION_PX, MAX_TEXTURE_RGBA_BYTES};

const EMBEDDED_RASTER_FORMAT_POLICY: &str =
    "embedded texture format is not permitted; only PNG and JPEG are accepted";

pub(crate) fn decode_embedded_raster(
    bytes: &[u8],
    format: &'static str,
) -> Result<MeshTexture, FormatError> {
    let image_format = accepted_embedded_raster_format(bytes, format)?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image_format);
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

fn accepted_embedded_raster_format(
    bytes: &[u8],
    container_format: &'static str,
) -> Result<image::ImageFormat, FormatError> {
    let image_format = image::guess_format(bytes).map_err(|error| {
        texture_error(
            container_format,
            format!("texture format detection failed: {error}"),
        )
    })?;
    match image_format {
        image::ImageFormat::Png | image::ImageFormat::Jpeg => Ok(image_format),
        unsupported => Err(texture_error(
            container_format,
            format!("{EMBEDDED_RASTER_FORMAT_POLICY}: {unsupported:?}"),
        )),
    }
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

    /// A compact PNG whose decoded width exceeds the allocation limit.
    fn over_wide_png(width: u32) -> Vec<u8> {
        use image::ImageEncoder as _;
        let mut bytes = Vec::new();
        let row = vec![0_u8; width as usize];
        let encoded = image::codecs::png::PngEncoder::new(&mut bytes).write_image(
            &row,
            width,
            1,
            image::ExtendedColorType::L8,
        );
        assert!(encoded.is_ok(), "fixture encode failed: {encoded:?}");
        bytes
    }

    #[test]
    fn a_header_claiming_more_than_the_pixel_limit_is_refused_before_decoding() {
        // `validate_texture_dimensions` runs on an image that has already been
        // decoded, so by the time it can complain the allocation has happened.
        // The line that actually prevents the bomb is `reader.limits(limits)`,
        // and deleting it broke no test. This one fails without it.
        let bomb = over_wide_png(MAX_TEXTURE_DIMENSION_PX + 808);
        assert!(
            bomb.len() < 4096,
            "the fixture must stay small to be a bomb at all: {} bytes",
            bomb.len()
        );

        let decoded = decode_embedded_raster(&bomb, "test");
        let Err(FormatError::Malformed { reason, .. }) = decoded else {
            unreachable!("an over-wide texture must be rejected, got {decoded:?}");
        };
        assert!(
            reason.contains("decode failed"),
            "rejection should come from the bounded decoder, not from a \
             post-decode dimension check: {reason}"
        );
    }

    #[test]
    fn a_texture_inside_the_limits_still_decodes() {
        let ordinary = over_wide_png(64);
        let decoded = decode_embedded_raster(&ordinary, "test");
        assert!(decoded.is_ok(), "a 64x1 texture should decode: {decoded:?}");
    }

    #[test]
    fn the_edge_and_byte_limits_bound_the_same_decoded_surface() {
        // 8192 x 8192 x 4 is exactly MAX_TEXTURE_RGBA_BYTES, so the two limits
        // meet at the same image: the largest square that passes the edge test
        // is also the largest surface the byte test allows.
        assert!(validate_texture_dimensions(4_096, 4_096, "test").is_ok());
        assert!(validate_texture_dimensions(8_192, 8_192, "test").is_ok());
        assert_eq!(
            u64::from(MAX_TEXTURE_DIMENSION_PX) * u64::from(MAX_TEXTURE_DIMENSION_PX) * 4,
            MAX_TEXTURE_RGBA_BYTES
        );
        // A lopsided atlas inside the edge limit is what the byte limit is for.
        assert!(validate_texture_dimensions(8_192, 4_096, "test").is_ok());
        assert!(validate_texture_dimensions(0, 256, "test").is_err());
    }

    #[test]
    fn both_readers_share_one_texture_budget() {
        // A second copy of these numbers here once read 64 MiB against the HPS
        // crate's 256 MiB, so the same image was accepted from a dental
        // container and refused from a `.glb`, in one process, with nothing
        // explaining why. The check is that this crate re-exports rather than
        // redefines; an equality assertion passes either way the moment the two
        // numbers happen to agree.
        // Only the part above this module counts. Search the whole file and
        // the needle in this very assertion answers it, so the import could be
        // split into two brace-less `use` lines and the guard would still pass.
        let source = include_str!("texture_decode.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            production.contains("pub(crate) use occluview_hps::{"),
            "the texture budget must be imported from occluview-hps, not redefined"
        );
        // The needles are assembled so this guard does not match its own source.
        for (name, ty) in [
            ("MAX_TEXTURE_DIMENSION_PX", "u32"),
            ("MAX_TEXTURE_RGBA_BYTES", "u64"),
        ] {
            let redefinition = format!("const {name}: {ty} =");
            assert!(
                !source.contains(&redefinition),
                "a second definition is how the two limits drifted apart: {redefinition}"
            );
        }
    }

    #[test]
    fn dimensions_reject_a_single_axis_over_the_strict_limit() {
        assert!(validate_texture_dimensions(MAX_TEXTURE_DIMENSION_PX + 1, 1, "test").is_err());
        assert!(validate_texture_dimensions(1, MAX_TEXTURE_DIMENSION_PX + 1, "test").is_err());
    }
}

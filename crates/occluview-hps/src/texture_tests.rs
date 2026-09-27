//! Texture decode/color-correction tests. Shares the base64/XML fixture
//! builders from [`crate::tests`].

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::float_cmp
)]

use super::*;
use crate::tests::{append_packed_uv, cc_fixture, encode_base64, red_png_bytes, small_jpeg_bytes};
use std::io::Cursor;

fn embedded_raster_hps(raster: &[u8]) -> Vec<u8> {
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raster.len(),
        encode_base64(raster)
    );
    cc_fixture(3, 1, &[4], &extra)
}

fn solid_raster(format: image::ImageFormat) -> Vec<u8> {
    let image = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).expect("image dims");
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut output, format)
        .expect("encode test raster");
    output.into_inner()
}

fn assert_whitelist_rejection(result: Result<DecodedSurface, HpsError>) {
    let Err(HpsError::TextureMalformed { reason }) = result else {
        unreachable!("non-PNG/JPEG texture must be rejected, got {result:?}");
    };
    assert!(
        reason.contains("only PNG and JPEG are accepted"),
        "the rejection must be the raster allowlist, not a decoder accident: {reason}"
    );
}

#[test]
fn texture_data_preserves_source_topology_and_corner_uvs() {
    let mut uv_bytes = Vec::new();
    uv_bytes.push(2);
    append_packed_uv(&mut uv_bytes, 0.0, 0.0);
    append_packed_uv(&mut uv_bytes, 0.75, 0.25);
    uv_bytes.push(2);
    append_packed_uv(&mut uv_bytes, 1.0, 0.0);
    append_packed_uv(&mut uv_bytes, 1.0, 1.0);
    uv_bytes.push(1);
    append_packed_uv(&mut uv_bytes, 0.5, 1.0);
    uv_bytes.push(1);
    append_packed_uv(&mut uv_bytes, 0.25, 0.75);

    let png_bytes = red_png_bytes();
    let extra = format!(
        r#"  <TextureData2>
    <PerVertexTextureCoord TextureCoordId="uv0" TextureId="tex0" Base64EncodedBytes="{}">{}</PerVertexTextureCoord>
    <TextureImages>
      <TextureImage TextureId="tex0" RefTextureCoordId="uv0" Width="2" Height="2" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        uv_bytes.len(),
        encode_base64(&uv_bytes),
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let surface = read(&cc_fixture(4, 2, &[4, 0], &extra)).expect("textured HPS should read");
    assert_eq!(surface.positions().len(), 4);
    assert_eq!(surface.indices().len(), 6);
    let corner_uvs = surface
        .corner_uvs()
        .expect("texture coordinates should be corner indexed");
    assert_eq!(corner_uvs.len(), surface.indices().len());
    assert_eq!(corner_uvs[0], Some([0.0, 0.0]));

    let texture = surface.texture().expect("HPS texture should be attached");
    assert_eq!(texture.width(), 2);
    assert_eq!(texture.height(), 2);
    assert!(texture
        .rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [255, 0, 0, 255]));
}

#[test]
fn embedded_png_and_jpeg_remain_accepted() {
    for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
        let surface = read(&embedded_raster_hps(&solid_raster(format)));
        assert!(
            surface.is_ok(),
            "{format:?} texture should remain accepted: {surface:?}"
        );
    }
}

#[test]
fn valid_bmp_is_rejected_by_whitelist_before_decode() {
    assert_whitelist_rejection(read(&embedded_raster_hps(&solid_raster(
        image::ImageFormat::Bmp,
    ))));
}

#[test]
fn malformed_bmp_magic_is_rejected_by_whitelist_before_decode() {
    assert_whitelist_rejection(read(&embedded_raster_hps(b"BM")));
}

#[test]
fn raw_bgra_texture_image_converts_to_rgba() {
    let raw_bgra = [
        12, 34, 200, 255, // R=200, G=34, B=12
        90, 80, 70, 255, // R=70, G=80, B=90
        3, 2, 1, 255, // R=1, G=2, B=3
        30, 20, 10, 128, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="BGRA" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(texture.width(), 2);
    assert_eq!(texture.height(), 2);
    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128,]
    );
}

#[test]
fn raw_rgba_texture_image_keeps_declared_rgba_order() {
    let raw_rgba = [
        200, 34, 12, 255, // R=200, G=34, B=12
        70, 80, 90, 255, // R=70, G=80, B=90
        1, 2, 3, 255, // R=1, G=2, B=3
        10, 20, 30, 128, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="RGBA" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_rgba.len(),
        encode_base64(&raw_rgba)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(texture.rgba(), &raw_rgba);
}

/// A declared channel order wins over the hue heuristic: a legitimately cool
/// atlas stored RGBA must be decoded as declared, not inverted to warm.
///
/// The heuristic exists for a compressed image, which carries no channel order
/// at all. A raw texture states its layout, so `parse_raw_texture_image` has
/// already produced the right order and the prior must not second-guess it.
#[test]
fn a_declared_raw_layout_is_not_overridden_by_the_hue_heuristic() {
    // A uniform cool-white surface, physically R=186, G=198, B=210.
    let raw_rgba = [
        186, 198, 210, 255, 186, 198, 210, 255, 186, 198, 210, 255, 186, 198, 210, 255,
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="RGBA" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_rgba.len(),
        encode_base64(&raw_rgba)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");
    assert_eq!(
        texture.rgba(),
        &raw_rgba,
        "a declared RGBA order must not be re-guessed by the hue prior"
    );
}

#[test]
fn raw_argb_texture_image_keeps_declared_argb_order() {
    let raw_argb = [
        255, 200, 34, 12, // R=200, G=34, B=12
        255, 70, 80, 90, // R=70, G=80, B=90
        255, 1, 2, 3, // R=1, G=2, B=3
        128, 10, 20, 30, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="ARGB" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_argb.len(),
        encode_base64(&raw_argb)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128]
    );
}

#[test]
fn raw_abgr_texture_image_keeps_declared_abgr_order() {
    let raw_abgr = [
        255, 12, 34, 200, // R=200, G=34, B=12
        255, 90, 80, 70, // R=70, G=80, B=90
        255, 3, 2, 1, // R=1, G=2, B=3
        128, 30, 20, 10, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="ABGR" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_abgr.len(),
        encode_base64(&raw_abgr)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 128]
    );
}

#[test]
fn raw_rgb_texture_image_keeps_declared_rgb_order() {
    let raw_rgb = [
        200, 34, 12, // R=200, G=34, B=12
        70, 80, 90, // R=70, G=80, B=90
        1, 2, 3, // R=1, G=2, B=3
        10, 20, 30, // R=10, G=20, B=30
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="3" Format="RGB" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_rgb.len(),
        encode_base64(&raw_rgb)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    assert_eq!(
        texture.rgba(),
        vec![200, 34, 12, 255, 70, 80, 90, 255, 1, 2, 3, 255, 10, 20, 30, 255]
    );
}

#[test]
fn compressed_texture_uses_decoded_dimensions_before_raw_metadata_limits() {
    let jpeg = small_jpeg_bytes();
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="8192" Height="4096" BytesPerPixel="3" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        jpeg.len(),
        encode_base64(&jpeg)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra))
        .expect("a compressed texture must not be rejected as raw RGBA");
    let texture = mesh
        .texture()
        .expect("compressed HPS texture should be attached");

    assert_eq!((texture.width(), texture.height()), (2, 1));
    assert_eq!(texture.rgba().len(), 2 * 4);
}

// A format-less raw HPS texture decodes deterministically as BGRA: HPS
// emits DirectX surfaces (D3DFMT_A8R8G8B8) whose memory byte order is [B,G,R,A].
// A warm-white dental surface (physical R>=G>B) is stored with the small blue
// value in byte 0, and swapping R<->B keeps enamel warm instead of turning it
// blue.
#[test]
fn raw_texture_image_without_format_defaults_to_bgra_swap() {
    // Bytes are a warm-white enamel patch stored BGRA: byte0=B(small) .. byte2=R(large).
    let raw_bgra = [
        118, 164, 205, 255, 105, 151, 194, 255, 101, 144, 184, 255, 132, 176, 218, 255,
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    // R<->B swapped: warm-white enamel (R>B), never cool blue.
    assert_eq!(
        texture.rgba(),
        vec![205, 164, 118, 255, 194, 151, 105, 255, 184, 144, 101, 255, 218, 176, 132, 255]
    );
    for pixel in texture.rgba().as_chunks::<4>().0 {
        assert!(
            pixel[0] > pixel[2],
            "format-less HPS enamel must stay warm (R>B), never blue: {pixel:?}"
        );
    }
}

// White regions must not decode blue: a texture atlas dominated by
// cool/neutral stone with a minority of warm-white enamel decodes
// deterministically as BGRA, so the enamel stays warm regardless of what the
// rest of the atlas looks like — no per-scan pixel-statistics guessing.
#[test]
fn raw_texture_image_cool_dominant_atlas_keeps_enamel_warm() {
    let mut raw_bgra = Vec::new();
    // Cool-neutral stone (physical R=210,G=214,B=220) stored BGRA -> [220,214,210].
    for _ in 0..13 {
        raw_bgra.extend_from_slice(&[220, 214, 210, 255]);
    }
    // Warm-white enamel (physical R=248,G=244,B=236) stored BGRA -> [236,244,248].
    for _ in 0..3 {
        raw_bgra.extend_from_slice(&[236, 244, 248, 255]);
    }
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw_bgra.len(),
        encode_base64(&raw_bgra)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    // Enamel pixels are the last three; they must be warm (R>B), not blue.
    let enamel = &texture.rgba()[13 * 4..];
    for pixel in enamel.as_chunks::<4>().0 {
        assert!(
            pixel[0] > pixel[2],
            "warm-white enamel rendered blue under a cool-dominant atlas: {pixel:?}"
        );
    }
    // And the cool stone is faithfully reproduced (R<B), not warm-flipped.
    assert_eq!(&texture.rgba()[0..4], &[210, 214, 220, 255]);
}

// A file that declares the DirectX pixel-format name D3DFMT_A8R8G8B8 (0xAARRGGBB)
// stores memory bytes [B,G,R,A]. Decode as BGRA (swap R<->B); a literal ARGB
// decode paints entire scans blue.
#[test]
fn raw_a8r8g8b8_directx_name_decodes_as_bgra() {
    // memory bytes for a warm-white pixel: [B=236, G=244, R=248, A=255]
    let raw = [
        236, 244, 248, 255, 236, 244, 248, 255, 236, 244, 248, 255, 236, 244, 248, 255,
    ];
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="2" Height="2" BytesPerPixel="4" PixelFormat="A8R8G8B8" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        raw.len(),
        encode_base64(&raw)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("raw-textured HPS should read");
    let texture = mesh.texture().expect("raw HPS texture should be attached");

    for pixel in texture.rgba().as_chunks::<4>().0 {
        assert_eq!(
            *pixel,
            [248, 244, 236, 255],
            "A8R8G8B8 must decode to warm BGRA"
        );
    }
}

fn solid_rgba_png_bytes(
    width: u32,
    height: u32,
    pixel: [u8; 4],
    transparent_corner: bool,
) -> Vec<u8> {
    let mut data = pixel.repeat((width * height) as usize);
    if transparent_corner {
        data[3] = 0; // First pixel fully transparent: must not skew the sample.
    }
    let img = image::RgbaImage::from_raw(width, height, data).expect("image dims");
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

// Source-level chroma swap in an embedded raster is corrected by the dental
// surface prior (swapped mean R≈107/B≈150; corrected R≈150/B≈107).
#[test]
fn embedded_png_with_swapped_dental_chroma_is_corrected_to_warm() {
    let png_bytes = solid_rgba_png_bytes(4, 4, [107, 117, 150, 255], true);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    let opaque_pixels: Vec<&[u8; 4]> = texture.rgba().as_chunks::<4>().0.iter().skip(1).collect();
    for pixel in opaque_pixels {
        assert_eq!(
            *pixel,
            [150, 117, 107, 255],
            "swapped dental chroma must be corrected back to warm (R>B)"
        );
    }
    // The transparent corner pixel must not itself be mangled by the swap
    // guard sampling it should have skipped in the first place.
    assert_eq!(texture.rgba()[3], 0);
}

// A mild cool tint remains below the whole-texture swap threshold. The 20-value
// gap clears the near-gray filter and stays below the required mean excess.
#[test]
fn embedded_png_with_a_mild_cool_tint_is_left_untouched() {
    let pixel = [150, 160, 170, 255]; // R=150, B=170: a 20-value cool tint.
    let png_bytes = solid_rgba_png_bytes(4, 4, pixel, false);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    for decoded in texture.rgba().as_chunks::<4>().0 {
        assert_eq!(
            *decoded, pixel,
            "a mild cool tint must not be treated as swapped channels"
        );
    }
}

/// The cool-cast line is 24 levels, and it is a line, not a slope.
///
/// A cast below it is a cast and stays; a bias at or above it is large enough to
/// be a wrong channel order and is corrected. Pinning both sides means the
/// threshold cannot drift unnoticed in either direction.
#[test]
fn the_cool_cast_line_sits_between_23_and_24_levels() {
    // 23 levels: R=187, B=210. Just under the line, so it stays as stored.
    let under = [187, 198, 210, 255];
    // 24 levels: R=186, B=210. At the line, so it is corrected to warm.
    let at = [186, 198, 210, 255];

    let under_png = solid_rgba_png_bytes(4, 4, under, false);
    let under_extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        under_png.len(),
        encode_base64(&under_png)
    );
    let mesh = read(&cc_fixture(3, 1, &[4], &under_extra)).expect("textured HPS should read");
    for decoded in mesh.texture().expect("texture").rgba().as_chunks::<4>().0 {
        assert_eq!(
            *decoded, under,
            "a 23-level cool cast is below the line and must stay as stored"
        );
    }

    let at_png = solid_rgba_png_bytes(4, 4, at, false);
    let at_extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        at_png.len(),
        encode_base64(&at_png)
    );
    let mesh = read(&cc_fixture(3, 1, &[4], &at_extra)).expect("textured HPS should read");
    for decoded in mesh.texture().expect("texture").rgba().as_chunks::<4>().0 {
        assert_eq!(
            *decoded,
            [210, 198, 186, 255],
            "a 24-level blue bias is at the line and must be corrected"
        );
    }
}

/// A whole-texture chroma swap on a bright atlas is corrected.
///
/// Bright atlases (mean hue-bearing red around 188) are the 3Shape lab-scanner
/// case: the swap's ~34-level blue excess sits below a brightness-scaled margin
/// and the scan decodes cyan.
#[test]
fn embedded_png_with_a_bright_swapped_dental_atlas_is_corrected_to_warm() {
    // A dental surface stored with R and B transposed: what should be warm cream
    // enamel (R=235, G=222, B=205) and warm gingiva (R=200, G=150, B=130)
    // arrives as their swaps.
    let mut pixels = Vec::with_capacity(64);
    for _ in 0..48 {
        pixels.push([205, 222, 235, 255]);
    }
    for _ in 0..16 {
        pixels.push([130, 150, 200, 255]);
    }
    let png_bytes = rgba_png_bytes_from_pixels(8, 8, pixels);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="8" Height="8" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    for decoded in texture.rgba().as_chunks::<4>().0 {
        assert!(
            decoded[0] > decoded[2],
            "a bright swapped atlas must be corrected to warm (R>B): {decoded:?}"
        );
    }
    // The enamel and the gingiva come back as the warm colours they are.
    assert_eq!(&texture.rgba()[0..4], &[235, 222, 205, 255]);
    assert_eq!(&texture.rgba()[48 * 4..48 * 4 + 4], &[200, 150, 130, 255]);
}

/// The same swapped atlas is corrected at bright, mid and dark brightness, so
/// the verdict cannot depend on brightness.
#[test]
fn a_swapped_atlas_is_corrected_at_every_brightness() {
    for (label, warm) in [
        ("bright", [235, 222, 205, 255]),
        ("mid", [180, 170, 150, 255]),
        ("dark", [150, 117, 107, 255]),
    ] {
        let swapped = [warm[2], warm[1], warm[0], warm[3]];
        assert!(
            swapped[2] > swapped[0],
            "the {label} fixture must be stored blue-biased, or it tests nothing"
        );
        let png_bytes = solid_rgba_png_bytes(4, 4, swapped, false);
        let extra = format!(
            r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="4" Height="4" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
            png_bytes.len(),
            encode_base64(&png_bytes)
        );

        let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
        let texture = mesh.texture().expect("HPS texture should be attached");
        for decoded in texture.rgba().as_chunks::<4>().0 {
            assert_eq!(
                *decoded, warm,
                "the {label} swapped atlas must come back as the warm original"
            );
        }
    }
}

/// The sample covers every column of a 4096-wide atlas: a blue edge column must
/// not invert an otherwise warm scan, and a gray edge column must not hide a
/// whole-atlas swap.
#[test]
fn the_swap_sample_covers_every_column_not_just_the_first() {
    // 4096x4096 where only the first column is blue and everything else is warm
    // gingiva: the population is overwhelmingly warm, so it must NOT be swapped.
    let width = 4096u32;
    let height = 4096u32;
    let mut pixels = vec![[200, 140, 80, 255]; (width * height) as usize];
    for row in 0..height {
        pixels[(row * width) as usize] = [40, 90, 230, 255];
    }
    let png = rgba_png_bytes_from_pixels(width, height, pixels);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="{width}" Height="{height}" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png.len(),
        encode_base64(&png)
    );
    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");
    assert_eq!(
        &texture.rgba()[0..4],
        &[40, 90, 230, 255],
        "a blue edge column must not invert an otherwise warm scan"
    );
    assert_eq!(
        &texture.rgba()[4..8],
        &[200, 140, 80, 255],
        "the rest of the atlas stays warm"
    );

    // And the reverse: only the first column is near-gray (so it carries no
    // hue), everything else is a swapped bright atlas that must be corrected.
    let mut swapped = vec![[205, 222, 235, 255]; (width * height) as usize];
    for row in 0..height {
        swapped[(row * width) as usize] = [10, 10, 10, 255];
    }
    let png = rgba_png_bytes_from_pixels(width, height, swapped);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="{width}" Height="{height}" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png.len(),
        encode_base64(&png)
    );
    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");
    assert_eq!(
        &texture.rgba()[4..8],
        &[235, 222, 205, 255],
        "a gray edge column must not hide a real whole-atlas swap"
    );
}

fn rgba_png_bytes_from_pixels(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Vec<u8> {
    let mut data = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        data.extend_from_slice(&pixel);
    }
    let img = image::RgbaImage::from_raw(width, height, data).expect("image dims");
    let mut buf = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
    buf.into_inner()
}

// A real dental scan can carry a localized patch of intensely blue material
// (anti-glare spray, bite-registration silicone) alongside otherwise-warm
// surface color. That patch alone can pull the whole-texture mean past the
// swap-detection margin even though most of the surface never reads blue, so
// the swap guard requires the bias to be near-uniform across sampled pixels
// (a real channel swap affects every pixel alike), not just present in the
// aggregate mean. Otherwise it would invert real warm gingiva/tooth color
// next to a blue material.
#[test]
fn embedded_png_with_a_localized_blue_material_patch_is_left_untouched() {
    let mut pixels = Vec::with_capacity(100);
    // Near-white teeth: filtered out by the swap guard's own near-gray skip,
    // contributing nothing to the sampled statistics.
    for _ in 0..70 {
        pixels.push([220, 218, 220, 255]);
    }
    // Real warm gingiva.
    for _ in 0..10 {
        pixels.push([200, 140, 80, 255]);
    }
    // A localized patch of intensely blue anti-glare/registration material.
    for _ in 0..20 {
        pixels.push([40, 90, 230, 255]);
    }
    let png_bytes = rgba_png_bytes_from_pixels(10, 10, pixels);
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="10" Height="10" BytesPerPixel="4" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        png_bytes.len(),
        encode_base64(&png_bytes)
    );

    let mesh = read(&cc_fixture(3, 1, &[4], &extra)).expect("textured HPS should read");
    let texture = mesh.texture().expect("HPS texture should be attached");

    // The gingiva pixels must stay warm (R>B); a global swap would flip them
    // to [80, 140, 200].
    let gingiva_pixel = &texture.rgba()[70 * 4..70 * 4 + 4];
    assert_eq!(
        gingiva_pixel,
        [200, 140, 80, 255],
        "a localized blue material patch must not swap real warm gingiva color"
    );
}

/// A structurally valid PNG whose header claims `width` x 1 grayscale.
///
/// A run of identical bytes compresses to almost nothing, which is what
/// makes an oversized header cheap to send and expensive to decode.
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
fn an_embedded_image_larger_than_the_pixel_limit_never_reaches_the_decoder() {
    // `validate_texture_dimensions` runs on an already-decoded image, so it can
    // only report a bomb that has already been allocated — inside dllhost.exe,
    // on a file Explorer passed in. The line that prevents it is
    // `reader.limits(limits)` in `decode_embedded_raster`; this test goes
    // through the real container path so it fails without that line.
    let bomb = over_wide_png(9_000);
    assert!(
        bomb.len() < 4096,
        "the fixture must stay small to be a bomb at all: {} bytes",
        bomb.len()
    );
    let extra = format!(
        r#"  <TextureData2>
    <TextureImages>
      <TextureImage TextureId="tex0" Width="9000" Height="1" BytesPerPixel="3" Base64EncodedBytes="{}">{}</TextureImage>
    </TextureImages>
  </TextureData2>
"#,
        bomb.len(),
        encode_base64(&bomb)
    );

    let result = read(&cc_fixture(3, 1, &[4], &extra));
    let Err(error) = result else {
        unreachable!("a 9000px texture must be refused");
    };
    let message = error.to_string();
    assert!(
        message.contains("decode failed"),
        "the refusal should come from the bounded decoder rather than a \
         post-decode dimension check: {message}"
    );
}

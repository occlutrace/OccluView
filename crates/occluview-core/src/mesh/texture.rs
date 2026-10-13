/// A CPU-side decoded texture image attached to a mesh (glTF `image` /
/// `texture`, decoded from PNG/JPEG/etc. by `occluview-formats`). The
/// renderer uploads this to a `wgpu::Texture` and samples it via the UV
/// channel of [`crate::Vertex`].
///
/// Stored as RGBA8 — the decoder normalizes whatever the source format was.
#[derive(Clone, Debug)]
pub struct MeshTexture {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` RGBA8 pixels, row-major top-to-bottom.
    pub rgba: Vec<u8>,
}

impl MeshTexture {
    /// Construct from decoded RGBA8 pixels. Asserts the length matches
    /// `width * height * 4`.
    ///
    /// # Panics
    ///
    /// Panics when the RGBA byte count overflows `usize` or `rgba.len()` differs
    /// from `width * height * 4`. The renderer indexes the buffer by these
    /// dimensions, so a shorter buffer would read past its end.
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        assert_eq!(
            expected_len,
            Some(rgba.len()),
            "texture dimensions overflow RGBA8 byte length or buffer length does not match"
        );
        Self {
            width,
            height,
            rgba,
        }
    }

    /// A 1×1 opaque white texture — the neutral fallback used when a mesh has
    /// UVs but no decoded image, or when the renderer needs a bound texture
    /// for an untextured mesh (so the shader's textureless branch still has a
    /// valid binding).
    #[must_use]
    pub fn white_1x1() -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        }
    }
}

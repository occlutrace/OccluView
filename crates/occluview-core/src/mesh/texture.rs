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
    #[must_use]
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        debug_assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
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

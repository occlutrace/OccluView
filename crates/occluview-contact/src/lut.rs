//! Packing the signed field into a GPU texture.
//!
//! The renderer reads the field in its vertex stage and passes the value to the
//! fragment stage as an interpolated varying, so the field travels as a texture
//! rather than as a vertex attribute: the core vertex format is fixed at 36
//! bytes and shared with every other layer in the scene.
//!
//! `Rgba8Unorm` is the format every wgpu adapter can filter, and filtering is
//! what makes the interpolated field smooth. There is no 32-bit float format
//! with the same guarantee, which is why the field's bits are stored in four
//! unsigned bytes rather than in a float texture: the value survives the round
//! trip exactly, and the only thing the hardware is allowed to do to it is
//! interpolate, which is what a per-fragment colour lookup needs.
//!
//! Byte 0 of a texel is the least significant byte of the little-endian `f32`,
//! so the decode on the GPU is a shift-and-or and a `bitcast`, with no per-texel
//! arithmetic that a driver could reorder.

/// What a vertex with no opposing surface is packed as, in millimetres.
///
/// Not infinity, and not zero. Infinity because `±∞` inside an interpolated
/// varying produces `0 × ∞` in the fragment stage — a single bad fragment can
/// take a whole triangle with it — and zero because zero is a real reading:
/// exact touch, fully painted. So the sentinel is a finite value beyond every
/// law's far edge *and* feather, in the region the paint weight already
/// reports as nothing to paint. It is also inside the search radius, so
/// a caller that reads the packed field back during a hover gets a plausible
/// "nothing here" distance rather than a number it has to special-case.
pub const FIELD_FAR_SENTINEL_MM: f32 = 0.5;

/// One packed field, ready to become an `Rgba8Unorm` texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldTexels {
    /// `width * height * 4` bytes: one texel per field value, then padding.
    pub rgba: Vec<u8>,
    /// Texels per row. Also the row stride the vertex stage decodes with:
    /// vertex `i` lives at `(i % width, i / width)`.
    pub width: u32,
    /// Rows in the texture.
    pub height: u32,
}

/// Pack a signed field, one texel per vertex value, row-major.
///
/// `max_width` is the caller's device limit or its preferred row length; the
/// width is narrowed to the value count so a small field does not pay for a
/// wide texture. The last row is padded with [`FIELD_FAR_SENTINEL_MM`] rather
/// than left undefined: a padding texel that decodes as a huge penetration
/// depth would paint a stray mark if the vertex stage ever read past the end,
/// and "nothing here" is the only harmless thing to be.
pub fn pack_field_texels(values: &[f32], max_width: u32) -> FieldTexels {
    let count = values.len();
    if count == 0 {
        return FieldTexels {
            rgba: sentinel_bytes(1),
            width: 1,
            height: 1,
        };
    }
    let width =
        usize::try_from(max_width.clamp(1, u32::try_from(count).unwrap_or(u32::MAX))).unwrap_or(1);
    let height = count.div_ceil(width);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for value in values {
        rgba.extend_from_slice(&pack_value(*value));
    }
    for _ in count..(width * height) {
        rgba.extend_from_slice(&pack_value(FIELD_FAR_SENTINEL_MM));
    }
    FieldTexels {
        rgba,
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
    }
}

/// One texel: the little-endian bytes of the value, or of the sentinel when
/// there is no measurement to pack.
fn pack_value(value: f32) -> [u8; 4] {
    let packed = if value.is_finite() {
        value
    } else {
        FIELD_FAR_SENTINEL_MM
    };
    packed.to_le_bytes()
}

/// The four bytes of the sentinel, for a texture that carries no field at all.
fn sentinel_bytes(texels: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(texels * 4);
    for _ in 0..texels {
        rgba.extend_from_slice(&pack_value(FIELD_FAR_SENTINEL_MM));
    }
    rgba
}

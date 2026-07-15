//! Pack a rendered frame into a packed `CF_DIB` payload for the clipboard.
//!
//! `SetClipboardData(CF_DIB, ...)` wants a single memory block: a
//! `BITMAPINFOHEADER` immediately followed by the pixel bits (no file header).
//! We emit the most broadly pasteable variant — 32bpp `BI_RGB`, **bottom-up**
//! (positive `biHeight`), opaque — which Word, Chrome, Paint, Snip, and chat
//! apps all accept. The byte layout is the risky part, so it lives here as a
//! pure function unit tested on any host; the Windows layer only allocates an
//! `HGLOBAL` and copies these bytes into it.

// The header packs a fixed 40-byte layout plus preview-pane dimensions into the
// integer widths `BITMAPINFOHEADER` mandates; all values are small and bounded.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

/// Size of a `BITMAPINFOHEADER` in bytes.
const HEADER_LEN: usize = 40;

/// Build a packed bottom-up 32bpp `CF_DIB` from top-down RGBA pixels.
///
/// Returns `None` if the dimensions are zero or do not match the buffer.
pub(crate) fn pack_clipboard_dib(rgba_top_down: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || rgba_top_down.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }

    let stride = w * 4;
    let image_bytes = stride * h;
    let mut dib = vec![0u8; HEADER_LEN + image_bytes];

    // BITMAPINFOHEADER (little-endian).
    dib[0..4].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes()); // biSize
    dib[4..8].copy_from_slice(&(width as i32).to_le_bytes()); // biWidth
    dib[8..12].copy_from_slice(&(height as i32).to_le_bytes()); // biHeight (+ = bottom-up)
    dib[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib[14..16].copy_from_slice(&32u16.to_le_bytes()); // biBitCount
    dib[16..20].copy_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    dib[20..24].copy_from_slice(&(image_bytes as u32).to_le_bytes()); // biSizeImage
                                                                      // biXPelsPerMeter / biYPelsPerMeter / biClrUsed / biClrImportant stay 0.

    // Pixels: bottom-up rows, RGBA -> BGRA, forced opaque.
    for row in 0..h {
        let src_row = h - 1 - row;
        for col in 0..w {
            let s = (src_row * w + col) * 4;
            let d = HEADER_LEN + (row * w + col) * 4;
            dib[d] = rgba_top_down[s + 2]; // B
            dib[d + 1] = rgba_top_down[s + 1]; // G
            dib[d + 2] = rgba_top_down[s]; // R
            dib[d + 3] = 255; // A (opaque)
        }
    }

    Some(dib)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_dimensions() {
        assert!(pack_clipboard_dib(&[0; 16], 0, 4).is_none());
        assert!(pack_clipboard_dib(&[0; 16], 2, 3).is_none());
        assert!(pack_clipboard_dib(&[], 1, 1).is_none());
    }

    #[test]
    fn header_describes_a_bottom_up_32bpp_dib() {
        let rgba = vec![0u8; 2 * 2 * 4];
        let dib = pack_clipboard_dib(&rgba, 2, 2).expect("2x2 should pack");
        assert_eq!(dib.len(), HEADER_LEN + 2 * 2 * 4);
        assert_eq!(
            u32::from_le_bytes(dib[0..4].try_into().expect("fixed-width slice")),
            40
        );
        assert_eq!(
            i32::from_le_bytes(dib[4..8].try_into().expect("fixed-width slice")),
            2
        );
        // Positive height signals a bottom-up DIB.
        assert_eq!(
            i32::from_le_bytes(dib[8..12].try_into().expect("fixed-width slice")),
            2
        );
        assert_eq!(
            u16::from_le_bytes(dib[12..14].try_into().expect("fixed-width slice")),
            1
        );
        assert_eq!(
            u16::from_le_bytes(dib[14..16].try_into().expect("fixed-width slice")),
            32
        );
        assert_eq!(
            u32::from_le_bytes(dib[16..20].try_into().expect("fixed-width slice")),
            0
        );
        assert_eq!(
            u32::from_le_bytes(dib[20..24].try_into().expect("fixed-width slice")),
            16
        );
    }

    #[test]
    fn swizzles_to_bgra_and_flips_rows() {
        // Top row: red, green. Bottom row: blue, white (all top-down RGBA).
        let rgba = vec![
            255, 0, 0, 200, // (0,0) red
            0, 255, 0, 200, // (1,0) green
            0, 0, 255, 200, // (0,1) blue
            255, 255, 255, 200, // (1,1) white
        ];
        let dib = pack_clipboard_dib(&rgba, 2, 2).expect("2x2 should pack");
        let px = |row: usize, col: usize| {
            let d = HEADER_LEN + (row * 2 + col) * 4;
            [dib[d], dib[d + 1], dib[d + 2], dib[d + 3]]
        };
        // DIB row 0 is the image's BOTTOM row (blue, white), stored BGRA opaque.
        assert_eq!(px(0, 0), [255, 0, 0, 255], "blue -> BGRA opaque");
        assert_eq!(px(0, 1), [255, 255, 255, 255], "white -> BGRA opaque");
        // DIB row 1 is the image's TOP row (red, green).
        assert_eq!(px(1, 0), [0, 0, 255, 255], "red -> BGRA opaque");
        assert_eq!(px(1, 1), [0, 255, 0, 255], "green -> BGRA opaque");
    }
}

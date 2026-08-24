//! Placing a square render on the preview pane's canvas.
//!
//! Platform-free on purpose: it is pixel arithmetic, and the preview pane is
//! the one surface where getting it wrong is visible rather than merely slow.

/// Draw a square image centred on a canvas of the pane's size.
///
/// The square is composited, not copied. A rendered scan is opaque where the
/// mesh is and transparent everywhere else, and the placeholder cube -- what a
/// file that cannot be read shows -- is transparent everywhere but the cube.
/// Copying those pixels put the placeholder's transparent black over the
/// canvas, so an unreadable scan appeared as a black square the height of the
/// pane on a light-grey background. In dark mode the canvas is already black,
/// which is why it went unseen.
#[must_use]
pub(crate) fn center_square_on_canvas(
    square: &[u8],
    side_px: u16,
    width: u32,
    height: u32,
    background: [u8; 4],
) -> Vec<u8> {
    let width = width.max(1) as usize;
    let height = height.max(1) as usize;
    let side = usize::from(side_px).min(width).min(height).max(1);
    let mut canvas = vec![0u8; width * height * 4];
    for pixel in canvas.chunks_exact_mut(4) {
        pixel.copy_from_slice(&background);
    }
    if square.len() < side * side * 4 {
        return canvas;
    }
    let x0 = (width - side) / 2;
    let y0 = (height - side) / 2;
    for y in 0..side {
        for x in 0..side {
            let src = (y * side + x) * 4;
            let dst = ((y0 + y) * width + x0 + x) * 4;
            let alpha = u32::from(square[src + 3]);
            if alpha == 0 {
                continue;
            }
            if alpha == 255 {
                canvas[dst..dst + 4].copy_from_slice(&square[src..src + 4]);
                continue;
            }
            for channel in 0..3 {
                let over = u32::from(square[src + channel]) * alpha;
                let under = u32::from(canvas[dst + channel]) * (255 - alpha);
                // The sum is at most 255 * 255, so the quotient is a byte.
                canvas[dst + channel] = u8::try_from((over + under + 127) / 255).unwrap_or(255);
            }
            canvas[dst + 3] = 255;
        }
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::center_square_on_canvas;

    const LIGHT: [u8; 4] = [204, 209, 214, 255];

    #[test]
    fn a_transparent_square_leaves_the_canvas_as_it_was() {
        // The placeholder cube's background, and the empty margin around any
        // rendered scan. Copied rather than composited, this was the black
        // square operators saw behind a file the viewer could not read.
        let square = vec![0u8; 2 * 2 * 4];
        let canvas = center_square_on_canvas(&square, 2, 4, 4, LIGHT);
        for pixel in canvas.chunks_exact(4) {
            assert_eq!(pixel, LIGHT, "a fully transparent square painted nothing");
        }
    }

    #[test]
    fn an_opaque_pixel_lands_exactly_where_it_was_drawn() {
        let mut square = vec![0u8; 2 * 2 * 4];
        square[0..4].copy_from_slice(&[10, 20, 30, 255]);
        let canvas = center_square_on_canvas(&square, 2, 4, 4, LIGHT);
        let top_left = (4 + 1) * 4;
        assert_eq!(&canvas[top_left..top_left + 4], &[10, 20, 30, 255]);
    }

    #[test]
    fn a_half_covered_pixel_meets_the_canvas_halfway() {
        let mut square = vec![0u8; 4];
        square.copy_from_slice(&[0, 0, 0, 128]);
        let canvas = center_square_on_canvas(&square, 1, 1, 1, LIGHT);
        // Black at half coverage over 204: 204 * 127 / 255 ~= 101.
        assert!(
            (100..=103).contains(&canvas[0]),
            "expected the two to mix, got {}",
            canvas[0]
        );
        assert_eq!(canvas[3], 255, "the pane's canvas stays opaque");
    }
}

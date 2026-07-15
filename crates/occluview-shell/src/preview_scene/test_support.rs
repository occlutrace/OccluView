use std::fmt::Write as _;

pub(super) fn binary_stl(triangles: &[[f32; 12]]) -> Vec<u8> {
    let mut out = vec![0u8; 84];
    out[80..84].copy_from_slice(&(triangles.len() as u32).to_le_bytes());
    for triangle in triangles {
        for f in triangle {
            out.extend_from_slice(&f.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

pub(super) fn binary_stl_triangle() -> Vec<u8> {
    binary_stl(&[[
        0.0, 0.0, 1.0, //
        0.0, 0.0, 0.0, //
        1.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, //
    ]])
}

pub(super) fn binary_stl_preview_smoke_mesh() -> Vec<u8> {
    binary_stl(&[
        [
            0.0, -0.6, 0.8, //
            -1.2, -0.7, -0.4, //
            1.4, -0.9, -0.6, //
            0.1, 1.1, 0.2, //
        ],
        [
            -0.8, 0.3, 0.9, //
            -1.2, -0.7, -0.4, //
            0.1, 1.1, 0.2, //
            -0.5, 0.0, 1.6, //
        ],
        [
            0.9, 0.2, 0.7, //
            0.1, 1.1, 0.2, //
            1.4, -0.9, -0.6, //
            -0.5, 0.0, 1.6, //
        ],
        [
            0.0, -1.0, -0.2, //
            1.4, -0.9, -0.6, //
            -1.2, -0.7, -0.4, //
            -0.3, -0.2, -1.0, //
        ],
    ])
}

/// Vertically asymmetric fixture for orientation/parity tests: a small marker
/// triangle at world **+Y** (the "top") floating clearly above a big low blob
/// spanning world `y ∈ [-2.0, -0.2]`. All faces point +Z so the `Front`
/// preset (look -Z, up +Y) lights them. With a correct app-convention present
/// path the marker lands in the TOP rows; a vertical mirror sends it to the
/// bottom — which is exactly the inverted-orbit defect this pins.
pub(super) fn binary_stl_marker_above_blob() -> Vec<u8> {
    binary_stl(&[
        // Big low blob (two triangles forming a quad), world-bottom.
        [
            0.0, 0.0, 1.0, -2.0, -2.0, 0.0, 2.0, -2.0, 0.0, 2.0, -0.2, 0.0,
        ],
        [
            0.0, 0.0, 1.0, -2.0, -2.0, 0.0, 2.0, -0.2, 0.0, -2.0, -0.2, 0.0,
        ],
        // Small marker triangle near +Y, world-top.
        [
            0.0, 0.0, 1.0, -0.25, 1.6, 0.0, 0.25, 1.6, 0.0, 0.0, 1.95, 0.0,
        ],
    ])
}

/// Row index (0 = top) of the centroid of the lit pixels that fall inside the
/// requested vertical half of an RGBA frame — used to locate the marker
/// (top half) or the blob (bottom half) of [`binary_stl_marker_above_blob`].
/// Returns `None` if that half has no lit pixel.
// Row indices are small preview dimensions (< 2^16), exactly representable in
// f32, so the centroid cast is lossless in practice.
#[allow(clippy::cast_precision_loss)]
pub(super) fn lit_centroid_row_in_half(
    rgba: &[u8],
    width: usize,
    height: usize,
    top_half: bool,
) -> Option<f32> {
    let (lo, hi) = if top_half {
        (0, height / 2)
    } else {
        (height / 2, height)
    };
    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    for row in lo..hi {
        for col in 0..width {
            let i = (row * width + col) * 4;
            let lit = u16::from(rgba[i]) + u16::from(rgba[i + 1]) + u16::from(rgba[i + 2]) > 24;
            if lit {
                sum += row as f32;
                count += 1.0;
            }
        }
    }
    (count > 0.0).then(|| sum / count)
}

pub(super) fn valid_obj_tiles(min_bytes: usize) -> Vec<u8> {
    let mut out = String::with_capacity(min_bytes + (min_bytes / 8).max(1024));
    let mut base_index = 1u32;
    let mut tile = 0u32;
    while out.len() <= min_bytes {
        let x = f32::from(u16::try_from(tile % 256).expect("bounded tile x")) * 0.30;
        let y = f32::from(u16::try_from(tile / 256).expect("bounded tile y")) * 0.30;
        let z = f32::from(u8::try_from(tile % 11).expect("bounded tile z")) * 0.03;
        for (dx, dy, dz, r, g, b) in [
            (0.0, 0.0, 0.0, 220, 180, 105),
            (0.22, 0.0, 0.0, 215, 170, 96),
            (0.22, 0.22, 0.0, 205, 160, 88),
            (0.0, 0.22, 0.0, 210, 166, 92),
        ] {
            let _ = writeln!(out, "v {} {} {} {} {} {}", x + dx, y + dy, z + dz, r, g, b);
        }
        let _ = writeln!(
            out,
            "f {} {} {}",
            base_index,
            base_index + 1,
            base_index + 2
        );
        let _ = writeln!(
            out,
            "f {} {} {}",
            base_index,
            base_index + 2,
            base_index + 3
        );
        base_index += 4;
        tile += 1;
    }
    out.into_bytes()
}

pub(super) fn obj_with_early_faces(min_bytes: usize) -> Vec<u8> {
    let mut out = String::with_capacity(min_bytes + (min_bytes / 8).max(1024));
    out.push_str("# OccluView preview OBJ stress fixture\n");
    out.push_str("mtllib missing-materials.mtl\nusemtl scan\n");
    let mut base_index = 1u32;
    let mut tile = 0u32;
    while out.len() < min_bytes {
        let _ = writeln!(
            out,
            "f {}/{} {}/{} {}/{}",
            base_index,
            base_index,
            base_index + 1,
            base_index + 1,
            base_index + 2,
            base_index + 2
        );
        let x = f32::from(u8::try_from(tile % 192).expect("bounded tile x")) * 0.25;
        let y = f32::from(u16::try_from(tile / 192).expect("bounded tile y")) * 0.25;
        let z = f32::from(u8::try_from(tile % 17).expect("bounded tile z")) * 0.02;
        for (dx, dy, r, g, b) in [
            (0.0, 0.0, 220, 180, 105),
            (0.18, 0.0, 215, 170, 96),
            (0.0, 0.18, 235, 196, 122),
        ] {
            let _ = writeln!(out, "vt {dx} {dy}");
            let _ = writeln!(out, "v {} {} {} {} {} {}", x + dx, y + dy, z, r, g, b);
        }
        base_index += 3;
        tile += 1;
    }
    out.into_bytes()
}

pub(super) fn noisy_obj_with_early_faces(min_bytes: usize) -> Vec<u8> {
    let mut out = Vec::from(&b"\xef\xbb\xbf# scanner metadata "[..]);
    out.extend_from_slice(&[0xff, 0xfe, b'\n']);
    let body_target = min_bytes.saturating_sub(out.len());
    out.extend_from_slice(&obj_with_early_faces(body_target));
    while out.len() < min_bytes {
        out.extend_from_slice(b"# preview padding\n");
    }
    out
}

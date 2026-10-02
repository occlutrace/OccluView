use glam::DVec3;

/// Use exact source coordinates for STL welding while treating signed zero as
/// the same geometric point. A tolerance would merge nearby anatomy and make
/// the coarse hypothesis less trustworthy; loaders already preserve repeated
/// STL coordinates bit-for-bit.
pub(super) fn canonical_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

/// Read one triangle's vertices, rejecting out-of-range or non-finite input.
pub(super) fn read_triangle(
    positions: &[f32],
    vertex_count: usize,
    slice: &[u32],
) -> Option<[DVec3; 3]> {
    let mut out = [DVec3::ZERO; 3];
    for (slot, &raw) in slice.iter().enumerate() {
        let vertex = usize::try_from(raw).ok()?;
        if vertex >= vertex_count {
            return None;
        }
        let xyz = positions.get(vertex * 3..vertex * 3 + 3)?;
        let point = DVec3::new(f64::from(xyz[0]), f64::from(xyz[1]), f64::from(xyz[2]));
        if !point.is_finite() {
            return None;
        }
        out[slot] = point;
    }
    Some(out)
}

/// Length of a triangle's longest edge.
pub(super) fn longest_edge(corners: &[DVec3; 3]) -> f64 {
    let a = (corners[1] - corners[0]).length();
    let b = (corners[2] - corners[1]).length();
    let c = (corners[0] - corners[2]).length();
    a.max(b).max(c)
}

/// Grid dimensions covering `extent` at `cell`, at least one cell per axis.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn grid_dims(extent: DVec3, cell: f64) -> [i64; 3] {
    let mut dims = [1i64; 3];
    for (dim, raw) in dims.iter_mut().zip(extent.to_array()) {
        let span = if raw.is_finite() { raw.max(0.0) } else { 0.0 };
        *dim = ((span / cell).floor() as i64 + 1).max(1);
    }
    dims
}

/// Total cell count, saturating instead of overflowing on absurd dimensions.
pub(super) fn cell_count(dims: [i64; 3]) -> usize {
    let product = dims[0]
        .saturating_mul(dims[1])
        .saturating_mul(dims[2])
        .max(1);
    usize::try_from(product).unwrap_or(usize::MAX)
}

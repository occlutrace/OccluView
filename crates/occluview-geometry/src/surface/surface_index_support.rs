//! Sampling, component mapping, and coarse-grid support for the surface index.

use super::{GeometryControl, GeometryStop, BLOCK};

/// The half of the 3x3x3 neighbourhood a forward raster sweep has already
/// visited, ending with the cell before this one on the same row.
const EARLIER: [[i64; 3]; 13] = [
    [-1, -1, -1],
    [0, -1, -1],
    [1, -1, -1],
    [-1, 0, -1],
    [0, 0, -1],
    [1, 0, -1],
    [-1, 1, -1],
    [0, 1, -1],
    [1, 1, -1],
    [-1, -1, 0],
    [0, -1, 0],
    [1, -1, 0],
    [-1, 0, 0],
];

#[allow(clippy::cast_precision_loss)]
pub(super) fn radical_inverse(mut index: usize, base: usize) -> f64 {
    let mut result = 0.0;
    let mut scale = 1.0 / base as f64;
    while index > 0 {
        result += (index % base) as f64 * scale;
        index /= base;
        scale /= base as f64;
    }
    result
}

/// Coarse blocks spanning `cells` fine cells, rounding up.
pub(super) fn block_count(cells: i64) -> i64 {
    (cells + BLOCK - 1) / BLOCK
}

/// Flat index of a coordinate in a grid of `dims`, or `None` when it falls
/// outside.
pub(super) fn flat_index(dims: [i64; 3], cell: [i64; 3]) -> Option<usize> {
    if cell
        .iter()
        .zip(dims)
        .any(|(&value, dim)| value < 0 || value >= dim)
    {
        return None;
    }
    usize::try_from((cell[2] * dims[1] + cell[1]) * dims[0] + cell[0]).ok()
}

/// The coordinate a flat index stands for.
pub(super) fn unflatten(dims: [i64; 3], flat: i64) -> [i64; 3] {
    let plane = dims[0] * dims[1];
    [flat % dims[0], (flat / dims[0]) % dims[1], flat / plane]
}

/// One raster sweep of the chessboard distance transform over `gaps`.
///
/// A chessboard distance is exactly the number of single steps through the
/// 3x3x3 neighbourhood, so a forward sweep against the already-visited half of
/// that neighbourhood followed by a backward sweep against the other half is
/// exact — no iteration to a fixed point, and the same answer every run.
pub(super) fn sweep(
    dims: [i64; 3],
    gaps: &mut [u8],
    forward: bool,
    control: &GeometryControl,
) -> Result<(), GeometryStop> {
    let total = i64::try_from(gaps.len()).unwrap_or(0);
    for step in 0..total {
        control.charge_operations(1)?;
        let flat = if forward { step } else { total - 1 - step };
        let Some(&current) = usize::try_from(flat).ok().and_then(|slot| gaps.get(slot)) else {
            continue;
        };
        if current == 0 {
            continue;
        }
        let cell = unflatten(dims, flat);
        let sign = if forward { 1 } else { -1 };
        let mut best = current;
        for offset in EARLIER {
            control.charge_operations(1)?;
            let neighbour = [
                cell[0] + sign * offset[0],
                cell[1] + sign * offset[1],
                cell[2] + sign * offset[2],
            ];
            if let Some(&found) = flat_index(dims, neighbour).and_then(|slot| gaps.get(slot)) {
                best = best.min(found.saturating_add(1));
            }
        }
        if let Some(entry) = usize::try_from(flat)
            .ok()
            .and_then(|slot| gaps.get_mut(slot))
        {
            *entry = best;
        }
    }
    Ok(())
}

/// Find a connected-component root with path compression.
///
/// Iterative, not recursive, as cheap insurance against deep trees: a
/// recursive form needs one stack frame per level, and a stack overflow
/// aborts the process instead of unwinding, so `catch_unwind` around a
/// worker body would never see it.
///
/// `union` below attaches the smaller tree under the larger (union by size),
/// so every parent hop at least doubles the component size and the depth
/// stays logarithmic in the vertex count even before compression.
///
/// Two passes instead of one: walk to the root, then point every node on the
/// path at it. Same compression, no stack.
pub(super) fn find(parent: &mut [usize], node: usize) -> usize {
    let mut root = node;
    while parent[root] != root {
        root = parent[root];
    }
    let mut current = node;
    while parent[current] != current {
        let next = parent[current];
        parent[current] = root;
        current = next;
    }
    root
}

/// Join two indexed vertices into one surface component.
///
/// Attaches the smaller tree under the larger, which is what keeps the depth
/// logarithmic even before `find` compresses anything — the counterpart to the
/// iterative `find`: together they bound the walk instead of relying on path
/// compression to rescue a linear chain on the first call.
pub(super) fn union(parent: &mut [usize], size: &mut [usize], left: usize, right: usize) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root == right_root {
        return;
    }
    let (large, small) = if size[left_root] >= size[right_root] {
        (left_root, right_root)
    } else {
        (right_root, left_root)
    };
    parent[small] = large;
    size[large] += size[small];
}

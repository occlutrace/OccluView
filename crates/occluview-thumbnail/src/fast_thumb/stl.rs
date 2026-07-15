use super::{
    sample_stride, FormatError, Mesh, RobustBoundsSampler, SurfaceGridCluster, Vec3, Vertex,
    FAST_CLUSTER_GRID, ROBUST_BOUNDS_SAMPLE_LIMIT, STL_FIRST_TRIANGLE_OFFSET, STL_HEADER_SIZE,
    STL_TRIANGLE_SIZE,
};

pub(super) fn fast_binary_stl_thumbnail_mesh(bytes: &[u8]) -> Result<Mesh, FormatError> {
    if bytes.len() < STL_FIRST_TRIANGLE_OFFSET {
        return Err(FormatError::Truncated {
            format: "STL (binary)",
            expected: STL_FIRST_TRIANGLE_OFFSET,
            got: bytes.len(),
        });
    }
    let count_bytes: [u8; 4] = bytes[STL_HEADER_SIZE..STL_FIRST_TRIANGLE_OFFSET]
        .try_into()
        .map_err(|_| FormatError::Malformed {
            format: "STL (binary)",
            offset: STL_HEADER_SIZE,
            reason: "count field is not 4 bytes".to_string(),
        })?;
    let declared_count = u32::from_le_bytes(count_bytes) as usize;
    let actual_count = actual_stl_triangle_count(bytes, declared_count)?;
    if actual_count == 0 {
        return Err(FormatError::Truncated {
            format: "STL (binary)",
            expected: STL_FIRST_TRIANGLE_OFFSET + STL_TRIANGLE_SIZE,
            got: bytes.len(),
        });
    }

    // Contiguous decimation: read EVERY triangle (never stride — striding is the
    // see-through speckle), welding vertices onto a coarse grid so the reduced
    // mesh stays a solid, opaque surface. Two passes over the fixed-size records
    // (robust bounds first, then cluster) keep this O(triangles) and
    // allocation-light regardless of how many millions of triangles the file
    // declares.
    let (min, max) = stl_robust_bounds(bytes, actual_count)?;
    let mut cluster = SurfaceGridCluster::new("STL", min, max, FAST_CLUSTER_GRID);

    for triangle_index in 0..actual_count {
        let (normal, a, b, c) = decode_stl_triangle(bytes, triangle_index)?;
        let color = [255, 255, 255, 255];
        cluster.push_triangle(
            Vertex {
                position: a,
                normal,
                color,
                uv: [0.0, 0.0],
            },
            Vertex {
                position: b,
                normal,
                color,
                uv: [0.0, 0.0],
            },
            Vertex {
                position: c,
                normal,
                color,
                uv: [0.0, 0.0],
            },
        );
    }

    // Zero-triangle safety net: the file declared triangles (actual_count > 0)
    // but none survived clustering (all degenerate / non-finite). Building here
    // would yield a triangle mesh with 0 triangles that renders as a fully
    // transparent tile. Decline instead (mirroring how the surface-PLY fast path
    // declines) so the loader falls through to the full reader; if that is also
    // blank, loading.rs's renderability guard surfaces the corrupt placeholder.
    if cluster.triangle_count() == 0 {
        return Err(FormatError::Deferred {
            format: "STL (binary)",
            reason: "clustered fast surrogate produced no surface; defer to the full reader"
                .to_string(),
        });
    }

    cluster.build()
}

/// A decoded STL triangle: shared face normal plus three corner positions.
type StlTriangle = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// Decode one triangle record into its shared normal and three corner
/// positions. `triangle_index` must be `< actual_count`.
fn decode_stl_triangle(bytes: &[u8], triangle_index: usize) -> Result<StlTriangle, FormatError> {
    let start = STL_FIRST_TRIANGLE_OFFSET + triangle_index * STL_TRIANGLE_SIZE;
    let record = bytes
        .get(start..start + STL_TRIANGLE_SIZE)
        .ok_or(FormatError::Truncated {
            format: "STL (binary)",
            expected: start + STL_TRIANGLE_SIZE,
            got: bytes.len(),
        })?;
    let mut floats = [0.0_f32; 12];
    for (slot, chunk) in floats.iter_mut().zip(record[..48].chunks_exact(4)) {
        let arr: [u8; 4] = chunk.try_into().map_err(|_| FormatError::Malformed {
            format: "STL (binary)",
            offset: start,
            reason: "float field is not 4 bytes".to_string(),
        })?;
        *slot = f32::from_le_bytes(arr);
    }
    Ok((
        [floats[0], floats[1], floats[2]],
        [floats[3], floats[4], floats[5]],
        [floats[6], floats[7], floats[8]],
        [floats[9], floats[10], floats[11]],
    ))
}

/// Outlier-robust bounds for grid sizing, over a deterministic strided sample
/// of the triangle corners. Non-finite corners are skipped; a few far outliers
/// are trimmed (see [`RobustBoundsSampler`]) so they cannot inflate the grid.
fn stl_robust_bounds(bytes: &[u8], count: usize) -> Result<(Vec3, Vec3), FormatError> {
    // Stride so the sample stays within the budget (3 corners per triangle).
    let stride = sample_stride(count, ROBUST_BOUNDS_SAMPLE_LIMIT / 3);
    let mut sampler = RobustBoundsSampler::with_capacity(ROBUST_BOUNDS_SAMPLE_LIMIT);
    let mut triangle_index = 0;
    while triangle_index < count {
        let (_, a, b, c) = decode_stl_triangle(bytes, triangle_index)?;
        sampler.push(a);
        sampler.push(b);
        sampler.push(c);
        triangle_index += stride;
    }
    sampler.finish().ok_or(FormatError::Malformed {
        format: "STL (binary)",
        offset: STL_FIRST_TRIANGLE_OFFSET,
        reason: "no finite triangle vertices".to_string(),
    })
}

fn actual_stl_triangle_count(bytes: &[u8], declared_count: usize) -> Result<usize, FormatError> {
    let declared_end = STL_FIRST_TRIANGLE_OFFSET + declared_count * STL_TRIANGLE_SIZE;
    if bytes.len() >= declared_end {
        return Ok(declared_count);
    }

    let payload_len = bytes.len() - STL_FIRST_TRIANGLE_OFFSET;
    let available = payload_len / STL_TRIANGLE_SIZE;
    let trailing = payload_len % STL_TRIANGLE_SIZE;
    if trailing != 0 {
        return Err(FormatError::Truncated {
            format: "STL (binary)",
            expected: STL_FIRST_TRIANGLE_OFFSET + (available + 1) * STL_TRIANGLE_SIZE,
            got: bytes.len(),
        });
    }
    Ok(available)
}

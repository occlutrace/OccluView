use super::MeshEditError;

/// The three undirected edge keys of one triangle.
pub(crate) fn triangle_edge_keys(triangle: &[u32]) -> [(u32, u32); 3] {
    let (a, b, c) = (triangle[0], triangle[1], triangle[2]);
    [
        (a.min(b), a.max(b)),
        (b.min(c), b.max(c)),
        (c.min(a), c.max(a)),
    ]
}

/// Convert a raw `u32` mesh index to `usize`, with a uniform typed error.
///
/// `position` is the index's offset in whatever buffer it came from, folded
/// into the error message for diagnosability.
pub(crate) fn vertex_index(raw: u32, position: usize) -> Result<usize, MeshEditError> {
    usize::try_from(raw).map_err(|_| MeshEditError::MalformedMesh {
        reason: format!("triangle index {raw} at position {position} does not fit in usize"),
    })
}

use super::adjacency::vertex_index;
use super::{EditVertex, MeshEditError};

/// Copy surviving vertices into a new dense buffer without welding
/// same-position vertices.
///
/// This copies the full [`EditVertex`] payload so color and UV survive
/// unchanged. Texture/image payloads are intentionally not part of this crate;
/// adapters carry those through their own mesh APIs.
///
/// # Errors
/// Returns [`MeshEditError::MalformedMesh`] if a survivor index is invalid.
pub fn copy_surviving_vertices(
    vertices: &[EditVertex],
    surviving_vertex_indices: &[usize],
) -> Result<(Vec<EditVertex>, Vec<Option<u32>>), MeshEditError> {
    let mut copied_vertices = Vec::with_capacity(surviving_vertex_indices.len());
    let mut remap = vec![None; vertices.len()];

    for (new_index, &old_index) in surviving_vertex_indices.iter().enumerate() {
        let vertex = vertices
            .get(old_index)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: format!(
                    "surviving vertex index {old_index} is out of range for vertex_count {}",
                    vertices.len()
                ),
            })?;
        copied_vertices.push(*vertex);
        let new_index = u32::try_from(new_index).map_err(|_| MeshEditError::MalformedMesh {
            reason: "remapped vertex count exceeds u32::MAX".to_string(),
        })?;
        remap[old_index] = Some(new_index);
    }

    Ok((copied_vertices, remap))
}

/// Remap triangle indices through a dense vertex remap.
///
/// # Errors
/// Returns [`MeshEditError::MalformedMesh`] if an index points to a removed or
/// out-of-range vertex.
pub fn remap_triangle_indices(
    indices: &[u32],
    vertex_remap: &[Option<u32>],
) -> Result<Vec<u32>, MeshEditError> {
    let mut remapped = Vec::with_capacity(indices.len());
    for (tri_index, &index) in indices.iter().enumerate() {
        let old_index = vertex_index(index, tri_index)?;
        let new_index = vertex_remap
            .get(old_index)
            .and_then(|slot| *slot)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: format!(
                    "triangle index {index} at position {tri_index} refers to a removed vertex"
                ),
            })?;
        remapped.push(new_index);
    }
    Ok(remapped)
}

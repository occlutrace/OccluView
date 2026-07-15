use super::adjacency::vertex_index;
use super::{EditVertex, FaceSelection, MeshEditError, MeshEditOptions, MeshTopology};

/// Validate raw triangle mesh data.
///
/// # Errors
/// Returns [`MeshEditError::MalformedMesh`] if index count or ranges are invalid.
pub fn validate_triangle_mesh_data(
    vertices: &[EditVertex],
    indices: &[u32],
) -> Result<(), MeshEditError> {
    if indices.len() % 3 != 0 {
        return Err(MeshEditError::MalformedMesh {
            reason: format!("index count {} is not a multiple of 3", indices.len()),
        });
    }

    let vertex_count = vertices.len();
    for (at_index, &value) in indices.iter().enumerate() {
        let index = vertex_index(value, at_index)?;
        if index >= vertex_count {
            return Err(MeshEditError::MalformedMesh {
                reason: format!(
                    "index {value} at position {at_index} is out of range for vertex_count {vertex_count}"
                ),
            });
        }
    }

    Ok(())
}

/// Validate a mesh-like buffer for face-edit operations.
///
/// # Errors
/// Returns typed errors for unsupported point clouds or malformed triangles.
pub fn validate_face_edit_buffers(
    topology: MeshTopology,
    vertices: &[EditVertex],
    indices: &[u32],
) -> Result<(), MeshEditError> {
    if topology == MeshTopology::PointCloud {
        return Err(MeshEditError::UnsupportedPointCloud);
    }
    validate_triangle_mesh_data(vertices, indices)
}

/// Validate a selection against a triangle count.
///
/// # Errors
/// Returns [`MeshEditError::InvalidSelectionLength`] on mismatch.
pub fn validate_selection_against_triangle_count(
    triangle_count: usize,
    selection: &FaceSelection,
) -> Result<(), MeshEditError> {
    selection.validate_for_triangle_count(triangle_count)
}

/// Validate mesh edit options.
///
/// # Errors
/// Returns [`MeshEditError::InvalidOptions`] if an option is invalid.
pub fn validate_mesh_edit_options(
    options: MeshEditOptions,
) -> Result<MeshEditOptions, MeshEditError> {
    options.validate()
}

use super::{
    recompute_all_normals, validate_face_edit_buffers, validate_selection_against_triangle_count,
    FaceSelection, MeshEditBuffers, MeshEditError, MeshEditReport, MeshEditResult,
};

/// Flip the winding of every selected triangle, or every triangle when no
/// selection is provided.
///
/// The vertex payload is preserved and normals are recomputed from the flipped
/// topology so they match the new orientation.
///
/// # Errors
/// Returns typed validation errors for unsupported point clouds, malformed
/// triangles, or invalid selection masks.
pub fn invert_orientation(
    mesh: &MeshEditBuffers,
    selection: Option<&FaceSelection>,
) -> Result<MeshEditResult, MeshEditError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    if let Some(selection) = selection {
        validate_selection_against_triangle_count(mesh.triangle_count(), selection)?;
    }

    let input_vertices = mesh.vertices.len();
    let input_triangles = mesh.triangle_count();

    let mut vertices = mesh.vertices.clone();
    let mut indices = mesh.indices.clone();
    for (triangle_index, triangle) in indices.chunks_exact_mut(3).enumerate() {
        let should_flip = selection.is_none_or(|selection| selection.as_slice()[triangle_index]);
        if should_flip {
            triangle.swap(1, 2);
        }
    }

    recompute_all_normals(&mut vertices, &indices)?;

    let report = MeshEditReport {
        input_vertices,
        input_triangles,
        output_vertices: vertices.len(),
        output_triangles: indices.len() / 3,
        removed_triangles: 0,
        filled_holes: 0,
        moved_vertices: 0,
        skipped_border_rims: 0,
        skipped_oversize_rims: 0,
        skipped_damaged_rims: 0,
        healed_rims: 0,
        warnings: Vec::new(),
    };

    Ok(MeshEditResult {
        mesh: MeshEditBuffers {
            vertices,
            indices,
            topology: mesh.topology,
        },
        report,
    })
}

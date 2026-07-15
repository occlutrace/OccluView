use super::adjacency::vertex_index;
use super::{
    copy_surviving_vertices, recompute_all_normals, remap_triangle_indices,
    validate_face_edit_buffers, validate_mesh_edit_options,
    validate_selection_against_triangle_count, FaceSelection, MeshEditBuffers, MeshEditError,
    MeshEditOptions, MeshEditReport, MeshEditResult,
};

/// Delete all selected faces from a triangle mesh.
///
/// Selected triangles are removed. When `options.compact_vertices` is true, the
/// output drops unreferenced vertices and remaps indices without welding
/// same-position vertices.
///
/// # Errors
/// Returns typed validation errors for unsupported point clouds, malformed
/// triangle data, selection length mismatches, or invalid options.
pub fn delete_selected_faces(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<MeshEditResult, MeshEditError> {
    edit_faces(mesh, selection, options, false)
}

/// Keep only the selected faces from a triangle mesh.
///
/// When `options.compact_vertices` is true, the output drops unreferenced
/// vertices and remaps indices without welding same-position vertices.
///
/// # Errors
/// Returns typed validation errors for unsupported point clouds, malformed
/// triangle data, selection length mismatches, or invalid options.
pub fn crop_to_selected_faces(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<MeshEditResult, MeshEditError> {
    edit_faces(mesh, selection, options, true)
}

fn edit_faces(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
    options: MeshEditOptions,
    keep_selected: bool,
) -> Result<MeshEditResult, MeshEditError> {
    let options = validate_mesh_edit_options(options)?;
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    validate_selection_against_triangle_count(mesh.triangle_count(), selection)?;

    let input_vertices = mesh.vertices.len();
    let input_triangles = mesh.triangle_count();

    let mut kept_triangle_indices = Vec::new();
    let mut surviving_vertex_indices = Vec::new();
    let mut vertex_seen = vec![false; mesh.vertices.len()];

    for (triangle_index, triangle) in mesh.indices.chunks_exact(3).enumerate() {
        let selected = selection.as_slice()[triangle_index];
        let keep_triangle = if keep_selected { selected } else { !selected };

        if !keep_triangle {
            continue;
        }

        kept_triangle_indices.extend_from_slice(triangle);

        if options.compact_vertices {
            for &raw_index in triangle {
                let old_index = vertex_index(raw_index, triangle_index)?;
                if !vertex_seen[old_index] {
                    vertex_seen[old_index] = true;
                    surviving_vertex_indices.push(old_index);
                }
            }
        }
    }

    let (mut vertices, indices) = if options.compact_vertices {
        let (copied_vertices, remap) =
            copy_surviving_vertices(&mesh.vertices, &surviving_vertex_indices)?;
        let remapped_indices = remap_triangle_indices(&kept_triangle_indices, &remap)?;
        (copied_vertices, remapped_indices)
    } else {
        (mesh.vertices.clone(), kept_triangle_indices)
    };

    if matches!(mesh.topology, super::MeshTopology::TriangleMesh) {
        recompute_all_normals(&mut vertices, &indices)?;
    }

    let output_triangles = indices.len() / 3;
    let report = MeshEditReport {
        input_vertices,
        input_triangles,
        output_vertices: vertices.len(),
        output_triangles,
        removed_triangles: input_triangles.saturating_sub(output_triangles),
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

use super::{Mesh, MeshKind, MeshTexture, Vertex};
use crate::error::CoreError;
use occlu_mesh_edit::{
    component_at_triangle, crop_to_selected_faces, delete_selected_faces, fill_holes,
    fill_selected_holes, invert_orientation, repair_mesh, selected_connected_components,
    smooth_selected_faces, EditVertex, FaceSelection, MeshEditBuffers, MeshEditError,
    MeshEditOptions, MeshEditReport, MeshEditResult as RawMeshEditResult, MeshTopology,
    RepairOptions, RepairReport,
};

/// Result of applying a mesh edit to a core [`Mesh`].
#[derive(Clone, Debug)]
pub struct CoreMeshEditResult {
    /// Edited mesh rebuilt with the source mesh metadata policy.
    pub mesh: Mesh,
    /// Product-neutral operation report.
    pub report: MeshEditReport,
}

/// Result of running the one-click repair pipeline on a core [`Mesh`].
#[derive(Clone, Debug)]
pub struct CoreMeshRepairResult {
    /// Repaired mesh rebuilt with the source mesh metadata policy.
    pub mesh: Mesh,
    /// Per-pass repair report.
    pub report: RepairReport,
}

fn edit_vertex_from_vertex(vertex: &Vertex) -> EditVertex {
    EditVertex {
        position: vertex.position,
        normal: vertex.normal,
        color: vertex.color,
        uv: vertex.uv,
    }
}

fn vertex_from_edit_vertex(vertex: &EditVertex) -> Vertex {
    Vertex {
        position: vertex.position,
        normal: vertex.normal,
        color: vertex.color,
        uv: vertex.uv,
    }
}

fn mesh_has_uvs(buffers: &MeshEditBuffers) -> bool {
    buffers
        .vertices
        .iter()
        .any(|vertex| vertex.uv != [0.0, 0.0])
}

fn source_texture(source: &Mesh) -> Option<MeshTexture> {
    source.texture().cloned()
}

/// Convert a core mesh into product-neutral editable buffers.
#[must_use]
pub fn mesh_edit_buffers_from_mesh(mesh: &Mesh) -> MeshEditBuffers {
    MeshEditBuffers {
        vertices: mesh
            .vertices()
            .iter()
            .map(edit_vertex_from_vertex)
            .collect(),
        indices: mesh.indices().to_vec(),
        topology: match mesh.kind() {
            MeshKind::TriangleMesh => MeshTopology::TriangleMesh,
            MeshKind::PointCloud => MeshTopology::PointCloud,
        },
    }
}

/// Rebuild a core mesh from editable buffers, using `source` as a template for
/// preserved metadata such as the name and decoded texture.
///
/// # Errors
/// Returns [`CoreError`] if triangle topology buffers contain invalid indices
/// or an invalid index count.
pub fn mesh_from_edit_buffers_like(
    source: &Mesh,
    buffers: MeshEditBuffers,
) -> Result<Mesh, CoreError> {
    mesh_from_edit_buffers_named_like(source, buffers, source.name().map(str::to_owned))
}

pub(super) fn mesh_from_edit_buffers_named_like(
    source: &Mesh,
    buffers: MeshEditBuffers,
    name: Option<String>,
) -> Result<Mesh, CoreError> {
    rebuild_mesh_from_edit_buffers(source, buffers, name, false)
}

pub(super) fn mesh_from_edit_buffers_named_preserving_texture(
    source: &Mesh,
    buffers: MeshEditBuffers,
    name: Option<String>,
) -> Result<Mesh, CoreError> {
    rebuild_mesh_from_edit_buffers(source, buffers, name, true)
}

fn rebuild_mesh_from_edit_buffers(
    source: &Mesh,
    buffers: MeshEditBuffers,
    name: Option<String>,
    preserve_source_texture: bool,
) -> Result<Mesh, CoreError> {
    let texture = source_texture(source);
    let has_uvs = mesh_has_uvs(&buffers);

    let mut mesh = match buffers.topology {
        MeshTopology::PointCloud => {
            if !buffers.indices.is_empty() {
                return Err(CoreError::Geometry(
                    "point cloud edit buffers must not carry triangle indices".to_string(),
                ));
            }
            let vertices = buffers
                .vertices
                .iter()
                .map(vertex_from_edit_vertex)
                .collect();
            Mesh::point_cloud(name, vertices)
        }
        MeshTopology::TriangleMesh => {
            let vertices = buffers
                .vertices
                .iter()
                .map(vertex_from_edit_vertex)
                .collect();
            Mesh::new(name, vertices, buffers.indices)?
        }
    };

    if let Some(texture) = texture.filter(|_| preserve_source_texture || has_uvs) {
        mesh.set_texture(texture);
    }

    Ok(mesh)
}

/// Delete selected faces from a core mesh.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, invalid selection, malformed
/// edit buffers, or invalid rebuilt mesh data.
pub fn delete_selected_faces_in_mesh(
    source: &Mesh,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| {
        delete_selected_faces(buffers, selection, options)
    })
}

/// Keep only selected faces from a core mesh.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, invalid selection, malformed
/// edit buffers, or invalid rebuilt mesh data.
pub fn crop_mesh_to_selected_faces(
    source: &Mesh,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| {
        crop_to_selected_faces(buffers, selection, options)
    })
}

/// Fill conservative boundary holes in a core mesh.
///
/// Passing `None` intentionally selects the whole mesh. Interactive callers
/// that require an explicit operator selection must use
/// [`fill_selected_holes_in_mesh`] instead.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, malformed edit buffers,
/// invalid selection, or invalid rebuilt mesh data.
pub fn fill_holes_in_mesh(
    source: &Mesh,
    selection: Option<&FaceSelection>,
    options: MeshEditOptions,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| fill_holes(buffers, selection, options))
}

/// Close only holes covered by an explicit face selection on a core mesh.
/// Unlike [`fill_holes_in_mesh`], this API has no whole-mesh fallback and is
/// the adapter used by the interactive Mesh Editor.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, malformed edit buffers,
/// invalid selections, or invalid rebuilt mesh data.
pub fn fill_selected_holes_in_mesh(
    source: &Mesh,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| {
        fill_selected_holes(buffers, selection, options)
    })
}

/// One-click Smooth (issue #11) over an explicit face selection on a core
/// mesh: volume-preserving Taubin relaxation blended into the surrounding
/// untouched surface, the adapter used by the interactive Mesh Editor's
/// Smooth button.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, malformed edit buffers, or
/// invalid selections.
pub fn smooth_selected_faces_in_mesh(
    source: &Mesh,
    selection: &FaceSelection,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| smooth_selected_faces(buffers, selection))
}

/// Flip selected triangle winding, or the whole mesh when no selection is provided.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, malformed edit buffers,
/// invalid selections, or invalid rebuilt mesh data.
pub fn invert_mesh_orientation(
    source: &Mesh,
    selection: Option<&FaceSelection>,
) -> Result<CoreMeshEditResult, CoreError> {
    run_mesh_edit(source, |buffers| invert_orientation(buffers, selection))
}

/// Split a selected face mask into connected components on a core mesh.
///
/// Each component is a list of its member triangle indices (ascending), not a
/// full-length mask, so a fragmented selection stays O(selected) in memory
/// rather than O(components × `triangle_count`). The caller materializes one
/// mask at a time.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, invalid selection, or
/// malformed mesh data.
pub fn selected_connected_components_in_mesh(
    source: &Mesh,
    selection: &FaceSelection,
) -> Result<Vec<Vec<usize>>, CoreError> {
    let buffers = mesh_edit_buffers_from_mesh(source);
    selected_connected_components(&buffers, selection).map_err(mesh_edit_error_to_core)
}

/// Triangle indices of the connected component (one object) that owns
/// `triangle_index` on a core mesh, in ascending order.
///
/// A soup STL that fuses several objects is welded to its true topology first,
/// so the result spans the whole clicked object, not a single facet. Returns
/// `Ok(None)` when there is nothing to pick (out-of-range triangle or a faceless
/// mesh). Used by the mesh editor's Object selection mode.
///
/// # Errors
/// Returns [`CoreError`] for point-cloud topology or malformed triangle data.
pub fn component_at_triangle_in_mesh(
    source: &Mesh,
    triangle_index: usize,
) -> Result<Option<Vec<usize>>, CoreError> {
    let buffers = mesh_edit_buffers_from_mesh(source);
    component_at_triangle(&buffers, triangle_index).map_err(mesh_edit_error_to_core)
}

/// Run the full mesh-repair pipeline (weld / degenerate / duplicate /
/// non-manifold / orientation / debris / pinhole passes) on a core mesh.
///
/// # Errors
/// Returns [`CoreError`] for unsupported topology, malformed edit buffers,
/// invalid options, or invalid rebuilt mesh data.
pub fn repair_mesh_in_mesh(
    source: &Mesh,
    options: RepairOptions,
) -> Result<CoreMeshRepairResult, CoreError> {
    let buffers = mesh_edit_buffers_from_mesh(source);
    let result = repair_mesh(&buffers, options).map_err(mesh_edit_error_to_core)?;
    let mesh = mesh_from_edit_buffers_like(source, result.mesh)?;
    Ok(CoreMeshRepairResult {
        mesh,
        report: result.report,
    })
}

fn run_mesh_edit(
    source: &Mesh,
    edit: impl FnOnce(&MeshEditBuffers) -> Result<RawMeshEditResult, MeshEditError>,
) -> Result<CoreMeshEditResult, CoreError> {
    let buffers = mesh_edit_buffers_from_mesh(source);
    let result = edit(&buffers).map_err(mesh_edit_error_to_core)?;
    let mesh = mesh_from_edit_buffers_like(source, result.mesh)?;
    Ok(CoreMeshEditResult {
        mesh,
        report: result.report,
    })
}

fn mesh_edit_error_to_core(error: MeshEditError) -> CoreError {
    CoreError::Geometry(error.to_string())
}

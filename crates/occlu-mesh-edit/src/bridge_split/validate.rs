use super::BridgeSplitRequest;
use crate::topology::{canonical_topology, indexed_topology, TopologyWeldPolicy};
use crate::topology_analysis::{connected_components, edge_incidence, topology_defects};
use crate::{validate_face_edit_buffers, BridgeSplitError, MeshEditBuffers, MeshEditError};

#[derive(Copy, Clone, Debug)]
pub(crate) struct NormalizedBridgeSplitRequest {
    pub(crate) center: glam::DVec3,
    pub(crate) normal: glam::DVec3,
    pub(crate) kerf_mm: f64,
    pub(crate) disc_radius_mm: f64,
    pub(crate) max_disc_radius_mm: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedBridgeSplit {
    pub(crate) request: NormalizedBridgeSplitRequest,
    pub(crate) topology: crate::topology::CanonicalTopology,
}

/// Validate that one separator-disc request targets a single closed, oriented,
/// manifold triangle mesh. Position seams are canonicalized for analysis only;
/// the source buffers and their color/UV payloads are never rewritten.
///
/// # Errors
/// Returns a typed [`BridgeSplitError`] for unsupported, malformed, open,
/// non-manifold, inconsistently oriented, or disconnected input.
pub fn validate_bridge_split(
    mesh: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<(), BridgeSplitError> {
    prepare_bridge_split(mesh, request).map(|_| ())
}

/// Validate separator geometry independently from any source mesh.
///
/// Backend adapters must call this before selecting a clipping or CSG path so
/// every implementation enforces the same finite-disc safety ceiling.
///
/// # Errors
/// Returns [`BridgeSplitError::InvalidRequest`] for non-finite, zero, or
/// out-of-range separator geometry.
pub fn validate_bridge_split_request(request: BridgeSplitRequest) -> Result<(), BridgeSplitError> {
    normalize_request(request).map(|_| ())
}

/// Validate one logical Bridge Split output part.
///
/// Unlike [`validate_bridge_split`], this accepts more than one disconnected
/// physical component. Every component must still be closed, consistently
/// oriented, and manifold; no invalid shell is hidden by the logical grouping.
/// Already-valid indexed topology keeps separate touching shells distinct;
/// exact-position recovery is used only for triangle soup and payload seams.
/// The returned count is the number of physical components in the part.
///
/// # Errors
/// Returns a typed [`BridgeSplitError`] for malformed, empty, open,
/// non-manifold, or inconsistently oriented geometry.
pub fn validate_bridge_split_part(mesh: &MeshEditBuffers) -> Result<usize, BridgeSplitError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    if mesh.indices.is_empty() {
        return Err(BridgeSplitError::EmptyInput);
    }
    validate_vertex_payloads(mesh)?;
    let (_, components) = validate_closed_topology(mesh)?;
    Ok(components)
}

pub(crate) fn prepare_bridge_split(
    mesh: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<PreparedBridgeSplit, BridgeSplitError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    if mesh.indices.is_empty() {
        return Err(BridgeSplitError::EmptyInput);
    }
    validate_vertex_payloads(mesh)?;
    let request = normalize_request(request)?;
    let (topology, components) = validate_closed_topology(mesh)?;

    if components != 1 {
        return Err(BridgeSplitError::DisconnectedInput { components });
    }

    Ok(PreparedBridgeSplit { request, topology })
}

fn validate_closed_topology(
    mesh: &MeshEditBuffers,
) -> Result<(crate::topology::CanonicalTopology, usize), BridgeSplitError> {
    let position_topology = canonical_topology(mesh, TopologyWeldPolicy::PositionOnly)?;
    let collapsed_faces = degenerate_face_count(&position_topology);
    if collapsed_faces > 0 {
        return Err(BridgeSplitError::DegenerateInput {
            faces: collapsed_faces,
        });
    }

    let indexed = indexed_topology(mesh);
    if let Ok(components) = validate_topology(&indexed) {
        return Ok((indexed, components));
    }

    let components = validate_topology(&position_topology)?;
    Ok((position_topology, components))
}

fn validate_topology(
    topology: &crate::topology::CanonicalTopology,
) -> Result<usize, BridgeSplitError> {
    let incidence = edge_incidence(topology.indices());

    let degenerate_faces = degenerate_face_count(topology);
    if degenerate_faces > 0 {
        return Err(BridgeSplitError::DegenerateInput {
            faces: degenerate_faces,
        });
    }

    let defects = topology_defects(topology.indices(), &incidence);
    let boundary_edges = defects.boundary_edges;
    let non_manifold_edges = defects.non_manifold_edges;
    let inconsistent_winding_edges = defects.inconsistent_winding_edges;
    let non_manifold_vertices = defects.non_manifold_vertices;
    if boundary_edges > 0
        || non_manifold_edges > 0
        || inconsistent_winding_edges > 0
        || non_manifold_vertices > 0
    {
        return Err(BridgeSplitError::OpenOrNonManifold {
            boundary_edges,
            non_manifold_edges,
            inconsistent_winding_edges,
            non_manifold_vertices,
        });
    }

    let components = connected_components(topology.indices(), &incidence)
        .members
        .len();
    Ok(components)
}

fn degenerate_face_count(topology: &crate::topology::CanonicalTopology) -> usize {
    topology
        .indices()
        .chunks_exact(3)
        .filter(|face| face[0] == face[1] || face[1] == face[2] || face[2] == face[0])
        .count()
}

fn validate_vertex_payloads(mesh: &MeshEditBuffers) -> Result<(), BridgeSplitError> {
    for (vertex_index, vertex) in mesh.vertices.iter().enumerate() {
        if !vertex.position.into_iter().all(f32::is_finite) {
            return Err(MeshEditError::MalformedMesh {
                reason: format!("vertex {vertex_index} has a non-finite position"),
            }
            .into());
        }
        if !vertex.normal.into_iter().all(f32::is_finite) {
            return Err(MeshEditError::MalformedMesh {
                reason: format!("vertex {vertex_index} has a non-finite normal"),
            }
            .into());
        }
        if !vertex.uv.into_iter().all(f32::is_finite) {
            return Err(MeshEditError::MalformedMesh {
                reason: format!("vertex {vertex_index} has a non-finite UV"),
            }
            .into());
        }
    }
    Ok(())
}

fn normalize_request(
    request: BridgeSplitRequest,
) -> Result<NormalizedBridgeSplitRequest, BridgeSplitError> {
    if !request.center.is_finite() {
        return Err(invalid_request("disc center must be finite"));
    }
    let normal = request.normal.as_dvec3();
    if !request.normal.is_finite()
        || !normal.length_squared().is_finite()
        || normal.length_squared() <= f64::EPSILON
    {
        return Err(invalid_request("disc normal must be finite and non-zero"));
    }
    if !request.kerf_mm.is_finite() || request.kerf_mm <= 0.0 {
        return Err(invalid_request(
            "kerf_mm must be finite and greater than zero",
        ));
    }
    if !request.disc_radius_mm.is_finite() || request.disc_radius_mm <= 0.0 {
        return Err(invalid_request(
            "disc_radius_mm must be finite and greater than zero",
        ));
    }
    if !request.max_disc_radius_mm.is_finite() || request.max_disc_radius_mm <= 0.0 {
        return Err(invalid_request(
            "max_disc_radius_mm must be finite and greater than zero",
        ));
    }
    if request.disc_radius_mm > request.max_disc_radius_mm {
        return Err(invalid_request(
            "disc_radius_mm must not exceed max_disc_radius_mm",
        ));
    }

    Ok(NormalizedBridgeSplitRequest {
        center: request.center.as_dvec3(),
        normal: normal.normalize(),
        kerf_mm: f64::from(request.kerf_mm),
        disc_radius_mm: f64::from(request.disc_radius_mm),
        max_disc_radius_mm: f64::from(request.max_disc_radius_mm),
    })
}

fn invalid_request(reason: &str) -> BridgeSplitError {
    BridgeSplitError::InvalidRequest {
        reason: reason.to_string(),
    }
}

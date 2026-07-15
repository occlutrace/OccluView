use glam::DVec3;

use super::cap::cap_open_part;
use super::clip::clip_bridge_open;
use super::{BridgeSplitRequest, BridgeSplitResult};
use crate::{validate_bridge_split_part, BridgeSplitError, MeshEditBuffers};

/// Split one closed bridge mesh with a finite separator disc and return two
/// independently closed, manufacturable parts. The operation is all-or-nothing:
/// the source is borrowed and no partial result escapes on any failure.
///
/// # Errors
/// Returns a typed [`BridgeSplitError`] when the source, disc placement, cut
/// loops, caps, final topology, or requested separation is invalid.
pub fn split_bridge(
    mesh: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<BridgeSplitResult, BridgeSplitError> {
    let open = clip_bridge_open(mesh, request)?;
    let normal = request.normal.as_dvec3().normalize();
    let (positive_mesh, positive_loop_count) =
        cap_open_part(open.part_a, &open.part_a_cut_edges, -normal)?;
    let (negative_mesh, negative_loop_count) =
        cap_open_part(open.part_b, &open.part_b_cut_edges, normal)?;

    validate_output("Part A", &positive_mesh)?;
    validate_output("Part B", &negative_mesh)?;
    validate_separation(&positive_mesh, &negative_mesh, request)?;

    let mut report = open.report;
    report.part_a_triangles = positive_mesh.triangle_count();
    report.part_b_triangles = negative_mesh.triangle_count();
    report.part_a_cut_loops = positive_loop_count;
    report.part_b_cut_loops = negative_loop_count;
    Ok(BridgeSplitResult {
        part_a: positive_mesh,
        part_b: negative_mesh,
        report,
    })
}

fn validate_output(side: &'static str, mesh: &MeshEditBuffers) -> Result<(), BridgeSplitError> {
    validate_bridge_split_part(mesh)
        .map(|_| ())
        .map_err(|error| BridgeSplitError::InvalidOutput {
            side,
            reason: error.to_string(),
        })
}

fn validate_separation(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<(), BridgeSplitError> {
    let center = request.center.as_dvec3();
    let normal = request.normal.as_dvec3().normalize();
    let positive_min = projected_min(part_a, center, normal);
    let negative_max = projected_max(part_b, center, normal);
    let observed = positive_min - negative_max;
    let tolerance = local_resolution_tolerance(part_a, part_b);
    if observed + tolerance < f64::from(request.kerf_mm) {
        return Err(BridgeSplitError::SeparationViolation {
            observed_mm: finite_f64_to_f32(observed.max(0.0)),
            requested_mm: request.kerf_mm,
        });
    }
    Ok(())
}

fn local_resolution_tolerance(part_a: &MeshEditBuffers, part_b: &MeshEditBuffers) -> f64 {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for vertex in part_a.vertices.iter().chain(&part_b.vertices) {
        let position = DVec3::from_array(vertex.position.map(f64::from));
        min = min.min(position);
        max = max.max(position);
    }
    let local_scale = (max - min).length().max(1.0);
    local_scale * (8.0 * f64::from(f32::EPSILON)) + 1.0e-7
}

fn projected_min(mesh: &MeshEditBuffers, center: DVec3, normal: DVec3) -> f64 {
    mesh.vertices
        .iter()
        .map(|vertex| (DVec3::from_array(vertex.position.map(f64::from)) - center).dot(normal))
        .fold(f64::INFINITY, f64::min)
}

fn projected_max(mesh: &MeshEditBuffers, center: DVec3, normal: DVec3) -> f64 {
    mesh.vertices
        .iter()
        .map(|vertex| (DVec3::from_array(vertex.position.map(f64::from)) - center).dot(normal))
        .fold(f64::NEG_INFINITY, f64::max)
}

#[allow(clippy::cast_possible_truncation)]
fn finite_f64_to_f32(value: f64) -> f32 {
    debug_assert!(value.is_finite());
    value as f32
}

use glam::DVec3;

use super::cap::{cap_open_part, cap_surface_part};
use super::clip::{clip_bridge_open, clip_bridge_surface_open};
use super::{BridgeSplitRequest, BridgeSplitResult, SurfaceSplitResult};
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
    report.parts_closed = true;
    Ok(BridgeSplitResult {
        part_a: positive_mesh,
        part_b: negative_mesh,
        report,
    })
}

/// Clip an open or multi-component dental surface with the same finite-disc
/// placement rules as [`split_bridge`]. Existing natural borders are preserved;
/// only a simple, complete cut rim is capped when the geometry provides one.
///
/// This is intentionally separate from [`split_bridge`]. It is useful for scan
/// surfaces and framework exports that are valid renderable geometry but are not
/// closed manufacturing solids. The returned report keeps `parts_closed` false.
///
/// # Errors
/// Returns a typed [`BridgeSplitError`] for malformed buffers, invalid disc
/// settings, a miss, or a cut that cannot produce two non-empty surface pieces.
pub fn split_bridge_surface(
    mesh: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<SurfaceSplitResult, BridgeSplitError> {
    let open = clip_bridge_surface_open(mesh, request)?;
    let normal = request.normal.as_dvec3().normalize();
    let (part_a, a_cut_loops) = cap_surface_cut(open.part_a, &open.part_a_cut_edges, -normal);
    let (part_b, b_cut_loops) = cap_surface_cut(open.part_b, &open.part_b_cut_edges, normal);
    validate_separation(&part_a, &part_b, request)?;

    let mut report = open.report;
    report.part_a_triangles = part_a.triangle_count();
    report.part_b_triangles = part_b.triangle_count();
    report.part_a_cut_loops = a_cut_loops;
    report.part_b_cut_loops = b_cut_loops;
    report.parts_closed = false;
    Ok(SurfaceSplitResult {
        part_a,
        part_b,
        report,
    })
}

fn cap_surface_cut(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> (MeshEditBuffers, usize) {
    match cap_surface_part(mesh.clone(), cut_edges, expected_normal) {
        Ok((capped, loops)) => (capped, loops),
        Err(_) => match cap_open_part(mesh.clone(), cut_edges, expected_normal) {
            Ok((capped, loops)) => (capped, loops),
            // An open source can make some cut paths terminate at a natural
            // border or contain a genuinely ambiguous branch. Preserve those
            // paths rather than fabricating a cap across anatomy or refusing
            // the useful split.
            Err(_) => (mesh, 0),
        },
    }
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

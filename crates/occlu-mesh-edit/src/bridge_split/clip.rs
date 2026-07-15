use std::collections::HashMap;

use glam::DVec3;

use super::attributes::{interpolate_vertex, VertexAttributeKey};
use super::validate::{prepare_bridge_split, NormalizedBridgeSplitRequest};
use crate::{
    BridgeSplitError, BridgeSplitReport, BridgeSplitRequest, EditVertex, MeshEditBuffers,
    MeshEditError, MeshTopology,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum PlaneSide {
    PartA,
    PartB,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct IntersectionKey {
    plane: PlaneSide,
    edge: (u32, u32),
    attributes: (VertexAttributeKey, VertexAttributeKey),
}

#[derive(Copy, Clone, Debug)]
enum VertexOrigin {
    Source(u32),
    Intersection(IntersectionKey),
}

#[derive(Copy, Clone, Debug)]
struct ClipVertex {
    value: EditVertex,
    position: DVec3,
    origin: VertexOrigin,
    on_plane: bool,
}

#[derive(Copy, Clone, Debug)]
struct ClipPlaneSpec {
    request: NormalizedBridgeSplitRequest,
    offset: f64,
    keep_positive: bool,
    side: PlaneSide,
    epsilon: f64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PlaneClass {
    Inside,
    OnPlane,
    Outside,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenBridgeSplit {
    pub(crate) part_a: MeshEditBuffers,
    pub(crate) part_b: MeshEditBuffers,
    pub(crate) part_a_cut_edges: Vec<[u32; 2]>,
    pub(crate) part_b_cut_edges: Vec<[u32; 2]>,
    pub(crate) report: BridgeSplitReport,
}

struct MeshBuilder {
    vertices: Vec<EditVertex>,
    indices: Vec<u32>,
    cut_edges: Vec<[u32; 2]>,
    source_vertices: Vec<Option<u32>>,
    intersections: HashMap<IntersectionKey, u32>,
    area_epsilon: f64,
}

impl MeshBuilder {
    fn new(source_vertex_count: usize, area_epsilon: f64) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            cut_edges: Vec::new(),
            source_vertices: vec![None; source_vertex_count],
            intersections: HashMap::new(),
            area_epsilon,
        }
    }

    fn emit_polygon(&mut self, polygon: &[ClipVertex]) -> Result<(), BridgeSplitError> {
        if polygon.len() < 3 {
            return Ok(());
        }
        let mut output_indices = Vec::with_capacity(polygon.len());
        for &vertex in polygon {
            output_indices.push(self.index_for(vertex)?);
        }

        for local in 0..polygon.len() {
            let next = (local + 1) % polygon.len();
            if polygon[local].on_plane
                && polygon[next].on_plane
                && output_indices[local] != output_indices[next]
            {
                self.cut_edges
                    .push([output_indices[local], output_indices[next]]);
            }
        }

        for local in 1..polygon.len() - 1 {
            let triangle = [
                output_indices[0],
                output_indices[local],
                output_indices[local + 1],
            ];
            if triangle[0] == triangle[1]
                || triangle[1] == triangle[2]
                || triangle[2] == triangle[0]
            {
                continue;
            }
            let a = polygon[0].position;
            let b = polygon[local].position;
            let c = polygon[local + 1].position;
            let max_edge_sq = (b - a)
                .length_squared()
                .max((c - b).length_squared())
                .max((a - c).length_squared());
            let cross_sq = (b - a).cross(c - a).length_squared();
            if cross_sq <= max_edge_sq * self.area_epsilon * self.area_epsilon {
                continue;
            }
            self.indices.extend(triangle);
        }
        Ok(())
    }

    fn index_for(&mut self, vertex: ClipVertex) -> Result<u32, BridgeSplitError> {
        match vertex.origin {
            VertexOrigin::Source(source) => {
                let slot = self
                    .source_vertices
                    .get_mut(source as usize)
                    .ok_or_else(|| MeshEditError::MalformedMesh {
                        reason: "bridge split source vertex is out of range".to_string(),
                    })?;
                if let Some(existing) = *slot {
                    return Ok(existing);
                }
                let index = push_vertex(&mut self.vertices, vertex.value)?;
                *slot = Some(index);
                Ok(index)
            }
            VertexOrigin::Intersection(key) => {
                if let Some(&existing) = self.intersections.get(&key) {
                    return Ok(existing);
                }
                let index = push_vertex(&mut self.vertices, vertex.value)?;
                self.intersections.insert(key, index);
                Ok(index)
            }
        }
    }

    fn finish(self) -> (MeshEditBuffers, Vec<[u32; 2]>) {
        (
            MeshEditBuffers {
                vertices: self.vertices,
                indices: self.indices,
                topology: MeshTopology::TriangleMesh,
            },
            self.cut_edges,
        )
    }
}

pub(crate) fn clip_bridge_open(
    mesh: &MeshEditBuffers,
    request: BridgeSplitRequest,
) -> Result<OpenBridgeSplit, BridgeSplitError> {
    let prepared = prepare_bridge_split(mesh, request)?;
    let normalized = prepared.request;
    let half_kerf = normalized.kerf_mm * 0.5;
    let epsilon = snap_epsilon(mesh, normalized.kerf_mm);
    let signed_distances: Vec<f64> = mesh
        .vertices
        .iter()
        .map(|vertex| signed_distance(vertex.position, normalized))
        .collect();
    let min_distance = signed_distances
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_distance = signed_distances
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if max_distance < -half_kerf - epsilon || min_distance > half_kerf + epsilon {
        return Err(BridgeSplitError::NoIntersection);
    }
    if max_distance <= half_kerf + epsilon || min_distance >= -half_kerf - epsilon {
        return Err(BridgeSplitError::TangentContact);
    }

    let required_radius = required_disc_radius(mesh, normalized, half_kerf, epsilon);
    if required_radius > normalized.max_disc_radius_mm + epsilon {
        return Err(BridgeSplitError::DiscLimitExceeded {
            required_radius_mm: finite_f64_to_f32(required_radius),
            max_radius_mm: finite_f64_to_f32(normalized.max_disc_radius_mm),
        });
    }
    if normalized.disc_radius_mm + epsilon < required_radius {
        return Err(BridgeSplitError::DiscTooSmall {
            disc_radius_mm: finite_f64_to_f32(normalized.disc_radius_mm),
            required_radius_mm: finite_f64_to_f32(required_radius),
        });
    }

    let area_epsilon = epsilon.max(f64::EPSILON * 64.0);
    let mut part_a = MeshBuilder::new(mesh.vertices.len(), area_epsilon);
    let mut part_b = MeshBuilder::new(mesh.vertices.len(), area_epsilon);
    for (triangle, source_face) in mesh.indices.chunks_exact(3).enumerate() {
        let canonical_face = &prepared.topology.indices()[triangle * 3..triangle * 3 + 3];
        let positive = clip_triangle(
            mesh,
            source_face,
            canonical_face,
            ClipPlaneSpec {
                request: normalized,
                offset: half_kerf,
                keep_positive: true,
                side: PlaneSide::PartA,
                epsilon,
            },
        )?;
        let negative = clip_triangle(
            mesh,
            source_face,
            canonical_face,
            ClipPlaneSpec {
                request: normalized,
                offset: -half_kerf,
                keep_positive: false,
                side: PlaneSide::PartB,
                epsilon,
            },
        )?;
        part_a.emit_polygon(&positive)?;
        part_b.emit_polygon(&negative)?;
    }

    let (positive_mesh, positive_cut_edges) = part_a.finish();
    let (negative_mesh, negative_cut_edges) = part_b.finish();
    if positive_mesh.indices.is_empty()
        || negative_mesh.indices.is_empty()
        || positive_cut_edges.is_empty()
        || negative_cut_edges.is_empty()
    {
        return Err(BridgeSplitError::TangentContact);
    }
    let report = BridgeSplitReport {
        input_triangles: mesh.triangle_count(),
        part_a_triangles: positive_mesh.triangle_count(),
        part_b_triangles: negative_mesh.triangle_count(),
        kerf_mm: finite_f64_to_f32(normalized.kerf_mm),
        disc_radius_mm: finite_f64_to_f32(normalized.disc_radius_mm),
        required_disc_radius_mm: finite_f64_to_f32(required_radius),
        part_a_cut_loops: 0,
        part_b_cut_loops: 0,
    };
    Ok(OpenBridgeSplit {
        part_a: positive_mesh,
        part_b: negative_mesh,
        part_a_cut_edges: positive_cut_edges,
        part_b_cut_edges: negative_cut_edges,
        report,
    })
}

fn clip_triangle(
    mesh: &MeshEditBuffers,
    source_face: &[u32],
    canonical_face: &[u32],
    plane: ClipPlaneSpec,
) -> Result<Vec<ClipVertex>, BridgeSplitError> {
    let mut input = Vec::with_capacity(3);
    for corner in 0..3 {
        let source = source_face[corner];
        let value =
            *mesh
                .vertices
                .get(source as usize)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "bridge split triangle references a missing vertex".to_string(),
                })?;
        input.push((
            ClipVertex {
                value,
                position: DVec3::from_array(value.position.map(f64::from)),
                origin: VertexOrigin::Source(source),
                on_plane: false,
            },
            canonical_face[corner],
        ));
    }

    let mut output = Vec::with_capacity(4);
    for edge in 0..3 {
        let (first, first_canonical) = input[edge];
        let (second, second_canonical) = input[(edge + 1) % 3];
        let first_distance = plane_distance(first.position, plane.request, plane.offset);
        let second_distance = plane_distance(second.position, plane.request, plane.offset);
        let first_class = classify(first_distance, plane.keep_positive, plane.epsilon);
        let second_class = classify(second_distance, plane.keep_positive, plane.epsilon);

        if first_class != PlaneClass::Outside {
            let mut kept = first;
            if first_class == PlaneClass::OnPlane {
                kept.position -= plane.request.normal * first_distance;
                kept.value.position = kept.position.as_vec3().to_array();
                kept.on_plane = true;
            }
            push_distinct(&mut output, kept, plane.epsilon);
        }
        if strict_crossing(first_class, second_class) {
            let denominator = first_distance - second_distance;
            if denominator.abs() <= f64::EPSILON {
                continue;
            }
            let t = (first_distance / denominator).clamp(0.0, 1.0);
            let mut position = first.position.lerp(second.position, t);
            let residual = plane_distance(position, plane.request, plane.offset);
            position -= plane.request.normal * residual;
            let key = intersection_key(
                plane.side,
                first_canonical,
                second_canonical,
                first.value,
                second.value,
            );
            push_distinct(
                &mut output,
                ClipVertex {
                    value: interpolate_vertex(first.value, second.value, t, position),
                    position,
                    origin: VertexOrigin::Intersection(key),
                    on_plane: true,
                },
                plane.epsilon,
            );
        }
    }
    remove_closing_duplicate(&mut output, plane.epsilon);
    Ok(output)
}

fn intersection_key(
    plane: PlaneSide,
    first_canonical: u32,
    second_canonical: u32,
    first: EditVertex,
    second: EditVertex,
) -> IntersectionKey {
    if first_canonical < second_canonical {
        IntersectionKey {
            plane,
            edge: (first_canonical, second_canonical),
            attributes: (first.into(), second.into()),
        }
    } else {
        IntersectionKey {
            plane,
            edge: (second_canonical, first_canonical),
            attributes: (second.into(), first.into()),
        }
    }
}

fn required_disc_radius(
    mesh: &MeshEditBuffers,
    request: NormalizedBridgeSplitRequest,
    half_kerf: f64,
    epsilon: f64,
) -> f64 {
    let mut radial_extent = 0.0_f64;
    for face in mesh.indices.chunks_exact(3) {
        let mut polygon: Vec<DVec3> = face
            .iter()
            .map(|&index| DVec3::from_array(mesh.vertices[index as usize].position.map(f64::from)))
            .collect();
        polygon = clip_position_polygon(&polygon, request, -half_kerf, true, epsilon);
        polygon = clip_position_polygon(&polygon, request, half_kerf, false, epsilon);
        for position in polygon {
            let relative = position - request.center;
            let radial = relative - request.normal * relative.dot(request.normal);
            radial_extent = radial_extent.max(radial.length());
        }
    }
    let margin = (request.kerf_mm * 0.5).clamp(0.01, 0.25);
    radial_extent + margin
}

fn clip_position_polygon(
    input: &[DVec3],
    request: NormalizedBridgeSplitRequest,
    plane_offset: f64,
    keep_positive: bool,
    epsilon: f64,
) -> Vec<DVec3> {
    if input.is_empty() {
        return Vec::new();
    }
    let mut output = Vec::with_capacity(input.len() + 1);
    for edge in 0..input.len() {
        let first = input[edge];
        let second = input[(edge + 1) % input.len()];
        let first_distance = plane_distance(first, request, plane_offset);
        let second_distance = plane_distance(second, request, plane_offset);
        let first_class = classify(first_distance, keep_positive, epsilon);
        let second_class = classify(second_distance, keep_positive, epsilon);
        if first_class != PlaneClass::Outside {
            let kept = if first_class == PlaneClass::OnPlane {
                first - request.normal * first_distance
            } else {
                first
            };
            push_position_distinct(&mut output, kept, epsilon);
        }
        if strict_crossing(first_class, second_class) {
            let denominator = first_distance - second_distance;
            if denominator.abs() > f64::EPSILON {
                let t = (first_distance / denominator).clamp(0.0, 1.0);
                let mut position = first.lerp(second, t);
                position -= request.normal * plane_distance(position, request, plane_offset);
                push_position_distinct(&mut output, position, epsilon);
            }
        }
    }
    if output.len() > 1
        && output
            .first()
            .zip(output.last())
            .is_some_and(|(first, last)| first.distance_squared(*last) <= epsilon * epsilon)
    {
        output.pop();
    }
    output
}

fn signed_distance(position: [f32; 3], request: NormalizedBridgeSplitRequest) -> f64 {
    (DVec3::from_array(position.map(f64::from)) - request.center).dot(request.normal)
}

fn plane_distance(position: DVec3, request: NormalizedBridgeSplitRequest, offset: f64) -> f64 {
    (position - request.center).dot(request.normal) - offset
}

fn classify(distance: f64, keep_positive: bool, epsilon: f64) -> PlaneClass {
    if distance.abs() <= epsilon {
        PlaneClass::OnPlane
    } else if (keep_positive && distance > 0.0) || (!keep_positive && distance < 0.0) {
        PlaneClass::Inside
    } else {
        PlaneClass::Outside
    }
}

fn strict_crossing(first: PlaneClass, second: PlaneClass) -> bool {
    matches!(
        (first, second),
        (PlaneClass::Inside, PlaneClass::Outside) | (PlaneClass::Outside, PlaneClass::Inside)
    )
}

fn push_distinct(output: &mut Vec<ClipVertex>, vertex: ClipVertex, epsilon: f64) {
    if output
        .last()
        .is_none_or(|last| last.position.distance_squared(vertex.position) > epsilon * epsilon)
    {
        output.push(vertex);
    }
}

fn remove_closing_duplicate(output: &mut Vec<ClipVertex>, epsilon: f64) {
    if output.len() > 1
        && output
            .first()
            .zip(output.last())
            .is_some_and(|(first, last)| {
                first.position.distance_squared(last.position) <= epsilon * epsilon
            })
    {
        output.pop();
    }
}

fn push_position_distinct(output: &mut Vec<DVec3>, position: DVec3, epsilon: f64) {
    if output
        .last()
        .is_none_or(|last| last.distance_squared(position) > epsilon * epsilon)
    {
        output.push(position);
    }
}

fn push_vertex(vertices: &mut Vec<EditVertex>, vertex: EditVertex) -> Result<u32, MeshEditError> {
    let index = u32::try_from(vertices.len()).map_err(|_| MeshEditError::MalformedMesh {
        reason: "bridge split output vertex count exceeds u32::MAX".to_string(),
    })?;
    vertices.push(vertex);
    Ok(index)
}

fn snap_epsilon(mesh: &MeshEditBuffers, kerf_mm: f64) -> f64 {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for vertex in &mesh.vertices {
        let position = DVec3::from_array(vertex.position.map(f64::from));
        min = min.min(position);
        max = max.max(position);
    }
    let scale = (max - min).length().max(1.0);
    (scale * (8.0 * f64::from(f32::EPSILON)))
        .max(f64::EPSILON * 64.0)
        .min(kerf_mm * 0.05)
}

#[allow(clippy::cast_possible_truncation)]
fn finite_f64_to_f32(value: f64) -> f32 {
    debug_assert!(value.is_finite());
    value as f32
}

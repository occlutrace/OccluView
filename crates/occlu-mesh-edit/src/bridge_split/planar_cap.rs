//! Planar triangulation for separator caps, including nested cut rims.
//!
//! A hollow crown, connector, or retained inner surface produces an outer
//! cut loop and one or more inner loops on the same separator plane. Treating
//! every loop as an independent filled polygon closes the wrong material. This
//! module groups loops by containment and triangulates each filled region with
//! its direct inner rims as holes.

use earcutr::earcut;
use glam::DVec3;

use crate::{BridgeSplitError, MeshEditBuffers, MeshEditError};

type Point2 = [f64; 2];

struct ProjectedLoop {
    vertices: Vec<usize>,
    points: Vec<Point2>,
    signed_area: f64,
}

#[derive(Clone, Copy)]
struct ProjectionBasis {
    origin: DVec3,
    normal: DVec3,
    u: DVec3,
    v: DVec3,
    tolerance: f64,
}

pub(crate) fn triangulate_regions(
    mesh: &MeshEditBuffers,
    loops: &[Vec<usize>],
    expected_normal: DVec3,
) -> Result<(Vec<u32>, usize), BridgeSplitError> {
    triangulate_regions_with_policy(mesh, loops, expected_normal, false)
}

pub(crate) fn triangulate_regions_best_effort(
    mesh: &MeshEditBuffers,
    loops: &[Vec<usize>],
    expected_normal: DVec3,
) -> Result<(Vec<u32>, usize), BridgeSplitError> {
    triangulate_regions_with_policy(mesh, loops, expected_normal, true)
}

fn triangulate_regions_with_policy(
    mesh: &MeshEditBuffers,
    loops: &[Vec<usize>],
    expected_normal: DVec3,
    best_effort: bool,
) -> Result<(Vec<u32>, usize), BridgeSplitError> {
    if loops.is_empty() {
        return Err(cap_failed("no planar cut loops were provided"));
    }
    let expected_normal = expected_normal
        .try_normalize()
        .ok_or_else(|| cap_failed("separator normal is not normalizable"))?;
    let projected = project_loops(mesh, loops, expected_normal, best_effort)?;
    let parents = containment_parents(&projected)?;
    let depths = containment_depths(&parents)?;
    let mut cap_indices = Vec::new();
    let mut capped_loops = 0;

    for (outer_index, depth) in depths.iter().copied().enumerate() {
        if depth % 2 != 0 {
            continue;
        }
        let holes: Vec<usize> = parents
            .iter()
            .enumerate()
            .filter_map(|(index, parent)| (*parent == Some(outer_index)).then_some(index))
            .collect();
        match triangulate_region(mesh, &projected, outer_index, &holes, expected_normal) {
            Ok(indices) => {
                cap_indices.extend(indices);
                capped_loops += 1 + holes.len();
            }
            Err(error) if best_effort => {
                // A natural surface border or a damaged nested contour must
                // not discard an independent region. Skipping the complete
                // parent region is safer than filling an inner void blindly.
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }

    if capped_loops == 0 || (!best_effort && capped_loops != loops.len()) {
        return Err(cap_failed(
            "no complete planar cap region could be triangulated",
        ));
    }
    Ok((cap_indices, capped_loops))
}

fn project_loops(
    mesh: &MeshEditBuffers,
    loops: &[Vec<usize>],
    normal: DVec3,
    best_effort: bool,
) -> Result<Vec<ProjectedLoop>, BridgeSplitError> {
    let seed = if normal.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = (seed - normal * seed.dot(normal))
        .try_normalize()
        .ok_or_else(|| cap_failed("separator basis is not normalizable"))?;
    let v = normal.cross(u);
    let origin = loops
        .iter()
        .flat_map(|ring| ring.iter())
        .find_map(|&index| mesh.vertices.get(index))
        .map(|vertex| DVec3::from_array(vertex.position.map(f64::from)))
        .ok_or_else(|| cap_failed("planar cut loop has no vertices"))?;

    let basis = ProjectionBasis {
        origin,
        normal,
        u,
        v,
        tolerance: planar_tolerance(mesh, loops),
    };
    let mut projected = Vec::with_capacity(loops.len());
    for ring in loops {
        match project_loop(mesh, ring, basis) {
            Ok(loop_data) => projected.push(loop_data),
            Err(error) if best_effort => {
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
    if projected.is_empty() {
        return Err(cap_failed("no planar cut loop could be projected"));
    }
    Ok(projected)
}

fn project_loop(
    mesh: &MeshEditBuffers,
    ring: &[usize],
    basis: ProjectionBasis,
) -> Result<ProjectedLoop, BridgeSplitError> {
    if ring.len() < 3 {
        return Err(cap_failed("planar cut loop has fewer than three vertices"));
    }
    let mut points = Vec::with_capacity(ring.len());
    for &index in ring {
        let vertex = mesh
            .vertices
            .get(index)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: "planar cut loop vertex is out of range".to_string(),
            })?;
        let point = DVec3::from_array(vertex.position.map(f64::from));
        if (point - basis.origin).dot(basis.normal).abs() > basis.tolerance {
            return Err(cap_failed("cut loops are not coplanar"));
        }
        let relative = point - basis.origin;
        points.push([relative.dot(basis.u), relative.dot(basis.v)]);
    }
    let signed_area = polygon_area(&points);
    if !signed_area.is_finite() || signed_area.abs() <= f64::EPSILON {
        return Err(cap_failed("planar cut loop has zero area"));
    }
    if loop_self_intersects(&points) {
        return Err(cap_failed("planar cut loop is self-crossing"));
    }
    Ok(ProjectedLoop {
        vertices: ring.to_vec(),
        points,
        signed_area,
    })
}

fn containment_parents(loops: &[ProjectedLoop]) -> Result<Vec<Option<usize>>, BridgeSplitError> {
    let mut parents = vec![None; loops.len()];
    for (child_index, child) in loops.iter().enumerate() {
        let sample = child
            .points
            .first()
            .copied()
            .ok_or_else(|| cap_failed("planar cut loop has no containment sample"))?;
        parents[child_index] = loops
            .iter()
            .enumerate()
            .filter(|(parent_index, parent)| {
                *parent_index != child_index
                    && parent.signed_area.abs() > child.signed_area.abs()
                    && point_in_or_on_polygon(sample, &parent.points)
            })
            .min_by(|(_, left), (_, right)| {
                left.signed_area.abs().total_cmp(&right.signed_area.abs())
            })
            .map(|(index, _)| index);
    }
    for (child, parent) in parents.iter().enumerate() {
        if parent
            .is_some_and(|parent| boundaries_intersect(&loops[child].points, &loops[parent].points))
        {
            return Err(cap_failed("nested cut loops intersect or touch"));
        }
    }
    Ok(parents)
}

fn containment_depths(parents: &[Option<usize>]) -> Result<Vec<usize>, BridgeSplitError> {
    let mut depths = vec![0; parents.len()];
    for (start, depth_slot) in depths.iter_mut().enumerate() {
        let mut current = start;
        let mut seen = Vec::new();
        let mut depth = 0;
        while let Some(parent) = parents[current] {
            if seen.contains(&parent) {
                return Err(cap_failed("planar cut loop containment is cyclic"));
            }
            seen.push(parent);
            depth += 1;
            current = parent;
        }
        *depth_slot = depth;
    }
    Ok(depths)
}

fn triangulate_region(
    mesh: &MeshEditBuffers,
    loops: &[ProjectedLoop],
    outer_index: usize,
    holes: &[usize],
    expected_normal: DVec3,
) -> Result<Vec<u32>, BridgeSplitError> {
    let mut coordinates = Vec::new();
    let mut global_vertices = Vec::new();
    let mut hole_indices = Vec::with_capacity(holes.len());
    append_contour(&loops[outer_index], &mut coordinates, &mut global_vertices);
    for &hole_index in holes {
        hole_indices.push(global_vertices.len());
        append_contour(&loops[hole_index], &mut coordinates, &mut global_vertices);
    }

    let local_indices = earcut::<f64>(&coordinates, &hole_indices, 2)
        .map_err(|_| cap_failed("planar cap triangulation failed"))?;
    let expected_triangles = global_vertices
        .len()
        .checked_add(holes.len().saturating_mul(2))
        .and_then(|count| count.checked_sub(2))
        .ok_or_else(|| cap_failed("planar cap triangle count overflow"))?;
    if local_indices.len() != expected_triangles.saturating_mul(3) {
        return Err(cap_failed("planar cap triangulation was incomplete"));
    }

    let mut indices = Vec::with_capacity(local_indices.len());
    for triangle in local_indices.chunks_exact(3) {
        let mut triangle = [
            *global_vertices
                .get(triangle[0])
                .ok_or_else(|| cap_failed("planar cap referenced a missing rim vertex"))?,
            *global_vertices
                .get(triangle[1])
                .ok_or_else(|| cap_failed("planar cap referenced a missing rim vertex"))?,
            *global_vertices
                .get(triangle[2])
                .ok_or_else(|| cap_failed("planar cap referenced a missing rim vertex"))?,
        ];
        let points = triangle.map(|index| {
            mesh.vertices
                .get(index)
                .map(|vertex| DVec3::from_array(vertex.position.map(f64::from)))
        });
        let [Some(a), Some(b), Some(c)] = points else {
            return Err(cap_failed("planar cap referenced an invalid vertex"));
        };
        let normal = (b - a).cross(c - a);
        if !normal.is_finite() || normal.length_squared() <= 0.0 {
            return Err(cap_failed("planar cap contains a degenerate triangle"));
        }
        if normal.dot(expected_normal) < 0.0 {
            triangle.swap(1, 2);
        }
        indices.extend(
            triangle
                .into_iter()
                .map(|index| {
                    u32::try_from(index).map_err(|_| MeshEditError::MalformedMesh {
                        reason: "planar cap vertex index exceeds u32::MAX".to_string(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(indices)
}

fn append_contour(
    contour: &ProjectedLoop,
    coordinates: &mut Vec<f64>,
    global_vertices: &mut Vec<usize>,
) {
    coordinates.extend(contour.points.iter().flat_map(|[x, y]| [*x, *y]));
    global_vertices.extend(contour.vertices.iter().copied());
}

fn polygon_area(points: &[Point2]) -> f64 {
    points
        .iter()
        .enumerate()
        .map(|(index, &point)| {
            let next = points[(index + 1) % points.len()];
            point[0] * next[1] - next[0] * point[1]
        })
        .sum::<f64>()
        * 0.5
}

fn point_in_or_on_polygon(point: Point2, polygon: &[Point2]) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        if point_on_segment(point, a, b) {
            return true;
        }
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let crossing = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < crossing {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_on_segment(point: Point2, a: Point2, b: Point2) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [point[0] - a[0], point[1] - a[1]];
    let cross = ab[0] * ap[1] - ab[1] * ap[0];
    let scale = ab[0].abs().max(ab[1].abs()).max(1.0);
    cross.abs() <= 1.0e-10 * scale * scale
        && ap[0] * (point[0] - b[0]) + ap[1] * (point[1] - b[1]) <= 1.0e-10 * scale * scale
}

fn boundaries_intersect(left: &[Point2], right: &[Point2]) -> bool {
    left.iter().enumerate().any(|(left_index, &left_start)| {
        let left_end = left[(left_index + 1) % left.len()];
        right.iter().enumerate().any(|(right_index, &right_start)| {
            let right_end = right[(right_index + 1) % right.len()];
            segments_intersect(left_start, left_end, right_start, right_end)
        })
    })
}

fn loop_self_intersects(points: &[Point2]) -> bool {
    points.iter().enumerate().any(|(left_index, &left_start)| {
        let left_end = points[(left_index + 1) % points.len()];
        points
            .iter()
            .enumerate()
            .skip(left_index + 1)
            .any(|(right_index, &right_start)| {
                let right_end = points[(right_index + 1) % points.len()];
                let shares_endpoint = left_index == right_index
                    || (left_index + 1) % points.len() == right_index
                    || (right_index + 1) % points.len() == left_index;
                !shares_endpoint && segments_intersect(left_start, left_end, right_start, right_end)
            })
    })
}

fn segments_intersect(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let ab = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);
    (ab.signum() != ab_d.signum() && cd_a.signum() != cd_b.signum())
        || (ab.abs() <= 1.0e-10 && point_on_segment(c, a, b))
        || (ab_d.abs() <= 1.0e-10 && point_on_segment(d, a, b))
        || (cd_a.abs() <= 1.0e-10 && point_on_segment(a, c, d))
        || (cd_b.abs() <= 1.0e-10 && point_on_segment(b, c, d))
}

fn orientation(a: Point2, b: Point2, c: Point2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn planar_tolerance(mesh: &MeshEditBuffers, loops: &[Vec<usize>]) -> f64 {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    for &index in loops.iter().flatten() {
        if let Some(vertex) = mesh.vertices.get(index) {
            let point = DVec3::from_array(vertex.position.map(f64::from));
            min = min.min(point);
            max = max.max(point);
        }
    }
    (max - min).length().max(1.0) * 8.0 * f64::from(f32::EPSILON)
}

fn cap_failed(reason: &str) -> BridgeSplitError {
    BridgeSplitError::CapFailed {
        reason: reason.to_string(),
    }
}

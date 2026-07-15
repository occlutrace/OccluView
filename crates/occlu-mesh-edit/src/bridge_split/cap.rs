use glam::{DVec3, Vec3};

use super::rims::build_cut_loops;
use crate::cap_minweight::rim_is_simple_3d;
use crate::holes_walk::ear_clip_cap;
use crate::{
    copy_surviving_vertices, recompute_all_normals, remap_triangle_indices, BridgeSplitError,
    MeshEditBuffers, MeshEditError,
};

pub(crate) fn cap_open_part(
    mut mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_cut_loops(&mesh, cut_edges)?;
    let mut cap_indices = Vec::new();
    for ring in &loops {
        let points = ring_points(&mesh, ring)?;
        validate_planar_simple_loop(&points, expected_normal)?;
        let local_triangles = triangulate_loop(&mesh, ring, &points, expected_normal)?;
        if local_triangles.len() != ring.len() - 2 {
            return Err(cap_failed("a cut loop did not produce a complete cap"));
        }
        validate_cap_winding(&mesh, ring, &local_triangles, expected_normal)?;
        for local in local_triangles {
            for corner in local {
                let global = *ring.get(corner).ok_or_else(|| {
                    cap_failed("cap triangulation referenced a missing rim vertex")
                })?;
                cap_indices.push(u32::try_from(global).map_err(|_| {
                    MeshEditError::MalformedMesh {
                        reason: "cap vertex index exceeds u32::MAX".to_string(),
                    }
                })?);
            }
        }
    }
    mesh.indices.extend(cap_indices);
    compact_unreferenced_vertices(&mut mesh)?;
    recompute_all_normals(&mut mesh.vertices, &mesh.indices)?;
    Ok((mesh, loops.len()))
}

fn compact_unreferenced_vertices(mesh: &mut MeshEditBuffers) -> Result<(), MeshEditError> {
    let mut referenced = vec![false; mesh.vertices.len()];
    for &index in &mesh.indices {
        if let Some(slot) = referenced.get_mut(index as usize) {
            *slot = true;
        }
    }
    if referenced.iter().all(|is_referenced| *is_referenced) {
        return Ok(());
    }
    let survivors: Vec<usize> = referenced
        .iter()
        .enumerate()
        .filter_map(|(index, is_referenced)| is_referenced.then_some(index))
        .collect();
    let (vertices, remap) = copy_surviving_vertices(&mesh.vertices, &survivors)?;
    mesh.indices = remap_triangle_indices(&mesh.indices, &remap)?;
    mesh.vertices = vertices;
    Ok(())
}

fn triangulate_loop(
    mesh: &MeshEditBuffers,
    ring: &[usize],
    points: &[Vec3],
    expected_normal: DVec3,
) -> Result<Vec<[usize; 3]>, BridgeSplitError> {
    let ear = ear_clip_cap(mesh, ring)?;
    if ear.len() == ring.len() - 2 && triangles_are_nondegenerate(points, &ear) {
        return Ok(ear);
    }

    if let Some(triangles) = planar_rim_triangulation(points, expected_normal) {
        return Ok(triangles);
    }

    Err(cap_failed("cut loop triangulation stalled"))
}

const MAX_PLANAR_RIM_FALLBACK_VERTICES: usize = 1024;
const PLANAR_RIM_FALLBACK_WORK_BUDGET: u64 = 8_000_000;

/// Deterministic, boundary-aware ear clipping in the known separator plane.
///
/// Separator cuts can leave collinear vertices where the disc crosses a source
/// triangle diagonal. The generic hole capper can stall around those points.
/// This path keeps every rim vertex, never draws a diagonal through another
/// rim vertex, and only accepts ears with positive 3D area.
fn planar_rim_triangulation(points: &[Vec3], expected_normal: DVec3) -> Option<Vec<[usize; 3]>> {
    if !(3..=MAX_PLANAR_RIM_FALLBACK_VERTICES).contains(&points.len()) {
        return None;
    }
    let normal = expected_normal.normalize();
    let seed = if normal.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = (seed - normal * seed.dot(normal)).normalize();
    let v = normal.cross(u);
    let origin = points.first()?.as_dvec3();
    let projected: Vec<[f64; 2]> = points
        .iter()
        .map(|point| {
            let relative = point.as_dvec3() - origin;
            [relative.dot(u), relative.dot(v)]
        })
        .collect();
    let signed_area = (0..projected.len())
        .map(|index| {
            let current = projected[index];
            let next = projected[(index + 1) % projected.len()];
            current[0] * next[1] - current[1] * next[0]
        })
        .sum::<f64>();
    if !signed_area.is_finite() || signed_area == 0.0 {
        return None;
    }
    let orientation = signed_area.signum();
    let epsilon = planar_epsilon(&projected);
    let mut ring: Vec<usize> = (0..points.len()).collect();
    let mut triangles = Vec::with_capacity(points.len() - 2);
    let mut work_left = PLANAR_RIM_FALLBACK_WORK_BUDGET;
    while ring.len() > 3 {
        let mut clipped = false;
        for index in 0..ring.len() {
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            let current = ring[index];
            let next = ring[(index + 1) % ring.len()];
            if signed_cross(projected[previous], projected[current], projected[next]) * orientation
                <= epsilon
                || !triangle_has_area(points, [previous, current, next])
            {
                continue;
            }
            if ring.len() == 4 {
                let after_next = ring[(index + 2) % ring.len()];
                if !triangle_has_area(points, [previous, next, after_next]) {
                    continue;
                }
            }
            if ring.iter().copied().any(|candidate| {
                candidate != previous && candidate != current && candidate != next && {
                    if work_left == 0 {
                        return true;
                    }
                    work_left -= 1;
                    point_in_or_on_triangle(
                        projected[candidate],
                        [projected[previous], projected[current], projected[next]],
                        orientation,
                        epsilon,
                    )
                }
            }) {
                continue;
            }
            triangles.push(orient_triangle(
                points,
                [previous, current, next],
                expected_normal,
            ));
            ring.remove(index);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    let final_triangle = [ring[0], ring[1], ring[2]];
    if !triangle_has_area(points, final_triangle) {
        return None;
    }
    triangles.push(orient_triangle(points, final_triangle, expected_normal));
    (triangles.len() == points.len() - 2).then_some(triangles)
}

fn triangles_are_nondegenerate(points: &[Vec3], triangles: &[[usize; 3]]) -> bool {
    triangles
        .iter()
        .copied()
        .all(|triangle| triangle_has_area(points, triangle))
}

fn triangle_has_area(points: &[Vec3], triangle: [usize; 3]) -> bool {
    let [Some(a), Some(b), Some(c)] = triangle.map(|index| points.get(index).copied()) else {
        return false;
    };
    (b.as_dvec3() - a.as_dvec3())
        .cross(c.as_dvec3() - a.as_dvec3())
        .length_squared()
        > 0.0
}

fn orient_triangle(
    points: &[Vec3],
    mut triangle: [usize; 3],
    expected_normal: DVec3,
) -> [usize; 3] {
    let [a, b, c] = triangle.map(|index| points[index].as_dvec3());
    if (b - a).cross(c - a).dot(expected_normal) < 0.0 {
        triangle.swap(1, 2);
    }
    triangle
}

fn point_in_or_on_triangle(
    point: [f64; 2],
    [a, b, c]: [[f64; 2]; 3],
    orientation: f64,
    epsilon: f64,
) -> bool {
    [
        signed_cross(a, b, point),
        signed_cross(b, c, point),
        signed_cross(c, a, point),
    ]
    .into_iter()
    .all(|side| side * orientation >= -epsilon)
}

fn planar_epsilon(points: &[[f64; 2]]) -> f64 {
    let scale = points
        .iter()
        .flat_map(|point| point.iter().copied())
        .map(f64::abs)
        .fold(f64::MIN_POSITIVE, f64::max);
    256.0 * f64::EPSILON * scale * scale
}

fn signed_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn ring_points(mesh: &MeshEditBuffers, ring: &[usize]) -> Result<Vec<Vec3>, BridgeSplitError> {
    ring.iter()
        .map(|&index| {
            mesh.vertices
                .get(index)
                .map(|vertex| Vec3::from_array(vertex.position))
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cap rim vertex is out of range".to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(BridgeSplitError::from)
}

fn validate_planar_simple_loop(
    points: &[Vec3],
    expected_normal: DVec3,
) -> Result<(), BridgeSplitError> {
    if !rim_is_simple_3d(points) {
        return Err(cap_failed("cut loop is self-crossing"));
    }
    let Some(first) = points.first() else {
        return Err(cap_failed("cut loop is empty"));
    };
    let origin = first.as_dvec3();
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut max_deviation = 0.0_f64;
    for point in points {
        let point = point.as_dvec3();
        min = min.min(point);
        max = max.max(point);
        max_deviation = max_deviation.max((point - origin).dot(expected_normal).abs());
    }
    let tolerance = (max - min).length().max(1.0) * (8.0 * f64::from(f32::EPSILON));
    if max_deviation > tolerance {
        return Err(cap_failed("cut loop is not planar within mesh precision"));
    }
    Ok(())
}

fn validate_cap_winding(
    mesh: &MeshEditBuffers,
    ring: &[usize],
    triangles: &[[usize; 3]],
    expected_normal: DVec3,
) -> Result<(), BridgeSplitError> {
    let mut valid_triangles = 0_usize;
    for triangle in triangles {
        let points = triangle.map(|local| {
            ring.get(local)
                .and_then(|&global| mesh.vertices.get(global))
                .map(|vertex| DVec3::from_array(vertex.position.map(f64::from)))
        });
        let [Some(a), Some(b), Some(c)] = points else {
            return Err(cap_failed("cap winding references a missing vertex"));
        };
        let normal = (b - a).cross(c - a);
        // The clipper already excludes exactly collapsed polygons. Do not use
        // an absolute f64 epsilon here: fine dental meshes can yield a very
        // thin, yet nonzero, cap triangle after a legitimate separator cut.
        if normal.length_squared() > 0.0 {
            if normal.dot(expected_normal) <= 0.0 {
                return Err(cap_failed("cap winding does not face away from the kerf"));
            }
            valid_triangles += 1;
        }
    }
    if valid_triangles == triangles.len() {
        Ok(())
    } else {
        Err(cap_failed("cap contains a degenerate triangle"))
    }
}

fn cap_failed(reason: &str) -> BridgeSplitError {
    BridgeSplitError::CapFailed {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_fallback_never_draws_a_diagonal_through_a_rim_vertex() {
        // The first tempting ear has a diagonal from the last point to B,
        // directly through D. Clipping it would create a T-junction even
        // though every emitted triangle has nonzero area.
        let points = [
            Vec3::new(0.0, 0.0, 0.0), // A
            Vec3::new(2.0, 0.0, 0.0), // B
            Vec3::new(2.0, 1.0, 0.0), // C
            Vec3::new(1.0, 0.5, 0.0), // D, on the A/B ear diagonal
            Vec3::new(0.0, 1.0, 0.0), // E
        ];
        let triangles = planar_rim_triangulation(&points, DVec3::Z)
            .expect("a simple concave rim with a diagonal point must cap");

        for triangle in triangles {
            for [start, end] in [
                [triangle[0], triangle[1]],
                [triangle[1], triangle[2]],
                [triangle[2], triangle[0]],
            ] {
                if are_boundary_neighbors(start, end, points.len()) {
                    continue;
                }
                for (candidate, point) in points.iter().enumerate() {
                    if candidate != start && candidate != end {
                        assert!(
                            !lies_on_open_segment(*point, points[start], points[end]),
                            "cap diagonal {start}-{end} passes through rim vertex {candidate}"
                        );
                    }
                }
            }
        }
    }

    fn are_boundary_neighbors(first: usize, second: usize, len: usize) -> bool {
        (first + 1) % len == second || (second + 1) % len == first
    }

    fn lies_on_open_segment(point: Vec3, start: Vec3, end: Vec3) -> bool {
        let segment = end - start;
        let offset = point - start;
        segment.cross(offset).length_squared() <= f32::EPSILON
            && offset.dot(segment) > 0.0
            && offset.dot(segment) < segment.length_squared()
    }
}

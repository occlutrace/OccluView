use glam::Vec3;

/// Squared sine of the smallest angle a facet may have and still contribute a
/// normal. Scale-free: the test compares twice the facet's area against its own
/// longest edge squared, so it means the same thing on a 10 mm arch and on a
/// 10 um sliver.
const DEGENERATE_AREA_SIN: f32 = 1e-10;
use std::collections::HashMap;

use super::{validate_triangle_mesh_data, EditVertex, MeshEditError};

const DUPLICATE_NORMAL_DOT: f32 = 0.5;
const DUPLICATE_POSITION_EPS_MM: f32 = 0.002;

/// Recompute every vertex normal from triangle winding.
///
/// This intentionally overwrites valid-looking stale normals. Constructors in
/// downstream mesh types may only repair missing normals; edit kernels need a
/// stronger operation after topology changes.
///
/// # Errors
/// Returns [`MeshEditError::MalformedMesh`] if indices are invalid.
pub fn recompute_all_normals(
    vertices: &mut [EditVertex],
    indices: &[u32],
) -> Result<(), MeshEditError> {
    validate_triangle_mesh_data(vertices, indices)?;

    if indices.is_empty() {
        for vertex in vertices.iter_mut() {
            vertex.normal = [0.0; 3];
        }
        return Ok(());
    }

    let mut normals = vec![Vec3::ZERO; vertices.len()];
    for triangle in indices.chunks_exact(3) {
        let ia = triangle[0] as usize;
        let ib = triangle[1] as usize;
        let ic = triangle[2] as usize;

        let a = Vec3::from_array(vertices[ia].position);
        let b = Vec3::from_array(vertices[ib].position);
        let c = Vec3::from_array(vertices[ic].position);
        let face_normal = (b - a).cross(c - a);
        // Relative to the facet's own edges, not an absolute epsilon. The
        // cross product is twice an AREA — square millimetres — so comparing it
        // against a dimensionless f32::EPSILON dropped every facet with edges
        // under about 19 um. Lab scanners at 7 um point spacing produce exactly
        // those, and their vertices fell through to a hard +Z fallback: visible
        // shading speckle on the finest regions of a scan.
        let longest_edge_sq = (b - a)
            .length_squared()
            .max((c - b).length_squared())
            .max((a - c).length_squared());
        if face_normal.is_finite()
            && face_normal.length_squared()
                > longest_edge_sq * longest_edge_sq * DEGENERATE_AREA_SIN
        {
            normals[ia] += face_normal;
            normals[ib] += face_normal;
            normals[ic] += face_normal;
        }
    }

    for (vertex, normal) in vertices.iter_mut().zip(normals) {
        // The accumulated normal is a sum of face normals, so its magnitude
        // carries the same area units; a normalize only needs it to be nonzero.
        vertex.normal = if normal.length_squared() > 0.0 && normal.is_finite() {
            normal.normalize().to_array()
        } else {
            Vec3::Z.to_array()
        };
    }

    smooth_duplicate_position_normals(vertices);
    Ok(())
}

fn smooth_duplicate_position_normals(vertices: &mut [EditVertex]) {
    let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::with_capacity(vertices.len());
    for (index, vertex) in vertices.iter().enumerate() {
        groups
            .entry(position_key(vertex.position))
            .or_default()
            .push(index);
    }

    let source_normals: Vec<Vec3> = vertices
        .iter()
        .map(|vertex| {
            let normal = Vec3::from_array(vertex.normal);
            if normal.is_finite() && normal.length_squared() > f32::EPSILON {
                normal.normalize()
            } else {
                Vec3::ZERO
            }
        })
        .collect();
    let mut smoothed = source_normals.clone();

    for indices in groups.values().filter(|indices| indices.len() > 1) {
        for &index in indices {
            let current = source_normals[index];
            if current.length_squared() <= f32::EPSILON {
                continue;
            }

            let mut normal = Vec3::ZERO;
            for &neighbor in indices {
                let candidate = source_normals[neighbor];
                if candidate.length_squared() > f32::EPSILON
                    && candidate.dot(current) >= DUPLICATE_NORMAL_DOT
                {
                    normal += candidate;
                }
            }

            if normal.length_squared() > f32::EPSILON {
                smoothed[index] = normal.normalize();
            }
        }
    }

    for (vertex, normal) in vertices.iter_mut().zip(smoothed) {
        if normal.length_squared() > f32::EPSILON {
            vertex.normal = normal.to_array();
        }
    }
}

fn position_key(position: [f32; 3]) -> [i32; 3] {
    [
        position_lane_key(position[0]),
        position_lane_key(position[1]),
        position_lane_key(position[2]),
    ]
}

#[allow(clippy::cast_possible_truncation)]
fn position_lane_key(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let scaled = f64::from(value / DUPLICATE_POSITION_EPS_MM).round();
    if scaled <= f64::from(i32::MIN) {
        i32::MIN
    } else if scaled >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        scaled as i32
    }
}

use glam::Vec3;
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
        if face_normal.is_finite() && face_normal.length_squared() > f32::EPSILON {
            normals[ia] += face_normal;
            normals[ib] += face_normal;
            normals[ic] += face_normal;
        }
    }

    for (vertex, normal) in vertices.iter_mut().zip(normals) {
        vertex.normal = if normal.length_squared() > f32::EPSILON {
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

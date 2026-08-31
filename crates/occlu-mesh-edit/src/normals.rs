use glam::Vec3;
use occlu_geometry_math::{coincident_position_key, DUPLICATE_NORMAL_DOT, MAX_DUPLICATE_CLUSTERS};

/// Squared sine of the smallest angle a facet may have and still contribute a
/// normal. Scale-free: the test compares twice the facet's area against its own
/// longest edge squared, so it means the same thing on a 10 mm arch and on a
/// 10 um sliver.
///
/// Owned by `occlu-geometry-math` since 2026-08-29. Before that it was three
/// copies across this crate, `occluview-core` and `occluview-hps`, and the
/// fix of 2026-07-25 landed in one crate and reached the others four weeks
/// later -- for those four weeks every scan opened through the other paths
/// lost shading on facets under 20 um.
pub use occlu_geometry_math::DEGENERATE_AREA_SIN;
use std::collections::HashMap;

use super::{validate_triangle_mesh_data, EditVertex, MeshEditError};

/// Past this many vertices at one position, agreement is judged against the
/// group's mean normal instead of against every other member.
///
/// Owned by `occlu-geometry-math`; core and this crate share the bound.
pub use occlu_geometry_math::MAX_PAIRWISE_DUPLICATE_GROUP;

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
    for triangle in indices.as_chunks::<3>().0 {
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

/// Average a large coincident group by clustering it, not by one global mean.
///
/// A single mean is right while the group points one way and wrong the moment
/// it does not: K coincident vertices at a hard crease form two clusters ninety
/// degrees apart, the mean lands on the bisector, both clusters agree with it
/// to within sixty degrees, and the crease is welded flat. Members join the
/// first cluster they agree with instead, so a coherent group forms one cluster
/// and a crease keeps its two. Linear in the group for any bounded cluster
/// count.
fn average_by_cluster(indices: &[usize], source_normals: &[Vec3], smoothed: &mut [Vec3]) {
    let mut sums: Vec<Vec3> = Vec::new();
    let mut assigned: Vec<(usize, usize)> = Vec::with_capacity(indices.len());

    for &index in indices {
        let current = source_normals[index];
        if current.length_squared() <= f32::EPSILON {
            continue;
        }
        let existing = sums
            .iter()
            .position(|sum| sum.normalize_or_zero().dot(current) >= DUPLICATE_NORMAL_DOT);
        if let Some(cluster) = existing {
            sums[cluster] += current;
            assigned.push((index, cluster));
        } else {
            if sums.len() == MAX_DUPLICATE_CLUSTERS {
                // Too many directions to be a surface. Leave them as they
                // arrived.
                return;
            }
            sums.push(current);
            assigned.push((index, sums.len() - 1));
        }
    }

    for (index, cluster) in assigned {
        let mean = sums[cluster].normalize_or_zero();
        if mean.length_squared() > f32::EPSILON {
            smoothed[index] = mean;
        }
    }
}

/// The duplicate-averaging pass alone, for tests that need to see it without
/// the recompute above it replacing the normals first.
#[cfg(test)]
pub(crate) fn smooth_duplicate_position_normals_for_tests(vertices: &mut [EditVertex]) {
    smooth_duplicate_position_normals(vertices);
}

fn smooth_duplicate_position_normals(vertices: &mut [EditVertex]) {
    let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::with_capacity(vertices.len());
    for (index, vertex) in vertices.iter().enumerate() {
        groups
            .entry(coincident_position_key(vertex.position))
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
        // Past the threshold the group is clustered instead, in one greedy
        // pass. See `average_by_cluster` for why a single mean is wrong.
        if indices.len() > MAX_PAIRWISE_DUPLICATE_GROUP {
            average_by_cluster(indices, &source_normals, &mut smoothed);
            continue;
        }

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

#[cfg(test)]
mod shared_tolerance_tests {
    use super::coincident_position_key;
    use occlu_geometry_math::COINCIDENT_POSITION_EPS_MM;

    #[test]
    fn one_tolerance_decides_which_vertices_share_a_normal() {
        // The seam described on `COINCIDENT_POSITION_EPS_MM`: two copies of
        // the number and the same scan shades one way on open, another after
        // any edit.
        let origin = [0.0_f32, 0.0, 0.0];
        let inside = [COINCIDENT_POSITION_EPS_MM * 0.4, 0.0, 0.0];
        let outside = [COINCIDENT_POSITION_EPS_MM * 4.0, 0.0, 0.0];

        assert_eq!(
            coincident_position_key(origin),
            coincident_position_key(inside),
            "positions inside the tolerance must be one point"
        );
        assert_ne!(
            coincident_position_key(origin),
            coincident_position_key(outside),
            "positions well outside it must not be"
        );
        // Non-finite input has to answer something rather than panic: it
        // arrives from files.
        assert_eq!(
            coincident_position_key([f32::NAN, f32::INFINITY, 0.0]),
            [0, 0, 0]
        );
    }

    #[test]
    fn core_and_this_crate_import_the_shared_tolerances() {
        let core = include_str!("../../occluview-core/src/mesh/normals.rs");
        assert!(
            core.contains("use occlu_geometry_math::"),
            "core must use the shared tolerances rather than redefining them"
        );
        assert!(
            !core.contains(&format!("{}_DUPLICATE_GROUP", "const MAX_PAIRWISE")),
            "core must not keep a local copy of the coincident-group bound: \
             that is how the two shadings drifted apart"
        );
        assert!(
            !core.contains(&format!("{}_NORMAL_DOT", "const DUPLICATE"))
                && !core.contains(&format!("{}_NORMAL_DOT", "const SMOOTH_DUPLICATE")),
            "core must not keep a local copy of the normal-agreement threshold"
        );
    }
}

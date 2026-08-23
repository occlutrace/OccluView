use glam::Vec3;

/// Squared sine of the smallest angle a facet may have and still contribute a
/// normal. Scale-free: the test compares twice the facet's area against its own
/// longest edge squared, so it means the same thing on a 10 mm arch and on a
/// 10 um sliver.
/// A facet is degenerate when its area falls below this fraction of its own
/// longest edge squared.
///
/// Deliberately a third copy of the same rule. This crate is a leaf and must
/// not depend on `occluview-core`, which holds the same constant as
/// `occluview_core::DEGENERATE_AREA_SIN`, and `occluview-hps` keeps a fourth
/// for the same reason. Change one, change all three: the fix of 2026-07-25
/// landed in one crate and reached the others four weeks later, and for those
/// four weeks every scan opened through the other paths lost shading on facets
/// under 20 um.
const DEGENERATE_AREA_SIN: f32 = 1e-10;
use std::collections::HashMap;

use super::{validate_triangle_mesh_data, EditVertex, MeshEditError};

const DUPLICATE_NORMAL_DOT: f32 = 0.5;

/// Past this many vertices at one position, agreement is judged against the
/// group's mean normal instead of against every other member.
///
/// A fourth copy of a number `occluview-core` also holds, for the same reason
/// the degeneracy threshold above is duplicated: this crate is a leaf and must
/// not depend on core. Core bounded its loader path and this one was left
/// pairwise, which made the situation worse rather than better -- the file now
/// opens in milliseconds, so the pile reaches the scene, and the first Repair,
/// Close holes or Invert normals runs it on the UI thread with no repaint, no
/// progress and no cancel. Measured here in the test profile: 20000 coincident
/// vertices cost 820 ms pairwise against 16 ms bounded, and the pairwise form
/// grows as the square.
const MAX_PAIRWISE_DUPLICATE_GROUP: usize = 256;
/// Two positions within this distance are the same point for shading.
///
/// This is the number that decides which vertices share a normal, and both
/// this crate and `occluview-core` need it: core welds at load, this crate
/// welds after every brush stroke and hole fill. It used to be written twice,
/// under two names, with two byte-identical key functions maintained
/// separately -- change one and the same scan is shaded one way on open and
/// another way after any edit, a seam that appears mid-session with nothing to
/// blame.
pub const COINCIDENT_POSITION_EPS_MM: f32 = 0.002;

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

/// How many directions one coincident group may hold before it is left alone.
///
/// A fourth copy of the reasoning in `occluview-core`, for the same reason the
/// numbers beside it are copied: this crate is a leaf and cannot depend on core.
const MAX_DUPLICATE_CLUSTERS: usize = 16;

/// Average a large coincident group by clustering it, not by one global mean.
///
/// Judging every member against the mean of the whole group is right while the
/// group points one way and wrong the moment it does not: K coincident vertices
/// at a hard crease form two clusters ninety degrees apart, their mean sits on
/// the bisector, both clusters agree with it to within sixty degrees, and every
/// member is welded to the bisector -- the crease is gone. Members are assigned
/// greedily to the first cluster they agree with instead, so a coherent group
/// forms one cluster and gets what the mean form gave it, and a crease keeps
/// its two. Linear in the group for any bounded number of clusters.
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
                // Too many directions to be a surface. Leaving the normals as
                // they arrived is the honest answer.
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

/// Quantize a position onto the [`COINCIDENT_POSITION_EPS_MM`] lattice.
///
/// Equal keys mean "the same point" for normal welding. Shared with
/// `occluview-core` so both sides of an edit agree.
#[must_use]
pub fn coincident_position_key(position: [f32; 3]) -> [i32; 3] {
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

    let scaled = f64::from(value / COINCIDENT_POSITION_EPS_MM).round();
    if scaled <= f64::from(i32::MIN) {
        i32::MIN
    } else if scaled >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        scaled as i32
    }
}

#[cfg(test)]
mod shared_tolerance_tests {
    use super::{coincident_position_key, COINCIDENT_POSITION_EPS_MM};

    #[test]
    fn one_tolerance_decides_which_vertices_share_a_normal() {
        // Core welds at load, this crate welds after every brush stroke and
        // hole fill. Two copies of this number meant the same scan could shade
        // one way on open and another way after any edit -- a seam that appears
        // mid-session with nothing to blame it on.
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
    fn core_does_not_keep_its_own_copy() {
        let core = include_str!("../../occluview-core/src/mesh/normals.rs");
        assert!(
            core.contains("use occlu_mesh_edit::coincident_position_key"),
            "core must use the shared key rather than redefining it"
        );
        let redefinition = format!("const {}_EPS_MM", "SMOOTH_POSITION");
        assert!(
            !core.contains(&redefinition),
            "a second tolerance is how the two shadings drifted apart"
        );
    }
}

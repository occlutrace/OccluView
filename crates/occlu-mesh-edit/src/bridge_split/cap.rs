use glam::DVec3;
use std::collections::BTreeMap;

use super::planar_cap::{triangulate_regions, triangulate_regions_best_effort};
use super::rims::{build_closed_cut_loops, build_cut_loops};
use crate::topology::canonical_position_key;
use crate::{
    copy_surviving_vertices, recompute_all_normals, remap_triangle_indices, BridgeSplitError,
    MeshEditBuffers, MeshEditError,
};

pub(crate) fn cap_open_part(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_cut_loops(&mesh, cut_edges)?;
    cap_open_part_from_loops(mesh, loops, expected_normal)
}

/// Cap every recoverable closed cut rim from a surface split.
///
/// Surface inputs may have a natural scan border intersecting the separator.
/// Such a cut path is intentionally left open, but it must not prevent an
/// independent closed cut loop on another connected surface from being capped.
/// A failed loop is isolated so one bad component cannot discard caps that were
/// already valid on the same output part.
pub(crate) fn cap_surface_part_best_effort(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_closed_cut_loops(&mesh, cut_edges)?;
    cap_open_part_from_loops_best_effort(mesh, loops, expected_normal)
}

fn cap_open_part_from_loops(
    mut mesh: MeshEditBuffers,
    loops: Vec<Vec<usize>>,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    // STL and payload-seamed exports can carry several indices at one cut
    // position. Join only the generated cut-rim aliases; source borders and
    // unrelated attribute seams remain untouched.
    weld_cut_rim_aliases(&mut mesh, &loops)?;
    let (cap_indices, capped_loops) = triangulate_regions(&mesh, &loops, expected_normal)?;
    mesh.indices.extend(cap_indices);
    compact_unreferenced_vertices(&mut mesh)?;
    recompute_all_normals(&mut mesh.vertices, &mesh.indices)?;
    Ok((mesh, capped_loops))
}

fn cap_open_part_from_loops_best_effort(
    mut mesh: MeshEditBuffers,
    loops: Vec<Vec<usize>>,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    weld_cut_rim_aliases(&mut mesh, &loops)?;
    let (cap_indices, capped_loops) =
        triangulate_regions_best_effort(&mesh, &loops, expected_normal)?;
    mesh.indices.extend(cap_indices);
    compact_unreferenced_vertices(&mut mesh)?;
    recompute_all_normals(&mut mesh.vertices, &mesh.indices)?;
    Ok((mesh, capped_loops))
}

fn weld_cut_rim_aliases(
    mesh: &mut MeshEditBuffers,
    loops: &[Vec<usize>],
) -> Result<(), MeshEditError> {
    let mut representative_by_position: BTreeMap<[u32; 3], usize> = BTreeMap::new();
    for &vertex_index in loops.iter().flatten() {
        let vertex =
            mesh.vertices
                .get(vertex_index)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut rim vertex is out of range while welding aliases".to_string(),
                })?;
        let key = canonical_position_key(vertex.position);
        representative_by_position
            .entry(key)
            .and_modify(|representative| *representative = (*representative).min(vertex_index))
            .or_insert(vertex_index);
    }
    if representative_by_position.is_empty() {
        return Ok(());
    }

    let mut remap: Vec<u32> = (0..mesh.vertices.len())
        .map(|index| {
            u32::try_from(index).map_err(|_| MeshEditError::MalformedMesh {
                reason: "cut rim vertex count exceeds u32::MAX".to_string(),
            })
        })
        .collect::<Result<_, _>>()?;
    for (index, vertex) in mesh.vertices.iter().enumerate() {
        if let Some(&representative) =
            representative_by_position.get(&canonical_position_key(vertex.position))
        {
            remap[index] =
                u32::try_from(representative).map_err(|_| MeshEditError::MalformedMesh {
                    reason: "cut rim representative exceeds u32::MAX".to_string(),
                })?;
        }
    }
    for index in &mut mesh.indices {
        let remapped =
            remap
                .get(*index as usize)
                .copied()
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut rim alias references an out-of-range index".to_string(),
                })?;
        *index = remapped;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn surface_cap_keeps_valid_loops_when_another_loop_is_unusable() {
        let vertices = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 2.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
            Vec3::new(6.0, 2.0, 0.2),
            Vec3::new(4.0, 2.0, 0.0),
        ];
        let mesh = MeshEditBuffers {
            vertices: vertices
                .into_iter()
                .map(|position| crate::EditVertex::at(position.to_array()))
                .collect(),
            indices: Vec::new(),
            topology: crate::MeshTopology::TriangleMesh,
        };
        let cut_edges = [
            [0, 1],
            [1, 2],
            [2, 3],
            [3, 0],
            [4, 5],
            [5, 6],
            [6, 7],
            [7, 4],
        ];

        let (capped, loop_count) = cap_surface_part_best_effort(mesh, &cut_edges, DVec3::Z)
            .expect("the valid loop must survive the unusable loop");

        assert_eq!(loop_count, 1);
        assert_eq!(capped.indices.len(), 6);
    }
}

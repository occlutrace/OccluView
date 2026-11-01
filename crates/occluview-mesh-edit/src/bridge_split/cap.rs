use glam::DVec3;
use std::collections::BTreeMap;

use super::clip::{ClipProvenance, LineageKey};
use super::planar_cap::{triangulate_regions, triangulate_regions_best_effort};
use super::rims::{
    build_closed_cut_loops_with_provenance, build_cut_loops_with_provenance, CutLoop,
};
#[cfg(test)]
use super::rims::{build_closed_cut_loops, build_cut_loops};
use crate::topology::canonical_position_key;
use crate::{
    copy_surviving_vertices, recompute_all_normals, remap_triangle_indices, BridgeSplitError,
    MeshEditBuffers, MeshEditError,
};

#[cfg(test)]
pub(crate) fn cap_open_part(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_cut_loops(&mesh, cut_edges)?;
    cap_open_part_from_loops(mesh, untagged_loops(loops), None, expected_normal)
}

pub(crate) fn cap_open_part_with_provenance(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: &ClipProvenance,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_cut_loops_with_provenance(&mesh, cut_edges, provenance)?;
    cap_open_part_from_loops(mesh, loops, Some(provenance), expected_normal)
}

/// Cap every recoverable closed cut rim from a surface split.
///
/// Surface inputs may have a natural scan border intersecting the separator.
/// Such a cut path is intentionally left open, but it must not prevent an
/// independent closed cut loop on another connected surface from being capped.
/// A failed loop is isolated so one bad component cannot discard caps that were
/// already valid on the same output part.
#[cfg(test)]
pub(crate) fn cap_surface_part_best_effort(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_closed_cut_loops(&mesh, cut_edges)?;
    cap_open_part_from_loops_best_effort(mesh, untagged_loops(loops), None, expected_normal)
}

pub(crate) fn cap_surface_part_best_effort_with_provenance(
    mesh: MeshEditBuffers,
    cut_edges: &[[u32; 2]],
    provenance: &ClipProvenance,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let loops = build_closed_cut_loops_with_provenance(&mesh, cut_edges, provenance)?;
    cap_open_part_from_loops_best_effort(mesh, loops, Some(provenance), expected_normal)
}

fn cap_open_part_from_loops(
    mut mesh: MeshEditBuffers,
    loops: Vec<CutLoop>,
    provenance: Option<&ClipProvenance>,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    if loops.iter().any(|ring| !ring.cap_eligible) {
        return Err(BridgeSplitError::DamagedCutRim {
            reason: "strict cut rim has ambiguous source lineage".to_string(),
        });
    }
    let rings = loop_vertices(&loops);
    let plan = triangulate_regions(&mesh, &rings, expected_normal)?;
    apply_cut_rim_aliases(&mut mesh, &loops, &plan.accepted_loop_indices, provenance)?;
    mesh.indices.extend(plan.indices);
    compact_unreferenced_vertices(&mut mesh)?;
    recompute_all_normals(&mut mesh.vertices, &mesh.indices)?;
    Ok((mesh, plan.accepted_loop_indices.len()))
}

fn cap_open_part_from_loops_best_effort(
    mut mesh: MeshEditBuffers,
    loops: Vec<CutLoop>,
    provenance: Option<&ClipProvenance>,
    expected_normal: DVec3,
) -> Result<(MeshEditBuffers, usize), BridgeSplitError> {
    let rings = loop_vertices(&loops);
    // Bounds-only component vertices stay in the planner input as rejected
    // geometry. The planner computes their AABBs without treating them as rims.
    let rejected_geometry: Vec<_> = loops
        .iter()
        .map(|ring| !ring.cap_eligible || ring.bounds_only)
        .collect();
    let plan = triangulate_regions_best_effort(
        &mesh,
        &rings,
        &rejected_geometry,
        expected_normal,
    )?;
    apply_cut_rim_aliases(&mut mesh, &loops, &plan.accepted_loop_indices, provenance)?;
    mesh.indices.extend(plan.indices);
    compact_unreferenced_vertices(&mut mesh)?;
    recompute_all_normals(&mut mesh.vertices, &mesh.indices)?;
    Ok((mesh, plan.accepted_loop_indices.len()))
}

#[cfg(test)]
fn untagged_loops(loops: Vec<Vec<usize>>) -> Vec<CutLoop> {
    loops
        .into_iter()
        .map(|vertices| CutLoop {
            lineages: vec![None; vertices.len()],
            vertices,
            cap_eligible: true,
            bounds_only: false,
        })
        .collect()
}

fn loop_vertices(loops: &[CutLoop]) -> Vec<Vec<usize>> {
    loops.iter().map(|ring| ring.vertices.clone()).collect()
}

fn apply_cut_rim_aliases(
    mesh: &mut MeshEditBuffers,
    loops: &[CutLoop],
    accepted_loop_indices: &[usize],
    provenance: Option<&ClipProvenance>,
) -> Result<(), MeshEditError> {
    // Only corners belonging to a cap region that actually triangulated may
    // follow its rim representative. The sidecar is per triangle corner so a
    // shared output row used by another fan keeps that fan's original index.
    let Some(provenance) = provenance else {
        return Ok(());
    };
    if provenance.corner_lineages.len() != mesh.indices.len() {
        return Err(MeshEditError::MalformedMesh {
            reason: "triangle corner lineage count does not match clipped indices".to_string(),
        });
    }

    let mut representative_by_lineage_position =
        BTreeMap::<([u32; 3], LineageKey), u32>::new();
    for &loop_index in accepted_loop_indices {
        let ring = loops
            .get(loop_index)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: "accepted cap loop index is out of range".to_string(),
            })?;
        if !ring.cap_eligible || ring.bounds_only {
            continue;
        }
        if ring.vertices.len() != ring.lineages.len() {
            return Err(MeshEditError::MalformedMesh {
                reason: "cap loop lineage count does not match its vertices".to_string(),
            });
        }
        for (&vertex_index, lineage) in ring.vertices.iter().zip(&ring.lineages) {
            let Some(lineage) = lineage else {
                continue;
            };
            let vertex = mesh
                .vertices
                .get(vertex_index)
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: "cut rim vertex is out of range while matching lineage".to_string(),
                })?;
            let representative = u32::try_from(vertex_index).map_err(|_| {
                MeshEditError::MalformedMesh {
                    reason: "cut rim representative exceeds u32::MAX".to_string(),
                }
            })?;
            representative_by_lineage_position
                .entry((canonical_position_key(vertex.position), *lineage))
                .or_insert(representative);
        }
    }

    for (corner, index) in mesh.indices.iter_mut().enumerate() {
        let lineage = provenance.corner_lineages[corner];
        if lineage.ambiguous {
            continue;
        }
        let vertex = mesh
            .vertices
            .get(*index as usize)
            .ok_or_else(|| MeshEditError::MalformedMesh {
                reason: "cut rim alias references an out-of-range index".to_string(),
            })?;
        if let Some(&representative) = representative_by_lineage_position
            .get(&(canonical_position_key(vertex.position), lineage.key))
        {
            *index = representative;
        }
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
    use crate::bridge_split::clip::{LineageKey, VertexLineage};
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
    #[test]
    fn surface_cap_preserves_a_region_with_an_invalid_nested_rim() {
        let rings = [
            vec![
                [0.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 4.0, 0.0],
                [0.0, 4.0, 0.0],
            ],
            vec![
                [1.0, 1.0, 0.0],
                [3.0, 3.0, 0.0],
                [3.0, 1.0, 0.0],
                [1.0, 3.0, 0.0],
            ],
            vec![
                [8.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 2.0, 0.0],
                [8.0, 2.0, 0.0],
            ],
        ];
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut cut_edges = Vec::new();
        for ring in rings {
            let base = u32::try_from(vertices.len()).expect("small fixture");
            for z in [0.0, 1.0] {
                vertices.extend(ring.iter().map(|p| crate::EditVertex::at([p[0], p[1], z])));
            }
            for i in 0..4 {
                let a = base + i;
                let b = base + (i + 1) % 4;
                indices.extend_from_slice(&[a, b, b + 4, a, b + 4, a + 4]);
                cut_edges.push([a, b]);
            }
        }
        let mut mesh = MeshEditBuffers {
            vertices,
            indices,
            topology: crate::MeshTopology::TriangleMesh,
        };
        const ALIAS_COLOR: [u8; 4] = [19, 81, 207, 255];
        let alias = u32::try_from(mesh.vertices.len()).expect("small fixture");
        for position in [[0.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [-1.0, 1.0, 0.0]] {
            let mut vertex = crate::EditVertex::at(position);
            vertex.color = ALIAS_COLOR;
            mesh.vertices.push(vertex);
        }
        mesh.indices.extend_from_slice(&[alias, alias + 1, alias + 2]);

        let lineage = |vertex| VertexLineage {
            key: LineageKey::SourceCorner { vertex, fan: 0 },
            ambiguous: false,
        };
        let mut corner_lineages: Vec<_> = mesh.indices[..mesh.indices.len() - 3]
            .iter()
            .map(|&vertex| lineage(vertex))
            .collect();
        corner_lineages.extend([lineage(0), lineage(alias + 1), lineage(alias + 2)]);
        let cut_edge_lineages = cut_edges
            .iter()
            .map(|&[first, second]| [lineage(first), lineage(second)])
            .collect();
        let provenance = ClipProvenance {
            corner_lineages,
            cut_edge_lineages,
        };
        let source_faces = mesh.triangle_count();
        let (result, capped) = cap_surface_part_best_effort_with_provenance(
            mesh,
            &cut_edges,
            &provenance,
            DVec3::Z,
        )
            .expect("the independent valid region remains usable");
        assert_eq!(
            capped, 1,
            "an invalid inner contour cannot disappear into an outer lid"
        );
        assert_eq!(result.triangle_count(), source_faces + 2);
        let alias_after_index = result
            .vertices
            .iter()
            .position(|vertex| vertex.color == ALIAS_COLOR && vertex.position == [0.0, 0.0, 0.0])
            .expect("the alias payload survives compaction");
        let alias_after = u32::try_from(alias_after_index).expect("small output");
        assert!(result.indices.contains(&alias_after));
        assert!(result.vertices.iter().enumerate().any(|(index, vertex)| {
            index != alias_after_index && vertex.position == [0.0, 0.0, 0.0]
        }));
    }

    #[test]
    fn ambiguous_nested_rim_keeps_its_lumen_open_and_caps_a_distant_region() {
        const TANGENT_COLOR: [u8; 4] = [27, 91, 203, 255];
        let rings = [
            [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]],
            [[8.0, 0.0], [10.0, 0.0], [10.0, 2.0], [8.0, 2.0]],
        ];
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut cut_edges = Vec::new();
        for ring in rings {
            let base = u32::try_from(vertices.len()).expect("small fixture");
            for z in [0.0, 1.0] {
                vertices.extend(
                    ring.iter()
                        .map(|point| crate::EditVertex::at([point[0], point[1], z])),
                );
            }
            for edge in 0..4 {
                let first = base + edge;
                let second = base + (edge + 1) % 4;
                indices.extend_from_slice(&[
                    first,
                    second,
                    second + 4,
                    first,
                    second + 4,
                    first + 4,
                ]);
                cut_edges.push([first, second]);
            }
        }

        // A tangent face repeats one inner-rim segment at distinct payload
        // vertices. Its geometric cycle stays closed, but its lineage is
        // ambiguous and must block the enclosing outer lid.
        let tangent = u32::try_from(vertices.len()).expect("small fixture");
        for position in [[1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [3.2, 1.4, 1.0]] {
            let mut vertex = crate::EditVertex::at(position);
            vertex.color = TANGENT_COLOR;
            vertices.push(vertex);
        }
        indices.extend_from_slice(&[tangent, tangent + 1, tangent + 2]);
        cut_edges.push([tangent, tangent + 1]);
        let mesh = MeshEditBuffers {
            vertices,
            indices,
            topology: crate::MeshTopology::TriangleMesh,
        };

        let lineage = |vertex| VertexLineage {
            key: LineageKey::SourceCorner { vertex, fan: 0 },
            ambiguous: false,
        };
        let provenance = ClipProvenance {
            corner_lineages: mesh.indices.iter().map(|&vertex| lineage(vertex)).collect(),
            cut_edge_lineages: cut_edges
                .iter()
                .map(|&[first, second]| [lineage(first), lineage(second)])
                .collect(),
        };
        let source_faces = mesh.triangle_count();
        assert!(matches!(
            cap_open_part_with_provenance(
                mesh.clone(),
                &cut_edges,
                &provenance,
                DVec3::Z,
            ),
            Err(BridgeSplitError::DamagedCutRim { .. })
        ));
        let (result, capped) = cap_surface_part_best_effort_with_provenance(
            mesh,
            &cut_edges,
            &provenance,
            DVec3::Z,
        )
        .expect("the distant independent region remains cap-able");

        assert_eq!(capped, 1);
        assert_eq!(result.triangle_count(), source_faces + 2);
        let new_cap_faces = &result.indices.as_chunks::<3>().0[source_faces..];
        assert!(new_cap_faces.iter().flatten().all(|&index| {
            result.vertices[index as usize].position[0] >= 8.0
        }));

        let tangent_face = result
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .find(|face| {
                face.iter()
                    .all(|&index| result.vertices[index as usize].color == TANGENT_COLOR)
            })
            .expect("the tangent face remains in the output");
        for position in [[1.0, 1.0, 0.0], [3.0, 1.0, 0.0]] {
            let tangent_index = tangent_face
                .iter()
                .copied()
                .find(|&index| result.vertices[index as usize].position == position)
                .expect("the tangent endpoint remains referenced");
            let shell_index = result
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|face| {
                    !face.iter().all(|&index| {
                        result.vertices[index as usize].color == TANGENT_COLOR
                    })
                })
                .flat_map(|face| face.iter().copied())
                .find(|&index| result.vertices[index as usize].position == position)
                .expect("the inner shell keeps its separate rim endpoint");
            assert_ne!(tangent_index, shell_index);
        }
    }
}

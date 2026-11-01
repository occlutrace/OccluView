use super::{
    closed_cube, closed_l_prism, closed_torus, exploded_cube_with_payload_seams,
    exploded_cube_with_uniform_payload, hollow_cube_with_payload, request, translated_cube,
};
use crate::bridge_split::clip::clip_bridge_surface_open;
use crate::topology::{canonical_topology, TopologyWeldPolicy};
use crate::topology_analysis::{connected_components, edge_incidence, topology_defects};
use crate::{
    split_bridge, split_bridge_surface, BridgeSplitRequest, BridgeSplitResult, EditVertex,
    MeshEditBuffers,
};
use glam::Vec3;

fn assert_closed_manifold(mesh: &MeshEditBuffers) {
    let canonical = canonical_topology(mesh, TopologyWeldPolicy::PositionOnly)
        .expect("canonical output topology");
    let incidence = edge_incidence(canonical.indices());
    let defects = topology_defects(canonical.indices(), &incidence);
    assert_eq!(defects.boundary_edges, 0);
    assert_eq!(defects.non_manifold_edges, 0);
    assert_eq!(defects.inconsistent_winding_edges, 0);
    assert_eq!(defects.non_manifold_vertices, 0);
    assert_eq!(
        connected_components(canonical.indices(), &incidence)
            .members
            .len(),
        1
    );
}

fn assert_result_manufacturable(result: &BridgeSplitResult) {
    assert_closed_manifold(&result.part_a);
    assert_closed_manifold(&result.part_b);
    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    for mesh in [&result.part_a, &result.part_b] {
        for face in mesh.indices.as_chunks::<3>().0 {
            assert_ne!(face[0], face[1]);
            assert_ne!(face[1], face[2]);
            assert_ne!(face[2], face[0]);
            let a = Vec3::from_array(mesh.vertices[face[0] as usize].position);
            let b = Vec3::from_array(mesh.vertices[face[1] as usize].position);
            let c = Vec3::from_array(mesh.vertices[face[2] as usize].position);
            assert!((b - a).cross(c - a).length_squared() > 0.0);
        }
    }
}

#[test]
fn cube_produces_two_closed_manifold_parts_with_requested_gap() {
    let result = split_bridge(&closed_cube(), request()).expect("cube splits and caps");
    assert_result_manufacturable(&result);
    let positive_min = result
        .part_a
        .vertices
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::INFINITY, f32::min);
    let negative_max = result
        .part_b
        .vertices
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((positive_min - negative_max - 0.05).abs() <= 2.0e-6);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn opposite_caps_face_outward() {
    let result = split_bridge(&closed_cube(), request()).expect("cube splits and caps");
    let cap_normal_x = |mesh: &MeshEditBuffers, boundary_x: f32| -> Vec<f32> {
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|face| {
                let points = [
                    Vec3::from_array(mesh.vertices[face[0] as usize].position),
                    Vec3::from_array(mesh.vertices[face[1] as usize].position),
                    Vec3::from_array(mesh.vertices[face[2] as usize].position),
                ];
                points
                    .iter()
                    .all(|point| (point.x - boundary_x).abs() <= 1.0e-6)
                    .then(|| (points[1] - points[0]).cross(points[2] - points[0]).x)
            })
            .collect()
    };
    let positive_cap = cap_normal_x(&result.part_a, 0.025);
    let negative_cap = cap_normal_x(&result.part_b, -0.025);
    assert!(!positive_cap.is_empty() && positive_cap.iter().all(|normal| *normal < 0.0));
    assert!(!negative_cap.is_empty() && negative_cap.iter().all(|normal| *normal > 0.0));
}

#[test]
fn uniform_stl_soup_matches_indexed_manifold_behavior() {
    let result = split_bridge(&exploded_cube_with_uniform_payload(), request())
        .expect("soup splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn payload_seam_soup_succeeds_end_to_end() {
    let result = split_bridge(&exploded_cube_with_payload_seams(), request())
        .expect("payload-seamed soup splits and caps");
    assert_result_manufacturable(&result);
}

#[test]
fn surface_split_caps_nested_connector_surfaces_as_a_planar_ring() {
    let result = split_bridge_surface(&hollow_cube_with_payload(), request())
        .expect("nested surface shells split");

    for part in [&result.part_a, &result.part_b] {
        let canonical = canonical_topology(part, TopologyWeldPolicy::PositionOnly)
            .expect("canonical output topology");
        let incidence = edge_incidence(canonical.indices());
        let defects = topology_defects(canonical.indices(), &incidence);
        assert_eq!(defects.boundary_edges, 0, "surface cap left a boundary");
        assert_eq!(defects.non_manifold_edges, 0);
        assert_eq!(defects.inconsistent_winding_edges, 0);
        assert_eq!(
            connected_components(canonical.indices(), &incidence)
                .members
                .len(),
            1
        );
        assert!(part
            .vertices
            .iter()
            .all(|vertex| vertex.position[0].abs() >= 0.024_999));
    }
    assert_eq!(result.report.part_a_cut_loops, 2);
    assert_eq!(result.report.part_b_cut_loops, 2);
}

#[test]
fn surface_split_keeps_lumen_open_for_coplanar_branch_and_caps_far_region() {
    const TANGENT_COLOR: [u8; 4] = [23, 101, 219, 255];
    let request = BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: Vec3::Z,
        kerf_mm: 0.05,
        disc_radius_mm: 60.0,
        max_disc_radius_mm: 60.0,
    };
    let mut source = hollow_cube_with_payload();
    let far_region = translated_cube(Vec3::new(4.0, 0.0, 0.0));
    let far_offset = u32::try_from(source.vertices.len()).expect("small fixture");
    source.vertices.extend(far_region.vertices);
    source
        .indices
        .extend(far_region.indices.into_iter().map(|index| index + far_offset));

    // Read an actual inner-rim segment from the clipper so this fixture follows
    // any source-triangle subdivision of the generated cut line.
    let baseline = clip_bridge_surface_open(&source, request)
        .expect("baseline surface clip exposes the inner rim");
    let inner_segment = baseline
        .part_a_cut_edges
        .iter()
        .find_map(|&[first, second]| {
            let first = baseline.part_a.vertices[first as usize].position;
            let second = baseline.part_a.vertices[second as usize].position;
            let on_inner_rim = |point: [f32; 3]| {
                (point[1] + 0.5).abs() <= 1.0e-6
                    && (point[2] - 0.025).abs() <= 1.0e-6
                    && point[0].abs() <= 0.5 + 1.0e-6
            };
            (on_inner_rim(first)
                && on_inner_rim(second)
                && (first[0] - second[0]).abs() > 1.0e-6)
            .then_some([first, second])
        })
        .expect("the clipper emits a horizontal inner-rim segment at +kerf/2");
    let plane_z = (inner_segment[0][2] + inner_segment[1][2]) * 0.5;
    assert!((plane_z - 0.025).abs() <= 1.0e-6);
    let tangent_start = u32::try_from(source.vertices.len()).expect("small fixture");
    for position in [
        inner_segment[0],
        inner_segment[1],
        [
            (inner_segment[0][0] + inner_segment[1][0]) * 0.5,
            0.0,
            plane_z,
        ],
    ] {
        let mut vertex = EditVertex::at(position);
        vertex.color = TANGENT_COLOR;
        source.vertices.push(vertex);
    }
    source
        .indices
        .extend([tangent_start, tangent_start + 1, tangent_start + 2]);

    let result = split_bridge_surface(&source, request)
        .expect("branched inner cut preserves the useful surface split");
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 3);

    let tangent_face = result
        .part_a
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .find(|face| {
            face.iter()
                .all(|&index| result.part_a.vertices[index as usize].color == TANGENT_COLOR)
        })
        .expect("the coplanar source triangle remains in Part A");
    for position in [inner_segment[0], inner_segment[1]] {
        let tangent_index = tangent_face
            .iter()
            .copied()
            .find(|&index| result.part_a.vertices[index as usize].position == position)
            .expect("the tangent face retains its generated-rim endpoint");
        let shell_index = result
            .part_a
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|face| {
                !face.iter().all(|&index| {
                    result.part_a.vertices[index as usize].color == TANGENT_COLOR
                })
            })
            .flat_map(|face| face.iter().copied())
            .find(|&index| result.part_a.vertices[index as usize].position == position)
            .expect("the inner shell retains its own rim endpoint");
        assert_ne!(tangent_index, shell_index);
    }

    let planar_non_tangent_faces: Vec<_> = result
        .part_a
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|face| {
            face.iter().all(|&index| {
                (result.part_a.vertices[index as usize].position[2] - plane_z).abs() <= 1.0e-6
            }) && !face.iter().all(|&index| {
                result.part_a.vertices[index as usize].color == TANGENT_COLOR
            })
        })
        .collect();
    assert!(!planar_non_tangent_faces.is_empty());
    assert!(planar_non_tangent_faces.iter().all(|face| {
        face.iter().all(|index| {
            result.part_a.vertices[*index as usize].position[0] >= 3.0
        })
    }));
}

#[test]
fn surface_split_keeps_a_one_point_payload_touch_separate_from_the_cap() {
    const TANGENT_COLOR: [u8; 4] = [17, 93, 211, 255];
    let mut source = closed_cube();
    let tangent = u32::try_from(source.vertices.len()).expect("small fixture");
    for position in [
        [0.025, -1.0, -1.0],
        [1.0, -0.8, -1.0],
        [1.0, -1.0, -0.8],
    ] {
        let mut vertex = EditVertex::at(position);
        vertex.color = TANGENT_COLOR;
        source.vertices.push(vertex);
    }
    source.indices.extend([tangent, tangent + 1, tangent + 2]);

    let result = split_bridge_surface(&source, request()).expect("surface split succeeds");
    assert_eq!(result.report.part_a_cut_loops, 1);
    let tangent_face = result
        .part_a
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .find(|face| {
            face.iter()
                .all(|&index| result.part_a.vertices[index as usize].color == TANGENT_COLOR)
        })
        .expect("the one-point triangle remains in Part A");
    let tangent_corner = tangent_face
        .iter()
        .copied()
        .find(|&index| result.part_a.vertices[index as usize].position == [0.025, -1.0, -1.0])
        .expect("tangent corner lies on the cap rim");
    let shell_corner = result
        .part_a
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|face| !face.iter().all(|&index| {
            result.part_a.vertices[index as usize].color == TANGENT_COLOR
        }))
        .flat_map(|face| face.iter().copied())
        .find(|&index| {
            result.part_a.vertices[index as usize].position == [0.025, -1.0, -1.0]
        })
        .expect("the shell still uses its own rim vertex");

    assert_ne!(tangent_corner, shell_corner);
}

#[test]
fn surface_split_leaves_a_rim_coincident_tangent_edge_uncapped() {
    const TANGENT_COLOR: [u8; 4] = [19, 97, 213, 255];
    let mut source = closed_cube();
    let tangent = u32::try_from(source.vertices.len()).expect("small fixture");
    for position in [
        [0.025, -1.0, -1.0],
        [0.025, -1.0, 0.025],
        [1.0, -0.8, -0.4],
    ] {
        let mut vertex = EditVertex::at(position);
        vertex.color = TANGENT_COLOR;
        source.vertices.push(vertex);
    }
    source.indices.extend([tangent, tangent + 1, tangent + 2]);

    let result = split_bridge_surface(&source, request()).expect("surface split succeeds");
    assert_eq!(result.report.part_a_cut_loops, 0);
    let tangent_face = result
        .part_a
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .find(|face| {
            face.iter()
                .all(|&index| result.part_a.vertices[index as usize].color == TANGENT_COLOR)
        })
        .expect("the ambiguous tangent face remains");
    for &tangent_corner in tangent_face {
        let position = result.part_a.vertices[tangent_corner as usize].position;
        if position[0] != 0.025 || position[1] != -1.0 {
            continue;
        }
        let shell_corner = result
            .part_a
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|face| !face.iter().all(|&index| {
                result.part_a.vertices[index as usize].color == TANGENT_COLOR
            }))
            .flat_map(|face| face.iter().copied())
            .find(|&index| result.part_a.vertices[index as usize].position == position)
            .expect("the cube retains its own rim vertex");
        assert_ne!(tangent_corner, shell_corner);
    }
}

#[test]
fn exact_original_stl_vertex_fan_closes_through_source_corner_aliases() {
    let mut source = exploded_cube_with_uniform_payload();
    for vertex in &mut source.vertices {
        if vertex.position == [1.0, 1.0, 1.0] {
            vertex.position[0] = 0.025;
        }
    }
    assert!(source.indices.as_chunks::<3>().0.iter().any(|face| {
        face.iter()
            .filter(|&&index| source.vertices[index as usize].position[0] == 0.025)
            .count()
            == 1
            && face
                .iter()
                .all(|&index| source.vertices[index as usize].position[0] >= 0.025)
    }));

    let result = split_bridge(&source, request()).expect("source-corner soup closes");

    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 1);
}

#[test]
fn near_plane_source_vertex_caps_without_a_split_rim() {
    let mut mesh = closed_cube();
    mesh.vertices[1].position[0] = 0.025_001;
    let result = split_bridge(&mesh, request()).expect("snapped vertex remains a closed rim");
    assert_result_manufacturable(&result);
}

#[test]
fn every_simple_cut_loop_is_capped_atomically() {
    let result = split_bridge(&closed_torus(32, 16), request()).expect("torus splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 2);
    assert_eq!(result.report.part_b_cut_loops, 2);
}

#[test]
fn concave_planar_connector_loop_caps_completely() {
    let result = split_bridge(&closed_l_prism(), request()).expect("concave prism splits and caps");
    assert_result_manufacturable(&result);
    assert_eq!(result.report.part_a_cut_loops, 1);
    assert_eq!(result.report.part_b_cut_loops, 1);
}

#[test]
fn open_surface_split_preserves_natural_source_borders() {
    let mut source = closed_cube();
    source.indices.drain(0..3);

    let result = split_bridge_surface(&source, request()).expect("open surface clips");

    assert!(!result.part_a.indices.is_empty());
    assert!(!result.part_b.indices.is_empty());
    assert!(!result.report.parts_closed);
    assert_eq!(
        result.report.part_a_triangles,
        result.part_a.triangle_count()
    );
    assert_eq!(
        result.report.part_b_triangles,
        result.part_b.triangle_count()
    );
}

#[test]
fn complete_split_is_deterministic() {
    let mesh = closed_torus(20, 10);
    let first = split_bridge(&mesh, request()).expect("first split");
    let second = split_bridge(&mesh, request()).expect("second split");
    assert_eq!(first, second);
}

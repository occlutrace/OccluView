use super::*;
use crate::{EditVertex, MeshTopology};

fn v(position: [f32; 3]) -> EditVertex {
    EditVertex::at(position)
}

fn mesh(vertices: Vec<EditVertex>, indices: Vec<u32>) -> MeshEditBuffers {
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

/// Unit cube, 8 shared vertices, 12 outward-wound triangles (volume +1).
fn cube() -> MeshEditBuffers {
    let vertices = vec![
        v([0.0, 0.0, 0.0]),
        v([1.0, 0.0, 0.0]),
        v([1.0, 1.0, 0.0]),
        v([0.0, 1.0, 0.0]),
        v([0.0, 0.0, 1.0]),
        v([1.0, 0.0, 1.0]),
        v([1.0, 1.0, 1.0]),
        v([0.0, 1.0, 1.0]),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // bottom (-z)
        4, 5, 6, 4, 6, 7, // top (+z)
        0, 1, 5, 0, 5, 4, // front (-y)
        2, 3, 7, 2, 7, 6, // back (+y)
        0, 4, 7, 0, 7, 3, // left (-x)
        1, 2, 6, 1, 6, 5, // right (+x)
    ];
    mesh(vertices, indices)
}

/// Explode a mesh into STL-style soup: every triangle gets private vertices.
fn soup(source: &MeshEditBuffers) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(source.indices.len());
    let mut indices = Vec::with_capacity(source.indices.len());
    for &index in &source.indices {
        indices.push(vertices.len() as u32);
        vertices.push(source.vertices[index as usize]);
    }
    mesh(vertices, indices)
}

/// Flat n×n-vertex grid in z=0, wound +z.
fn grid(n: usize) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(n * n);
    for k in 0..n * n {
        vertices.push(v([(k % n) as f32, (k / n) as f32, 0.0]));
    }
    let mut indices = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let i = (y * n + x) as u32;
            let nn = n as u32;
            indices.extend_from_slice(&[i, i + 1, i + nn + 1, i, i + nn + 1, i + nn]);
        }
    }
    mesh(vertices, indices)
}

/// Closed-top open-bottom "cup": apex fan + wall ring, outward-wound. One
/// apex-fan triangle is omitted, leaving a 3-edge pinhole well away from the
/// bottom rim, which has `sides` edges and is the natural boundary.
fn cup_with_pinhole(sides: usize) -> MeshEditBuffers {
    let mut vertices = vec![v([0.0, 0.0, 1.0])];
    for ring_z in [1.0_f32, 0.0] {
        for i in 0..sides {
            let angle = std::f32::consts::TAU * (i as f32) / (sides as f32);
            vertices.push(v([angle.cos(), angle.sin(), ring_z]));
        }
    }
    let top = |i: usize| (1 + i % sides) as u32;
    let bottom = |i: usize| (1 + sides + i % sides) as u32;
    let mut indices = Vec::new();
    for i in 0..sides {
        if i != 0 {
            indices.extend_from_slice(&[0, top(i), top(i + 1)]);
        }
        indices.extend_from_slice(&[top(i), bottom(i), bottom(i + 1)]);
        indices.extend_from_slice(&[top(i), bottom(i + 1), top(i + 1)]);
    }
    mesh(vertices, indices)
}

fn flip_triangle(buffers: &mut MeshEditBuffers, triangle: usize) {
    buffers.indices.swap(triangle * 3 + 1, triangle * 3 + 2);
}

fn repair(buffers: &MeshEditBuffers) -> RepairResult {
    repair_mesh(buffers, RepairOptions::default()).expect("repair succeeds")
}

/// Count of undirected edges carried by exactly one face.
fn boundary_edge_count(buffers: &MeshEditBuffers) -> usize {
    edge_runs(buffers)
        .into_iter()
        .filter(|&count| count == 1)
        .count()
}

fn max_edge_face_count(buffers: &MeshEditBuffers) -> usize {
    edge_runs(buffers).into_iter().max().unwrap_or(0)
}

fn edge_runs(buffers: &MeshEditBuffers) -> Vec<usize> {
    let incidence = edge_incidence(&buffers.indices);
    let mut runs = Vec::new();
    let mut start = 0;
    while start < incidence.len() {
        let mut end = start + 1;
        while end < incidence.len() && incidence[end].0 == incidence[start].0 {
            end += 1;
        }
        runs.push(end - start);
        start = end;
    }
    runs
}

/// Every shared edge must be traversed in opposite directions by its faces.
fn directed_edges_opposed(buffers: &MeshEditBuffers) -> bool {
    let mut directed = Vec::with_capacity(buffers.indices.len());
    for tri in buffers.indices.chunks_exact(3) {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            directed.push((a, b));
        }
    }
    directed.sort_unstable();
    directed.windows(2).all(|pair| pair[0] != pair[1])
}

fn signed_volume(buffers: &MeshEditBuffers) -> f64 {
    let mut six_volume = 0.0_f64;
    for tri in buffers.indices.chunks_exact(3) {
        let p = |index: u32| {
            let position = buffers.vertices[index as usize].position;
            DVec3::new(
                f64::from(position[0]),
                f64::from(position[1]),
                f64::from(position[2]),
            )
        };
        six_volume += p(tri[0]).dot(p(tri[1]).cross(p(tri[2])));
    }
    six_volume / 6.0
}

fn component_count(buffers: &MeshEditBuffers) -> usize {
    let incidence = edge_incidence(&buffers.indices);
    connected_components(&buffers.indices, &incidence)
        .members
        .len()
}

/// Position-based canonical triangle set (winding- and index-agnostic),
/// quantized so it is comparable across runs.
fn triangle_position_set(buffers: &MeshEditBuffers) -> Vec<[[i64; 3]; 3]> {
    let quantize = |index: u32| {
        let p = buffers.vertices[index as usize].position;
        [
            (f64::from(p[0]) * 1e6).round() as i64,
            (f64::from(p[1]) * 1e6).round() as i64,
            (f64::from(p[2]) * 1e6).round() as i64,
        ]
    };
    let mut set: Vec<[[i64; 3]; 3]> = buffers
        .indices
        .chunks_exact(3)
        .map(|tri| {
            let mut corners = [quantize(tri[0]), quantize(tri[1]), quantize(tri[2])];
            corners.sort_unstable();
            corners
        })
        .collect();
    set.sort_unstable();
    set
}

#[test]
fn stl_soup_cube_welds_watertight() {
    let result = repair(&soup(&cube()));
    assert_eq!(result.report.welded_vertices, 28);
    assert_eq!(result.mesh.vertices.len(), 8);
    assert_eq!(result.mesh.triangle_count(), 12);
    assert_eq!(boundary_edge_count(&result.mesh), 0);
    assert!(signed_volume(&result.mesh) > 0.0);
    assert!(result.report.warnings.is_empty());
}

#[test]
fn same_positions_different_colors_are_not_welded() {
    let mut soup_mesh = soup(&mesh(
        vec![v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0]), v([0.0, 1.0, 0.0])],
        vec![0, 1, 2, 0, 1, 2],
    ));
    for vertex in &mut soup_mesh.vertices[..3] {
        vertex.color = [200, 10, 10, 255];
    }
    for vertex in &mut soup_mesh.vertices[3..] {
        vertex.color = [10, 10, 200, 255];
    }
    let result = repair(&soup_mesh);
    assert_eq!(result.report.welded_vertices, 0);
    assert_eq!(result.report.output_vertices, 6);
}

#[test]
fn degenerate_triangles_are_removed_with_exact_counts() {
    let mut broken = cube();
    broken.vertices.push(v([2.0, 0.0, 0.0]));
    broken.indices.extend_from_slice(&[0, 1, 8]); // exactly collinear sliver
    broken.indices.extend_from_slice(&[0, 0, 1]); // repeated index
    let result = repair(&broken);
    assert_eq!(result.report.removed_degenerate_triangles, 2);
    assert_eq!(result.report.removed_unreferenced_vertices, 1);
    assert_eq!(result.mesh.triangle_count(), 12);
    assert_eq!(boundary_edge_count(&result.mesh), 0);
}

#[test]
fn duplicate_faces_same_and_opposite_winding_are_removed() {
    let mut doubled = cube();
    doubled.indices.extend_from_slice(&[4, 5, 6]); // same winding
    doubled.indices.extend_from_slice(&[4, 6, 5]); // opposite winding
    let result = repair(&doubled);
    assert_eq!(result.report.removed_duplicate_triangles, 2);
    assert_eq!(result.mesh.triangle_count(), 12);
    assert!(signed_volume(&result.mesh) > 0.0);
}

#[test]
fn nonmanifold_fin_is_split_to_border() {
    // A small fin triangle glued onto an interior grid edge: that edge gets
    // three incident faces. The two larger grid faces must keep the original
    // edge; the fin is re-pointed onto duplicated vertices.
    let mut finned = grid(10);
    let apex = finned.vertices.len() as u32;
    finned.vertices.push(v([0.55, 0.45, 0.1]));
    finned.indices.extend_from_slice(&[0, 11, apex]);
    let result = repair(&finned);
    assert_eq!(result.report.split_nonmanifold_edges, 1);
    assert!(max_edge_face_count(&result.mesh) <= 2);
    assert!(result.report.output_vertices > result.report.input_vertices);
}

#[test]
fn bowtie_vertex_is_split_into_separable_fans() {
    let bowtie = mesh(
        vec![
            v([0.0, 0.0, 0.0]),
            v([1.0, 0.0, 0.0]),
            v([0.0, 1.0, 0.0]),
            v([-1.0, 0.0, 0.0]),
            v([0.0, -1.0, 0.0]),
        ],
        vec![0, 1, 2, 0, 3, 4],
    );
    let result = repair(&bowtie);
    assert_eq!(result.report.split_bowtie_vertices, 1);
    assert_eq!(component_count(&result.mesh), 2);
    assert!(result.report.output_vertices > result.report.input_vertices);
}

#[test]
fn minority_flipped_faces_are_reoriented() {
    let mut flipped = cube();
    for triangle in [0, 3, 7] {
        flip_triangle(&mut flipped, triangle);
    }
    let result = repair(&flipped);
    assert_eq!(result.report.reoriented_triangles, 3);
    assert_eq!(result.report.flipped_components, 0);
    assert!(directed_edges_opposed(&result.mesh));
    assert!(signed_volume(&result.mesh) > 0.0);
}

#[test]
fn inside_out_cube_is_flipped_outward() {
    let mut inverted = cube();
    for triangle in 0..12 {
        flip_triangle(&mut inverted, triangle);
    }
    let result = repair(&inverted);
    assert_eq!(result.report.flipped_components, 1);
    assert_eq!(result.report.reoriented_triangles, 0);
    assert!(signed_volume(&result.mesh) > 0.0);
}

#[test]
fn open_sheet_gets_majority_fix_but_never_volume_flip() {
    let mut sheet = grid(10);
    flip_triangle(&mut sheet, 0);
    let result = repair(&sheet);
    assert_eq!(result.report.reoriented_triangles, 1);
    assert_eq!(result.report.flipped_components, 0);
    assert!(directed_edges_opposed(&result.mesh));
    assert_eq!(result.report.filled_holes, 0);
    assert_eq!(result.report.open_rims_left, 1);
}

#[test]
fn tiny_distant_speck_is_dropped_as_debris() {
    // The big component must dwarf the speck: 1 face < 2% of 162 faces AND
    // the speck extent < 10% of the whole-mesh diagonal.
    let mut littered = grid(10);
    let base = littered.vertices.len() as u32;
    littered.vertices.push(v([100.0, 100.0, 100.0]));
    littered.vertices.push(v([100.001, 100.0, 100.0]));
    littered.vertices.push(v([100.0, 100.001, 100.0]));
    littered
        .indices
        .extend_from_slice(&[base, base + 1, base + 2]);
    let result = repair(&littered);
    assert_eq!(result.report.removed_debris_components, 1);
    assert_eq!(result.report.removed_debris_triangles, 1);
    assert_eq!(result.report.removed_unreferenced_vertices, 3);
    assert_eq!(result.mesh.triangle_count(), 162);
}

#[test]
fn speck_connected_by_an_edge_is_kept() {
    let mut sheet = grid(10);
    let apex = sheet.vertices.len() as u32;
    sheet.vertices.push(v([0.5, -0.5, 0.0]));
    sheet.indices.extend_from_slice(&[1, 0, apex]);
    let result = repair(&sheet);
    assert_eq!(result.report.removed_debris_components, 0);
    assert_eq!(result.report.removed_debris_triangles, 0);
    assert_eq!(result.mesh.triangle_count(), 163);
}

#[test]
fn pinhole_is_filled_and_natural_boundary_stays_open() {
    let cup = cup_with_pinhole(40);
    let result = repair(&cup);
    assert_eq!(result.report.filled_holes, 1);
    assert_eq!(result.report.open_rims_left, 1);
    assert!(result.report.warnings.is_empty());
    assert_eq!(result.mesh.triangle_count(), cup.triangle_count() + 1);
    assert_eq!(boundary_edge_count(&result.mesh), 40);
}

#[test]
fn repair_is_idempotent() {
    let mut messy = soup(&cube());
    for triangle in [0, 3, 7] {
        flip_triangle(&mut messy, triangle);
    }
    let base = messy.vertices.len() as u32;
    messy.vertices.push(v([100.0, 100.0, 100.0]));
    messy.vertices.push(v([100.001, 100.0, 100.0]));
    messy.vertices.push(v([100.0, 100.001, 100.0]));
    messy.indices.extend_from_slice(&[base, base + 1, base + 2]);

    let first = repair(&messy);
    assert!(first.report.changed_content());
    let second = repair(&first.mesh);
    assert!(!second.report.changed_content());
    assert_eq!(second.mesh, first.mesh);
    assert_eq!(second.report.output_vertices, first.mesh.vertices.len());
}

#[test]
fn repair_is_deterministic_and_shuffle_invariant() {
    let messy = soup(&cube());
    let first = repair(&messy);
    let second = repair(&messy);
    assert_eq!(first.mesh, second.mesh);
    assert_eq!(first.report, second.report);

    // Index-shuffled clone: reversed triangle order, same content.
    let mut shuffled_indices = Vec::with_capacity(messy.indices.len());
    for tri in messy.indices.chunks_exact(3).rev() {
        shuffled_indices.extend_from_slice(tri);
    }
    let shuffled = mesh(messy.vertices.clone(), shuffled_indices);
    let third = repair(&shuffled);
    assert_eq!(third.report.welded_vertices, first.report.welded_vertices);
    assert_eq!(
        triangle_position_set(&third.mesh),
        triangle_position_set(&first.mesh)
    );
    assert!((signed_volume(&third.mesh) - signed_volume(&first.mesh)).abs() < 1e-9);
}

#[test]
fn clean_mesh_returns_exact_input_clone() {
    let mut clean = cube();
    // Stale, obviously wrong normals must survive a no-op repair untouched.
    for vertex in &mut clean.vertices {
        vertex.normal = [9.0, 9.0, 9.0];
    }
    let result = repair(&clean);
    assert!(!result.report.changed_content());
    assert_eq!(result.mesh, clean);
    assert_eq!(result.report.output_vertices, 8);
    assert_eq!(result.report.output_triangles, 12);
}

#[test]
fn point_clouds_are_rejected() {
    let cloud = MeshEditBuffers {
        vertices: vec![v([0.0, 0.0, 0.0])],
        indices: Vec::new(),
        topology: MeshTopology::PointCloud,
    };
    let err = repair_mesh(&cloud, RepairOptions::default()).expect_err("point cloud rejected");
    assert_eq!(err, MeshEditError::UnsupportedPointCloud);
}

#[test]
fn invalid_options_are_rejected() {
    let cube = cube();
    for options in [
        RepairOptions {
            weld_epsilon: 0.0,
            ..RepairOptions::default()
        },
        RepairOptions {
            weld_epsilon: f32::NAN,
            ..RepairOptions::default()
        },
        RepairOptions {
            tiny_hole_max_edges: 0,
            ..RepairOptions::default()
        },
        RepairOptions {
            debris_face_fraction: -0.1,
            ..RepairOptions::default()
        },
        RepairOptions {
            debris_diameter_fraction: f32::NAN,
            ..RepairOptions::default()
        },
    ] {
        let err = repair_mesh(&cube, options).expect_err("invalid options rejected");
        assert!(matches!(err, MeshEditError::InvalidOptions { .. }));
    }
}

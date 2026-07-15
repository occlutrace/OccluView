use super::super::{BridgeSplitRequest, MeshEditBuffers};
use crate::{EditVertex, MeshTopology};
use glam::Vec3;

mod cap;
mod clip;
mod hostile;
mod manifold;
mod preflight;

fn request() -> BridgeSplitRequest {
    BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: Vec3::X,
        kerf_mm: 0.05,
        disc_radius_mm: 60.0,
        max_disc_radius_mm: 60.0,
    }
}

fn closed_cube() -> MeshEditBuffers {
    let positions = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // -Z
        4, 5, 6, 4, 6, 7, // +Z
        0, 1, 5, 0, 5, 4, // -Y
        3, 7, 6, 3, 6, 2, // +Y
        0, 4, 7, 0, 7, 3, // -X
        1, 2, 6, 1, 6, 5, // +X
    ];
    MeshEditBuffers {
        vertices: positions.into_iter().map(EditVertex::at).collect(),
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn exploded_cube_with_payload_seams() -> MeshEditBuffers {
    let indexed = closed_cube();
    let mut vertices = Vec::with_capacity(indexed.indices.len());
    let mut indices = Vec::with_capacity(indexed.indices.len());
    for (corner, &source_index) in indexed.indices.iter().enumerate() {
        let mut vertex = indexed.vertices[source_index as usize];
        let face = corner / 3;
        vertex.uv = [face as f32 / 12.0, (corner % 3) as f32 / 2.0];
        vertex.color = [face as u8, 255_u8.saturating_sub(face as u8), 127, 255];
        vertices.push(vertex);
        indices.push(u32::try_from(corner).expect("small fixture"));
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn exploded_cube_with_uniform_payload() -> MeshEditBuffers {
    let indexed = closed_cube();
    let mut vertices = Vec::with_capacity(indexed.indices.len());
    let mut indices = Vec::with_capacity(indexed.indices.len());
    for (corner, &source_index) in indexed.indices.iter().enumerate() {
        vertices.push(indexed.vertices[source_index as usize]);
        indices.push(u32::try_from(corner).expect("small fixture"));
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn translated_cube(offset: Vec3) -> MeshEditBuffers {
    let mut cube = closed_cube();
    for vertex in &mut cube.vertices {
        let position = Vec3::from_array(vertex.position) + offset;
        vertex.position = position.to_array();
    }
    cube
}

fn disconnected_cubes() -> MeshEditBuffers {
    let first = translated_cube(Vec3::new(-3.0, 0.0, 0.0));
    let second = translated_cube(Vec3::new(3.0, 0.0, 0.0));
    let offset = u32::try_from(first.vertices.len()).expect("small fixture");
    let mut vertices = first.vertices;
    vertices.extend(second.vertices);
    let mut indices = first.indices;
    indices.extend(second.indices.into_iter().map(|index| index + offset));
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn point_touching_cubes_with_separate_topology() -> MeshEditBuffers {
    let first = closed_cube();
    let second = translated_cube(Vec3::splat(2.0));
    let offset = u32::try_from(first.vertices.len()).expect("small fixture");
    let mut vertices = first.vertices;
    vertices.extend(second.vertices);
    let mut indices = first.indices;
    indices.extend(second.indices.into_iter().map(|index| index + offset));
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn closed_tetrahedra_sharing_only_one_vertex() -> MeshEditBuffers {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
    ];
    let first = [0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3];
    let second = [0, 5, 4, 0, 4, 6, 0, 6, 5, 4, 5, 6];
    MeshEditBuffers {
        vertices: positions.into_iter().map(EditVertex::at).collect(),
        indices: first.into_iter().chain(second).collect(),
        topology: MeshTopology::TriangleMesh,
    }
}

fn closed_torus(major_segments: usize, minor_segments: usize) -> MeshEditBuffers {
    let mut vertices = Vec::with_capacity(major_segments * minor_segments);
    let major_radius = 3.0_f32;
    let minor_radius = 1.0_f32;
    for major in 0..major_segments {
        let u = std::f32::consts::TAU * major as f32 / major_segments as f32;
        for minor in 0..minor_segments {
            let v = std::f32::consts::TAU * minor as f32 / minor_segments as f32;
            let radial = major_radius + minor_radius * v.cos();
            vertices.push(EditVertex::at([
                radial * u.cos(),
                radial * u.sin(),
                minor_radius * v.sin(),
            ]));
        }
    }
    let vertex = |major: usize, minor: usize| -> u32 {
        u32::try_from((major % major_segments) * minor_segments + (minor % minor_segments))
            .expect("small fixture")
    };
    let mut indices = Vec::with_capacity(major_segments * minor_segments * 6);
    for major in 0..major_segments {
        for minor in 0..minor_segments {
            let a = vertex(major, minor);
            let b = vertex(major + 1, minor);
            let c = vertex(major + 1, minor + 1);
            let d = vertex(major, minor + 1);
            indices.extend([a, b, c, a, c, d]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

fn closed_u_prism() -> MeshEditBuffers {
    closed_voxel_union([
        (-2, -2, 0),
        (-2, -1, 0),
        (-2, 0, 0),
        (-1, -2, 0),
        (0, -2, 0),
        (1, -2, 0),
        (-1, 0, 0),
        (0, 0, 0),
        (1, 0, 0),
    ])
}

fn closed_l_prism() -> MeshEditBuffers {
    closed_voxel_union([
        (-1, 0, 0),
        (-1, 1, 0),
        (-1, 0, 1),
        (0, 0, 0),
        (0, 1, 0),
        (0, 0, 1),
    ])
}

fn closed_voxel_union<const N: usize>(cells: [(i32, i32, i32); N]) -> MeshEditBuffers {
    use std::collections::{BTreeMap, BTreeSet};

    let occupied: BTreeSet<(i32, i32, i32)> = cells.into_iter().collect();
    let directions = [
        ((-1, 0, 0), 0),
        ((1, 0, 0), 1),
        ((0, -1, 0), 2),
        ((0, 1, 0), 3),
        ((0, 0, -1), 4),
        ((0, 0, 1), 5),
    ];
    let mut vertex_ids: BTreeMap<(i32, i32, i32), u32> = BTreeMap::new();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let face_corners = |cell: (i32, i32, i32), side: usize| {
        let (x, y, z) = cell;
        match side {
            0 => [(x, y, z), (x, y, z + 1), (x, y + 1, z + 1), (x, y + 1, z)],
            1 => [
                (x + 1, y, z),
                (x + 1, y + 1, z),
                (x + 1, y + 1, z + 1),
                (x + 1, y, z + 1),
            ],
            2 => [(x, y, z), (x + 1, y, z), (x + 1, y, z + 1), (x, y, z + 1)],
            3 => [
                (x, y + 1, z),
                (x, y + 1, z + 1),
                (x + 1, y + 1, z + 1),
                (x + 1, y + 1, z),
            ],
            4 => [(x, y, z), (x, y + 1, z), (x + 1, y + 1, z), (x + 1, y, z)],
            _ => [
                (x, y, z + 1),
                (x + 1, y, z + 1),
                (x + 1, y + 1, z + 1),
                (x, y + 1, z + 1),
            ],
        }
    };
    for &cell in &occupied {
        for &(direction, side) in &directions {
            let neighbor = (
                cell.0 + direction.0,
                cell.1 + direction.1,
                cell.2 + direction.2,
            );
            if occupied.contains(&neighbor) {
                continue;
            }
            let corners = face_corners(cell, side);
            let mut face = [0_u32; 4];
            for (slot, coordinate) in corners.into_iter().enumerate() {
                face[slot] = if let Some(&existing) = vertex_ids.get(&coordinate) {
                    existing
                } else {
                    let index = u32::try_from(vertices.len()).expect("small fixture");
                    vertices.push(EditVertex::at([
                        coordinate.0 as f32,
                        coordinate.1 as f32,
                        coordinate.2 as f32,
                    ]));
                    vertex_ids.insert(coordinate, index);
                    index
                };
            }
            indices.extend([face[0], face[1], face[2], face[0], face[2], face[3]]);
        }
    }
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

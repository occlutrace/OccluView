use super::*;

fn v(position: [f32; 3]) -> EditVertex {
    EditVertex::at(position)
}

fn mesh_with_two_triangles() -> MeshEditBuffers {
    MeshEditBuffers {
        vertices: vec![
            EditVertex {
                color: [200, 10, 10, 255],
                uv: [0.0, 0.0],
                normal: [9.0, 9.0, 9.0],
                ..v([0.0, 0.0, 0.0])
            },
            EditVertex {
                color: [10, 200, 10, 255],
                uv: [1.0, 0.0],
                normal: [9.0, 9.0, 9.0],
                ..v([1.0, 0.0, 0.0])
            },
            EditVertex {
                color: [10, 10, 200, 255],
                uv: [0.0, 1.0],
                normal: [9.0, 9.0, 9.0],
                ..v([0.0, 1.0, 0.0])
            },
            EditVertex {
                color: [250, 240, 20, 255],
                uv: [1.0, 1.0],
                normal: [9.0, 9.0, 9.0],
                ..v([1.0, 1.0, 0.0])
            },
        ],
        indices: vec![0, 1, 2, 1, 3, 2],
        topology: MeshTopology::TriangleMesh,
    }
}

fn mesh_with_duplicate_positions_and_distinct_attributes() -> MeshEditBuffers {
    MeshEditBuffers {
        vertices: vec![
            EditVertex {
                color: [11, 22, 33, 44],
                uv: [0.0, 0.0],
                ..v([0.0, 0.0, 0.0])
            },
            EditVertex {
                color: [55, 66, 77, 88],
                uv: [1.0, 0.0],
                ..v([1.0, 0.0, 0.0])
            },
            EditVertex {
                color: [99, 100, 101, 102],
                uv: [0.0, 1.0],
                ..v([0.0, 1.0, 0.0])
            },
            EditVertex {
                color: [201, 202, 203, 204],
                uv: [0.9, 0.1],
                ..v([0.0, 0.0, 0.0])
            },
            EditVertex {
                color: [205, 206, 207, 208],
                uv: [0.2, 0.8],
                ..v([1.0, 0.0, 0.0])
            },
            EditVertex {
                color: [209, 210, 211, 212],
                uv: [0.3, 0.7],
                ..v([0.0, 1.0, 0.0])
            },
        ],
        indices: vec![0, 1, 2, 3, 4, 5],
        topology: MeshTopology::TriangleMesh,
    }
}

fn mesh_with_three_islands() -> MeshEditBuffers {
    MeshEditBuffers {
        vertices: vec![
            EditVertex {
                color: [10, 20, 30, 255],
                uv: [0.0, 0.0],
                ..v([0.0, 0.0, 0.0])
            },
            EditVertex {
                color: [11, 21, 31, 255],
                uv: [1.0, 0.0],
                ..v([1.0, 0.0, 0.0])
            },
            EditVertex {
                color: [12, 22, 32, 255],
                uv: [0.0, 1.0],
                ..v([0.0, 1.0, 0.0])
            },
            EditVertex {
                color: [13, 23, 33, 255],
                uv: [0.0, 0.0],
                ..v([2.0, 0.0, 0.0])
            },
            EditVertex {
                color: [14, 24, 34, 255],
                uv: [1.0, 0.0],
                ..v([3.0, 0.0, 0.0])
            },
            EditVertex {
                color: [15, 25, 35, 255],
                uv: [0.0, 1.0],
                ..v([2.0, 1.0, 0.0])
            },
            EditVertex {
                color: [16, 26, 36, 255],
                uv: [0.0, 0.0],
                ..v([0.0, 0.0, 1.0])
            },
            EditVertex {
                color: [17, 27, 37, 255],
                uv: [1.0, 0.0],
                ..v([1.0, 0.0, 1.0])
            },
            EditVertex {
                color: [18, 28, 38, 255],
                uv: [0.0, 1.0],
                ..v([0.0, 1.0, 1.0])
            },
        ],
        indices: vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
        topology: MeshTopology::TriangleMesh,
    }
}

fn selected_faces(values: &[bool]) -> FaceSelection {
    FaceSelection::new(values.to_vec())
}

fn bowl_mesh() -> MeshEditBuffers {
    let mut vertices = vec![
        EditVertex::at([1.0, 0.0, 0.0]),
        EditVertex::at([0.0, 1.0, 0.0]),
        EditVertex::at([-1.0, 0.0, 0.0]),
        EditVertex::at([0.0, -1.0, 0.0]),
        EditVertex::at([0.0, 0.0, 1.0]),
    ];
    let indices = vec![
        4, 0, 1, //
        4, 1, 2, //
        4, 2, 3, //
        4, 3, 0, //
    ];
    recompute_all_normals(&mut vertices, &indices).expect("seed normals");
    MeshEditBuffers {
        vertices,
        indices,
        topology: MeshTopology::TriangleMesh,
    }
}

mod attributes;
mod holes;
mod operations;

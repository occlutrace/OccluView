use crate::bridge_split::cap::cap_open_part;
use crate::{EditVertex, MeshEditBuffers, MeshTopology};
use glam::DVec3;

#[test]
fn thin_but_nonzero_cut_cap_remains_manufacturable() {
    let mesh = MeshEditBuffers {
        vertices: vec![
            EditVertex::at([0.0, 0.0, 0.0]),
            EditVertex::at([1.0, 0.0, 0.0]),
            EditVertex::at([1.0, 1.0e-9, 0.0]),
            EditVertex::at([0.0, 0.0, 1.0]),
        ],
        indices: vec![0, 1, 3, 1, 2, 3, 2, 0, 3],
        topology: MeshTopology::TriangleMesh,
    };

    let (capped, loops) = cap_open_part(mesh, &[[0, 1], [1, 2], [2, 0]], DVec3::NEG_Z)
        .expect("a nonzero cut cap must not be rejected only for being thin");

    assert_eq!(loops, 1);
    assert_eq!(capped.triangle_count(), 4);
}

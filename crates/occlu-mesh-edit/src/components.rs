use super::{
    topology::weld_soup_topology, validate_face_edit_buffers,
    validate_selection_against_triangle_count, FaceSelection, MeshEditBuffers, MeshEditError,
};

/// Split the selected triangles into connected components.
///
/// Connectivity is derived from shared undirected EDGES over the mesh's TRUE
/// topology: STL and other soup formats store every triangle's corners as
/// independent vertices, so the buffers are welded back to shared topology first
/// (exact position/color/uv bits — see `weld_soup_topology`). Without this a
/// soup model reads as one component per triangle — the "317k confetti parts"
/// Separate failure. Triangles touching at only a single shared vertex remain
/// separate components. Components are returned in deterministic order by their
/// lowest triangle index in the source mesh, and each component's member list is
/// ascending.
///
/// The weld only remaps vertex ids; triangle order and count are preserved, so
/// the returned member lists are indices into the CALLER's original mesh and
/// stay valid for it. The weld is a no-op (no allocation of new buffers) on an
/// already-welded mesh.
///
/// Each component is a list of its member triangle indices, NOT a full-length
/// mask: a fragmented selection can yield many components, and a per-component
/// `vec![bool; triangle_count]` would cost O(components × `triangle_count`)
/// memory. The consumer materializes one mask at a time.
///
/// Grouping runs in near-linear time: each selected triangle's three edges are
/// emitted once, sorted, and triangles sharing an edge key are merged with a
/// union-find. This replaced an edge→triangle `HashMap<_, Vec<_>>` that spent
/// most of its time hashing and heap-allocating a `Vec` per edge.
///
/// # Errors
/// Returns typed validation errors for unsupported point clouds, malformed
/// triangle data, or selection length mismatches.
pub fn selected_connected_components(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
) -> Result<Vec<Vec<usize>>, MeshEditError> {
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    validate_selection_against_triangle_count(mesh.triangle_count(), selection)?;

    let triangle_count = mesh.triangle_count();
    if triangle_count == 0 || selection.selected_count() == 0 {
        return Ok(Vec::new());
    }

    // Recover shared topology before connectivity so an STL soup does not read as
    // one component per triangle. Triangle order/count are preserved, so the
    // dense selection indices below are unaffected and the returned member lists
    // stay valid for the caller's original buffers. `None` => already welded.
    let welded = weld_soup_topology(mesh)?;
    let indices: &[u32] = welded
        .as_ref()
        .map_or(mesh.indices.as_slice(), |w| &w.indices);

    // Dense-index the selected triangles so the union-find only sizes to the
    // selection, not the whole mesh.
    let dense: Vec<usize> = selection
        .as_slice()
        .iter()
        .enumerate()
        .filter_map(|(triangle, &selected)| selected.then_some(triangle))
        .collect();

    let mut union_find = UnionFind::new(dense.len());
    union_shared_edges(indices, &dense, &mut union_find)?;
    Ok(group_by_root(&dense, &mut union_find))
}

/// Emit every selected triangle's three undirected edges as a packed key, sort
/// them, and union triangles that share an edge key.
fn union_shared_edges(
    indices: &[u32],
    dense: &[usize],
    union_find: &mut UnionFind,
) -> Result<(), MeshEditError> {
    let mut edges: Vec<(u64, usize)> = Vec::with_capacity(dense.len() * 3);
    for (dense_id, &triangle) in dense.iter().enumerate() {
        let corners = indices.get(triangle * 3..triangle * 3 + 3).ok_or_else(|| {
            MeshEditError::MalformedMesh {
                reason: format!("triangle {triangle} is out of range for the index buffer"),
            }
        })?;
        for (a, b) in [
            (corners[0], corners[1]),
            (corners[1], corners[2]),
            (corners[2], corners[0]),
        ] {
            edges.push((packed_edge_key(a, b), dense_id));
        }
    }

    edges.sort_unstable();

    let mut run_start = 0;
    while run_start < edges.len() {
        let (key, anchor) = edges[run_start];
        let mut cursor = run_start + 1;
        while cursor < edges.len() && edges[cursor].0 == key {
            union_find.union(anchor, edges[cursor].1);
            cursor += 1;
        }
        run_start = cursor;
    }
    Ok(())
}

/// Bucket dense triangles by their union-find root. Because `dense` is ascending
/// and iterated in order, each component's member list comes out ascending and
/// components come out ordered by their lowest source-triangle index.
fn group_by_root(dense: &[usize], union_find: &mut UnionFind) -> Vec<Vec<usize>> {
    let mut root_slot = vec![usize::MAX; dense.len()];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for (dense_id, &triangle) in dense.iter().enumerate() {
        let root = union_find.find(dense_id);
        let slot = if root_slot[root] == usize::MAX {
            let slot = components.len();
            root_slot[root] = slot;
            components.push(Vec::new());
            slot
        } else {
            root_slot[root]
        };
        components[slot].push(triangle);
    }
    components
}

/// Pack an undirected edge (unordered index pair) into a sortable `u64`.
fn packed_edge_key(a: u32, b: u32) -> u64 {
    let (lo, hi) = (a.min(b), a.max(b));
    (u64::from(lo) << 32) | u64::from(hi)
}

/// Disjoint-set forest with path halving and union by size — near-linear
/// connectivity without recursion (kernel forbids panics/unsafe).
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
            size: vec![1; count],
        }
    }

    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            let grandparent = self.parent[self.parent[node]];
            self.parent[node] = grandparent;
            node = grandparent;
        }
        node
    }

    fn union(&mut self, a: usize, b: usize) {
        let (root_a, root_b) = (self.find(a), self.find(b));
        if root_a == root_b {
            return;
        }
        let (larger, smaller) = if self.size[root_a] >= self.size[root_b] {
            (root_a, root_b)
        } else {
            (root_b, root_a)
        };
        self.parent[smaller] = larger;
        self.size[larger] += self.size[smaller];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EditVertex, MeshTopology};

    fn grid(cols: usize, rows: usize) -> MeshEditBuffers {
        let stride = (cols + 1) as u32;
        let mut vertices = Vec::new();
        for y in 0..=rows {
            for x in 0..=cols {
                vertices.push(EditVertex::at([x as f32, y as f32, 0.0]));
            }
        }
        let mut indices = Vec::new();
        for y in 0..rows as u32 {
            for x in 0..cols as u32 {
                let a = y * stride + x;
                indices.extend_from_slice(&[
                    a,
                    a + 1,
                    a + stride,
                    a + 1,
                    a + stride + 1,
                    a + stride,
                ]);
            }
        }
        MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        }
    }

    fn mask(len: usize, predicate: impl Fn(usize) -> bool) -> FaceSelection {
        FaceSelection::new((0..len).map(predicate).collect())
    }

    #[test]
    fn fragmented_selection_yields_ascending_ordered_components() {
        // 4x4 grid, 32 triangles. Select even rows (0 and 2). Each selected row
        // is its own component because the odd row between breaks connectivity.
        let mesh = grid(4, 4);
        let tris_per_row = 8;
        let selection = mask(mesh.triangle_count(), |t| (t / tris_per_row) % 2 == 0);
        let components =
            selected_connected_components(&mesh, &selection).expect("components computed");

        assert_eq!(components.len(), 2, "two disconnected selected rows");
        // Deterministic order: component 0 owns the lowest triangle indices.
        assert_eq!(components[0], (0..8).collect::<Vec<_>>());
        assert_eq!(components[1], (16..24).collect::<Vec<_>>());
        // Every member is ascending and every selected triangle appears once.
        let mut flat: Vec<usize> = components.iter().flatten().copied().collect();
        flat.sort_unstable();
        assert_eq!(flat, (0..8).chain(16..24).collect::<Vec<_>>());
    }

    #[test]
    fn edge_connected_run_merges_into_one_component() {
        // Whole first row of a 5x1 grid is edge-connected -> single component.
        let mesh = grid(5, 1);
        let selection = mask(mesh.triangle_count(), |_| true);
        let components =
            selected_connected_components(&mesh, &selection).expect("components computed");
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0],
            (0..mesh.triangle_count()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn triangles_sharing_only_a_vertex_stay_separate() {
        let mesh = MeshEditBuffers {
            vertices: (0..5)
                .map(|i| EditVertex::at([i as f32, 0.0, 0.0]))
                .collect(),
            // Two triangles touching only at vertex 2.
            indices: vec![0, 1, 2, 2, 3, 4],
            topology: MeshTopology::TriangleMesh,
        };
        let components =
            selected_connected_components(&mesh, &mask(2, |_| true)).expect("computed");
        assert_eq!(components, vec![vec![0], vec![1]]);
    }

    #[test]
    fn empty_selection_returns_no_components() {
        let mesh = grid(2, 2);
        let components =
            selected_connected_components(&mesh, &mask(mesh.triangle_count(), |_| false))
                .expect("computed");
        assert!(components.is_empty());
    }

    /// Explode a welded mesh into STL-style soup: three fresh vertices per
    /// triangle corner, sequential indices, nothing shared. Byte-for-byte the
    /// topology a binary STL reader produces.
    fn explode_to_soup(mesh: &MeshEditBuffers) -> MeshEditBuffers {
        let mut vertices = Vec::with_capacity(mesh.indices.len());
        let mut indices = Vec::with_capacity(mesh.indices.len());
        for &vi in &mesh.indices {
            indices.push(vertices.len() as u32);
            vertices.push(mesh.vertices[vi as usize]);
        }
        MeshEditBuffers {
            vertices,
            indices,
            topology: MeshTopology::TriangleMesh,
        }
    }

    #[test]
    fn soup_selection_welds_to_one_component_not_confetti() {
        // The 317k-confetti bug: an edge-connected patch, once exploded to STL
        // soup, must NOT read as one component per triangle. It welds back to a
        // single connected island.
        let welded = grid(5, 5);
        let soup = explode_to_soup(&welded);
        assert_eq!(
            soup.vertices.len(),
            soup.indices.len(),
            "soup has no shared vertices"
        );
        let selection = mask(soup.triangle_count(), |_| true);
        let components =
            selected_connected_components(&soup, &selection).expect("components computed");
        assert_eq!(
            components.len(),
            1,
            "a connected soup patch is ONE island, never one-per-triangle"
        );
        assert_eq!(components[0].len(), soup.triangle_count());
    }

    #[test]
    fn soup_two_disjoint_walls_yield_two_components_in_order() {
        // The owner's through-selection case: two disjoint marked patches (an
        // outer and an inner wall) exploded to soup weld to exactly two islands,
        // ordered by lowest source-triangle index. 4x4 grid: even rows 0 and 2
        // are two disconnected strips (odd row 1 breaks connectivity).
        let welded = grid(4, 4);
        let soup = explode_to_soup(&welded);
        let tris_per_row = 8;
        let selection = mask(soup.triangle_count(), |t| (t / tris_per_row) % 2 == 0);
        let components =
            selected_connected_components(&soup, &selection).expect("components computed");
        assert_eq!(components.len(), 2, "two disjoint walls -> two islands");
        assert_eq!(components[0], (0..8).collect::<Vec<_>>());
        assert_eq!(components[1], (16..24).collect::<Vec<_>>());
    }

    #[test]
    fn mixed_soup_and_welded_input_recovers_true_topology() {
        // A partially welded mesh (some corners shared, some duplicated) still
        // welds to the true island count. Build a welded strip, then duplicate
        // one triangle's corners so the buffer is neither pure soup nor pure
        // welded — connectivity must still see one island.
        let welded = grid(4, 1);
        let mut mesh = welded.clone();
        // Duplicate the corners of triangle 0 as fresh vertices, repointing it.
        let base = mesh.vertices.len() as u32;
        let tri0 = [mesh.indices[0], mesh.indices[1], mesh.indices[2]];
        for &c in &tri0 {
            mesh.vertices.push(mesh.vertices[c as usize]);
        }
        mesh.indices[0] = base;
        mesh.indices[1] = base + 1;
        mesh.indices[2] = base + 2;
        let selection = mask(mesh.triangle_count(), |_| true);
        let components =
            selected_connected_components(&mesh, &selection).expect("components computed");
        assert_eq!(
            components.len(),
            1,
            "duplicated corners weld back; the strip stays one island"
        );
    }

    #[test]
    fn single_triangle_soup_island_is_a_valid_component() {
        // A tiny sliver island (one triangle) is still a valid, panic-free part.
        let mesh = MeshEditBuffers {
            vertices: vec![
                EditVertex::at([0.0, 0.0, 0.0]),
                EditVertex::at([1.0, 0.0, 0.0]),
                EditVertex::at([0.0, 1.0, 0.0]),
            ],
            indices: vec![0, 1, 2],
            topology: MeshTopology::TriangleMesh,
        };
        let components =
            selected_connected_components(&mesh, &mask(1, |_| true)).expect("computed");
        assert_eq!(components, vec![vec![0]]);
    }

    #[test]
    fn duplicate_and_nan_vertices_do_not_panic() {
        // Two identical (duplicate) triangles plus a triangle carrying a NaN
        // coordinate must not panic: `to_bits` keys NaN deterministically, and
        // the weld/union-find never indexes out of range.
        let nan = f32::NAN;
        let mesh = MeshEditBuffers {
            vertices: vec![
                EditVertex::at([0.0, 0.0, 0.0]),
                EditVertex::at([1.0, 0.0, 0.0]),
                EditVertex::at([0.0, 1.0, 0.0]),
                EditVertex::at([0.0, 0.0, 0.0]),
                EditVertex::at([1.0, 0.0, 0.0]),
                EditVertex::at([0.0, 1.0, 0.0]),
                EditVertex::at([nan, 0.0, 0.0]),
                EditVertex::at([nan, 1.0, 0.0]),
                EditVertex::at([nan, 0.0, 1.0]),
            ],
            indices: vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            topology: MeshTopology::TriangleMesh,
        };
        let components =
            selected_connected_components(&mesh, &mask(3, |_| true)).expect("must not panic");
        // The two identical triangles weld to a shared, degenerate face (all
        // three corners collapse to the same ids) which shares no edge with the
        // isolated NaN triangle. The result is well-formed and panic-free.
        assert!(!components.is_empty());
        let mut flat: Vec<usize> = components.iter().flatten().copied().collect();
        flat.sort_unstable();
        assert_eq!(flat, vec![0, 1, 2]);
    }
}

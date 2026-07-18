//! Incremental Lawson machinery for cap refinement: a cap triangulation with a
//! PERSISTENT edge→owner map, a deterministic suspect-edge worklist, and the
//! conforming 2:4 edge bisection.
//!
//! Why not sweep: the previous scheme re-tested EVERY cap edge up to 64 times
//! after every bisection pass, rebuilding the edge map from scratch each sweep.
//! On a ~1000-edge rim (a routine lasso cut, issue #9) that made refinement
//! cost ~5 s of the total fill. After a split or a flip only the edges of the
//! rewritten quads can newly violate the Delaunay criterion, so a worklist
//! seeded by exactly those edges does the same repair in near-linear time.
//! The worklist is a `BTreeSet` and candidate edges are visited in sorted
//! order, so the output stays bit-deterministic run to run.

use std::collections::{BTreeSet, HashMap};

use glam::Vec2;

use super::cap_delaunay::{
    apex_of, circumcircle_verdict, replace_edge, signed_area, CircleVerdict,
};

/// Safety valve on total flips per repair call, as a multiple of the triangle
/// count. Lawson terminates on planar inputs (inside-flips lexicographically
/// increase the angle vector; tie-flips strictly shorten the diagonal), so the
/// budget only guards numerically degenerate inputs from cycling forever.
const FLIP_BUDGET_PER_TRIANGLE: usize = 32;

/// Undirected edge key.
#[inline]
fn edge_key(u: usize, v: usize) -> (usize, usize) {
    (u.min(v), u.max(v))
}

/// A cap triangulation whose edge→owner map stays live across bisections and
/// flips. Triangle SLOTS are stable: a flip rewrites the two owner slots in
/// place, a bisection rewrites both owners and pushes two children.
pub(super) struct CapMesh {
    triangles: Vec<[usize; 3]>,
    owners: HashMap<(usize, usize), Vec<usize>>,
}

impl CapMesh {
    pub(super) fn new(triangles: Vec<[usize; 3]>) -> Self {
        let mut owners: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (slot, &[a, b, c]) in triangles.iter().enumerate() {
            for (u, v) in [(a, b), (b, c), (c, a)] {
                owners.entry(edge_key(u, v)).or_default().push(slot);
            }
        }
        Self { triangles, owners }
    }

    #[cfg(test)]
    pub(super) fn triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }

    pub(super) fn into_triangles(self) -> Vec<[usize; 3]> {
        self.triangles
    }

    /// Every edge currently in the triangulation, ascending. Snapshot for a
    /// deterministic bisection pass; entries may be invalidated by splits made
    /// later in the same pass, so callers re-check membership via [`Self::owner_pair`].
    pub(super) fn edges_sorted(&self) -> Vec<(usize, usize)> {
        let mut edges: Vec<(usize, usize)> = self.owners.keys().copied().collect();
        edges.sort_unstable();
        edges
    }

    /// The two owner slots of an interior edge, or `None` for rim edges (one
    /// owner), stale keys (zero), and non-manifold noise (three or more).
    pub(super) fn owner_pair(&self, edge: (usize, usize)) -> Option<[usize; 2]> {
        match self.owners.get(&edge).map(Vec::as_slice) {
            Some(&[t1, t2]) => Some([t1, t2]),
            _ => None,
        }
    }

    fn remove_owner(&mut self, edge: (usize, usize), slot: usize) {
        if let Some(list) = self.owners.get_mut(&edge) {
            list.retain(|&owner| owner != slot);
            if list.is_empty() {
                self.owners.remove(&edge);
            }
        }
    }

    fn add_owner(&mut self, edge: (usize, usize), slot: usize) {
        self.owners.entry(edge).or_default().push(slot);
    }

    /// Rewrite the triangle in `slot`, diffing its edges into the owner map.
    fn set_triangle(&mut self, slot: usize, next: [usize; 3]) {
        let previous = self.triangles[slot];
        for (u, v) in [
            (previous[0], previous[1]),
            (previous[1], previous[2]),
            (previous[2], previous[0]),
        ] {
            self.remove_owner(edge_key(u, v), slot);
        }
        for (u, v) in [(next[0], next[1]), (next[1], next[2]), (next[2], next[0])] {
            self.add_owner(edge_key(u, v), slot);
        }
        self.triangles[slot] = next;
    }

    /// Append a new triangle and register its edges.
    fn push_triangle(&mut self, triangle: [usize; 3]) {
        let slot = self.triangles.len();
        self.triangles.push(triangle);
        for (u, v) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            self.add_owner(edge_key(u, v), slot);
        }
    }

    /// Conforming 2:4 bisection of interior edge `(u, v)` at the already-pushed
    /// planar vertex `midpoint_index`. Both owner triangles are split with
    /// winding preserved; the four edges around each rewritten quad plus the
    /// new spokes are pushed into `suspects` for the next Lawson repair.
    ///
    /// The caller has verified via [`Self::owner_pair`] that the edge is
    /// interior, so this cannot fail; rim edges (one owner) are never split.
    pub(super) fn bisect(
        &mut self,
        edge: (usize, usize),
        owners: [usize; 2],
        midpoint_index: usize,
        suspects: &mut BTreeSet<(usize, usize)>,
    ) {
        let (u, v) = edge;
        for slot in owners {
            let parent = self.triangles[slot];
            let mut keeps_u = parent;
            for corner in &mut keeps_u {
                if *corner == v {
                    *corner = midpoint_index;
                }
            }
            let mut keeps_v = parent;
            for corner in &mut keeps_v {
                if *corner == u {
                    *corner = midpoint_index;
                }
            }
            self.set_triangle(slot, keeps_u);
            self.push_triangle(keeps_v);
            // The parent's outer edges and the new spoke all border a rewritten
            // triangle: exactly the edges that can newly violate Delaunay.
            if let Some(apex) = apex_of(parent, u, v) {
                suspects.extend([
                    edge_key(u, apex),
                    edge_key(v, apex),
                    edge_key(midpoint_index, apex),
                ]);
            }
        }
        suspects.extend([edge_key(u, midpoint_index), edge_key(v, midpoint_index)]);
    }

    /// Lawson repair from a seed set: pop suspect edges in ascending order,
    /// flip any interior edge violating the Delaunay criterion (with the
    /// cocircular shorter-diagonal tie-break), and re-seed the four quad
    /// boundary edges of every flip. Convex-quad and new-diagonal guards are
    /// identical to the retired sweep implementation.
    pub(super) fn lawson(&mut self, uv: &[Vec2], mut suspects: BTreeSet<(usize, usize)>) {
        let mut budget = self
            .triangles
            .len()
            .saturating_mul(FLIP_BUDGET_PER_TRIANGLE)
            .max(1024);
        while let Some(edge) = suspects.pop_first() {
            if budget == 0 {
                break;
            }
            let (u, v) = edge;
            let Some([t1, t2]) = self.owner_pair(edge) else {
                continue;
            };
            let Some((apex1, apex2)) =
                (apex_of(self.triangles[t1], u, v)).zip(apex_of(self.triangles[t2], u, v))
            else {
                continue;
            };
            // The flipped diagonal must be a NEW edge, or the cap goes
            // non-manifold.
            let diagonal = edge_key(apex1, apex2);
            if self.owners.contains_key(&diagonal) {
                continue;
            }
            // Convex-quad test: both candidates keep the original winding sign.
            let sign = signed_area(uv, self.triangles[t1]).signum();
            let candidate1 = replace_edge(self.triangles[t1], (u, v), apex2);
            let candidate2 = replace_edge(self.triangles[t2], (v, u), apex1);
            let area1 = signed_area(uv, candidate1);
            let area2 = signed_area(uv, candidate2);
            if area1 * sign <= f32::EPSILON || area2 * sign <= f32::EPSILON {
                continue;
            }
            let verdict = circumcircle_verdict(uv[u], uv[v], uv[apex1], uv[apex2]);
            let flip = match verdict {
                CircleVerdict::Inside => true,
                CircleVerdict::Tie => {
                    let old_len = uv[u].distance_squared(uv[v]);
                    let new_len = uv[apex1].distance_squared(uv[apex2]);
                    new_len < old_len * 0.999
                }
                CircleVerdict::Outside => false,
            };
            if flip {
                self.set_triangle(t1, candidate1);
                self.set_triangle(t2, candidate2);
                budget -= 1;
                suspects.extend([
                    edge_key(u, apex1),
                    edge_key(u, apex2),
                    edge_key(v, apex1),
                    edge_key(v, apex2),
                ]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad whose fourth corner sits strictly inside the circumcircle of the
    /// first triangle: Lawson must flip the shared diagonal.
    #[test]
    fn lawson_flips_a_non_delaunay_diagonal() {
        // Circumcircle of (0,0),(4,0),(4,3) has center (2,1.5), r=2.5; vertex
        // (0.5,1.0) lies strictly inside it, so diagonal (0,2) is not Delaunay.
        let uv = [
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 3.0),
            Vec2::new(0.5, 1.0),
        ];
        let mut cap = CapMesh::new(vec![[0, 1, 2], [0, 2, 3]]);
        let seeds: BTreeSet<(usize, usize)> = cap.edges_sorted().into_iter().collect();
        cap.lawson(&uv, seeds);
        let has_flipped_diagonal = cap
            .triangles()
            .iter()
            .any(|t| t.contains(&1) && t.contains(&3));
        assert!(
            has_flipped_diagonal,
            "expected the flip to diagonal (1,3), got {:?}",
            cap.triangles()
        );
    }

    /// Bisection keeps the owner map consistent: every edge of every triangle
    /// owns the right slots, and the parent edge is gone.
    #[test]
    fn bisect_keeps_owner_map_consistent() {
        let mut cap = CapMesh::new(vec![[0, 1, 2], [0, 2, 3]]);
        let mut suspects = BTreeSet::new();
        let owners = cap.owner_pair((0, 2)).expect("interior edge");
        cap.bisect((0, 2), owners, 4, &mut suspects);
        assert_eq!(cap.triangles().len(), 4);
        assert!(cap.owner_pair((0, 2)).is_none(), "split edge must be gone");
        // Rebuild from scratch and compare owner maps.
        let rebuilt = CapMesh::new(cap.triangles().to_vec());
        let mut expected: Vec<_> = rebuilt.owners.iter().collect();
        let mut actual: Vec<_> = cap.owners.iter().collect();
        expected.sort();
        actual.sort();
        assert_eq!(
            actual, expected,
            "incremental owner map must equal a fresh rebuild"
        );
        assert!(suspects.contains(&(0, 4)) && suspects.contains(&(2, 4)));
    }

    /// The repair is deterministic: two runs over the same input produce the
    /// same triangulation.
    #[test]
    fn lawson_is_deterministic() {
        let uv: Vec<Vec2> = (0..12)
            .map(|i| {
                let angle = std::f32::consts::TAU * (i as f32) / 12.0;
                Vec2::new(angle.cos(), angle.sin())
            })
            .chain([Vec2::new(0.1, 0.05)])
            .collect();
        let fan: Vec<[usize; 3]> = (1..11).map(|i| [0, i, i + 1]).collect();
        let run = || {
            let mut cap = CapMesh::new(fan.clone());
            let seeds: BTreeSet<(usize, usize)> = cap.edges_sorted().into_iter().collect();
            cap.lawson(&uv, seeds);
            cap.into_triangles()
        };
        assert_eq!(run(), run());
    }
}

//! Local topology operators and their shared manifold predicates and budgets.
//! The live cycle validates every edit before committing it and updates only
//! affected adjacency rows. Split and collapse use the same target spacing.

use super::*;
use crate::RemeshPolicy;
/// Minimum min-triangle-quality gain for a flip to commit. Below this the
/// diagonal is not bad enough to spend a revision on.
const FLIP_QUALITY_EPSILON: f64 = 1e-6;

/// Directed boundary edges of a small face set, sorted. Opposite interior
/// edges cancel; the remaining edges still meet untouched faces. `None` when
/// an edge is shared by more than two faces or twice in one direction.
fn directed_raw_boundary(raw: &[[u32; 3]]) -> Option<Vec<(u32, u32)>> {
    let mut edges: Vec<((u32, u32), (u32, u32))> = Vec::with_capacity(raw.len() * 3);
    for &[a, b, c] in raw {
        for (x, y) in [(a, b), (b, c), (c, a)] {
            edges.push(((x.min(y), x.max(y)), (x, y)));
        }
    }
    edges.sort_unstable();
    let mut boundary = Vec::with_capacity(edges.len());
    let mut run = 0;
    while run < edges.len() {
        let key = edges[run].0;
        let mut end = run + 1;
        while end < edges.len() && edges[end].0 == key {
            end += 1;
        }
        match end - run {
            1 => boundary.push(edges[run].1),
            2 => {
                let ((a, b), (c, d)) = (edges[run].1, edges[run + 1].1);
                if a != d || b != c {
                    return None;
                }
            }
            _ => return None,
        }
        run = end;
    }
    boundary.sort_unstable();
    Some(boundary)
}

/// A local remesh must keep the old directed boundary exactly.
fn replacement_preserves_raw_boundary(before: &[[u32; 3]], after: &[[u32; 3]]) -> bool {
    matches!(
        (directed_raw_boundary(before), directed_raw_boundary(after)),
        (Some(old), Some(new)) if old == new
    )
}

/// Link edges a vertex star holds without leaving the stack.
const LINK_INLINE: usize = 32;

/// Exact link predicate for one vertex: its undirected link edges must form a
/// single connected 1-manifold (a cycle for an interior vertex, a path for a
/// boundary vertex), degree at most 2, with 0 or 2 endpoints, and no duplicate
/// link edge. This is stronger than "common neighbours == opposite corners":
/// it also rejects a shared link edge (a tunnel) and a disconnected fan.
///
/// A star is a handful of edges, and this runs for every vertex of every
/// candidate merge and flip, so it works on sorted stack arrays: no map, no
/// set, no allocation for an ordinary star.
fn union_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn link_edges_are_one_fan(edges: &[(u32, u32)]) -> bool {
    let count = edges.len();
    if count == 0 {
        return true;
    }
    let mut inline_edges = [(0u32, 0u32); LINK_INLINE];
    let mut inline_ends = [0u32; LINK_INLINE * 2];
    let mut inline_parent = [0usize; LINK_INLINE * 2];
    let mut heap_edges: Vec<(u32, u32)>;
    let mut heap_ends: Vec<u32>;
    let mut heap_parent: Vec<usize>;
    let (canonical, ends, parent): (&mut [(u32, u32)], &mut [u32], &mut [usize]) =
        if count <= LINK_INLINE {
            (
                &mut inline_edges[..count],
                &mut inline_ends[..count * 2],
                &mut inline_parent[..count * 2],
            )
        } else {
            heap_edges = vec![(0, 0); count];
            heap_ends = vec![0; count * 2];
            heap_parent = vec![0; count * 2];
            (&mut heap_edges, &mut heap_ends, &mut heap_parent)
        };
    for (slot, &(a, b)) in canonical.iter_mut().zip(edges) {
        *slot = if a < b { (a, b) } else { (b, a) };
    }
    canonical.sort_unstable();
    if canonical.windows(2).any(|pair| pair[0] == pair[1]) {
        return false;
    }
    for (index, &(a, b)) in canonical.iter().enumerate() {
        ends[index * 2] = a;
        ends[index * 2 + 1] = b;
    }
    ends.sort_unstable();
    // Degree is the run length of a vertex in the sorted endpoint list.
    let mut vertices = 0;
    let mut endpoints = 0;
    let mut run = 0;
    while run < ends.len() {
        let mut end = run + 1;
        while end < ends.len() && ends[end] == ends[run] {
            end += 1;
        }
        match end - run {
            1 => endpoints += 1,
            2 => {}
            _ => return false,
        }
        ends[vertices] = ends[run];
        vertices += 1;
        run = end;
    }
    if endpoints != 0 && endpoints != 2 {
        return false;
    }
    // One component: union the ends of every edge over the distinct vertices.
    let unique = &ends[..vertices];
    let parent = &mut parent[..vertices];
    for (index, slot) in parent.iter_mut().enumerate() {
        *slot = index;
    }
    for &(a, b) in canonical.iter() {
        let (Ok(x), Ok(y)) = (unique.binary_search(&a), unique.binary_search(&b)) else {
            return false;
        };
        let (x, y) = (union_root(parent, x), union_root(parent, y));
        if x != y {
            parent[x] = y;
        }
    }
    let first = union_root(parent, 0);
    (1..vertices).all(|node| union_root(parent, node) == first)
}

// The quality law has one definition in the crate root; this alias keeps the
// call sites here unchanged.
use crate::triangle_quality_3d as tri_quality;

mod collapse;
mod flip;
mod sizing;
pub(in crate::sculpt_session) use sizing::input_spacing_mm;
mod local_surface;
pub(super) use local_surface::LocalSurface;

impl SculptSession {
    /// An open fan has fewer incident faces than neighbors.
    pub(super) fn group_is_boundary(&self, group: u32) -> bool {
        self.topology.incident_triangles(group).len() < self.topology.neighbors(group).len()
    }

    /// Groups that still hold faces: every slot minted so far, less the ones
    /// a merge retired.
    pub(super) fn live_group_count(&self) -> u32 {
        (self.topology.group_count() as u32).saturating_sub(self.retired_groups)
    }

    /// Whether a group still holds faces after the session's edits. A retired
    /// group is gone, not merely hidden, and every loop that walks live
    /// geometry asks this one question of it. Shared with the Smooth selection,
    /// which must not hold a retired group as a Dirichlet anchor.
    pub(super) fn group_is_live(&self, group: u32) -> bool {
        !self
            .group_retired
            .get(group as usize)
            .copied()
            .unwrap_or(true)
    }

    /// Whether a group is a live, editable part of the surface: still holding
    /// faces. Retired slots are excluded because they no longer carry geometry.
    pub(super) fn group_is_editable(&self, group: u32) -> bool {
        self.group_is_live(group)
    }

    /// The exact vertex-link law: the live incident faces of `v` must form a
    /// single cycle (interior) or a single path (boundary). `slots` is the
    /// candidate face set, `dying` faces are removed, and `substitute` maps a
    /// group to its post-operation identity. This is the predicate Apply
    /// enforces, evaluated on a virtual result before any commit.
    fn link_is_manifold_after(
        &self,
        v: u32,
        slots: &[u32],
        dying: &[u32],
        substitute: &[(u32, u32)],
    ) -> bool {
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for &slot in slots {
            if dying.contains(&slot) {
                continue;
            }
            let Some(corners) = self.topology.triangle(slot) else {
                continue;
            };
            let mapped: [u32; 3] = std::array::from_fn(|i| {
                let mut group = corners[i];
                for &(from, to) in substitute {
                    if group == from {
                        group = to;
                    }
                }
                group
            });
            if !mapped.contains(&v) {
                continue;
            }
            let others: Vec<u32> = mapped.iter().copied().filter(|&g| g != v).collect();
            if others.len() != 2 || others[0] == others[1] {
                return false;
            }
            edges.push((others[0], others[1]));
        }
        // An unreferenced vertex has an empty star, which is not a pinch, so
        // an empty link is acceptable here.
        if edges.is_empty() {
            return true;
        }
        if !link_edges_are_one_fan(&edges) {
            return false;
        }
        true
    }

    /// Virtual result of collapsing `removed` into `survivor`: every affected
    /// vertex's link must stay one cycle/path. Rejects shared link edges, a
    /// tunnel, a boundary pinch and a disconnected fan that the vertex-only
    /// common-neighbour test misses.
    fn collapse_result_is_manifold(&self, a: u32, b: u32) -> bool {
        let mut incident = self.topology.edge_triangles(a, b).to_vec();
        incident.extend_from_slice(&self.topology.edge_triangles(b, a));
        incident.sort_unstable();
        incident.dedup();
        if incident.len() != 2 {
            return false;
        }
        let (survivor, removed) = if a < b { (a, b) } else { (b, a) };
        let mut vertices: Vec<u32> = vec![survivor];
        for &group in &[a, b] {
            for &neighbor in self.topology.neighbors(group) {
                if neighbor != removed {
                    vertices.push(neighbor);
                }
            }
        }
        vertices.sort_unstable();
        vertices.dedup();
        let substitute = [(removed, survivor)];
        for &v in &vertices {
            // A vertex's link is defined by all its live faces, so the slots
            // must be its own incident set, not a union around the edge.
            let mut slots: Vec<u32> = self.topology.incident_triangles(v).to_vec();
            if v == survivor {
                slots.extend_from_slice(self.topology.incident_triangles(removed));
            }
            slots.sort_unstable();
            slots.dedup();
            if !self.link_is_manifold_after(v, &slots, &incident, &substitute) {
                return false;
            }
        }
        true
    }

    /// Virtual result of a flip `(a,b) -> (c,d)`: links of a,b,c,d must stay
    /// single fans. The new diagonal must not already exist.
    // the candidate quad and its two faces are one link question.
    #[allow(clippy::too_many_arguments)]
    fn flip_result_is_manifold(&self, a: u32, b: u32, c: u32, d: u32, t0: u32, t1: u32) -> bool {
        if !self.topology.edge_triangles(c, d).is_empty()
            || !self.topology.edge_triangles(d, c).is_empty()
        {
            return false;
        }
        let new_faces = [[c, d, b], [d, c, a]];
        for &v in &[a, b, c, d] {
            let mut edges: Vec<(u32, u32)> = Vec::new();
            for &slot in self.topology.incident_triangles(v) {
                if slot == t0 || slot == t1 {
                    continue;
                }
                let Some(corners) = self.topology.triangle(slot) else {
                    continue;
                };
                if !corners.contains(&v) {
                    continue;
                }
                let others: Vec<u32> = corners.iter().copied().filter(|&g| g != v).collect();
                if others.len() != 2 {
                    return false;
                }
                edges.push((others[0], others[1]));
            }
            for face in &new_faces {
                if face.contains(&v) {
                    let others: Vec<u32> = face.iter().copied().filter(|&g| g != v).collect();
                    if others.len() != 2 || others[0] == others[1] {
                        return false;
                    }
                    edges.push((others[0], others[1]));
                }
            }
            if !link_edges_are_one_fan(&edges) {
                return false;
            }
        }
        true
    }

    /// Shortest footprint edges, selected in linear time before sorting the cap.
    fn footprint_edges(&mut self, region: &[SurfacePoint], cap: usize) -> Vec<(u32, u32)> {
        let inside = self.mark_region_groups(region);
        let mut edges: Vec<(f64, (u32, u32))> = Vec::new();
        for point in region {
            let group = point.group;
            let here = self.group_v(group);
            for &neighbor in self.topology.neighbors(group) {
                // Each edge once: from its lower end inside the footprint, or
                // from its only end inside it.
                if neighbor < group && self.group_stamp[neighbor as usize] == inside {
                    continue;
                }
                let edge = if group < neighbor {
                    (group, neighbor)
                } else {
                    (neighbor, group)
                };
                edges.push(((self.group_v(neighbor) - here).length(), edge));
            }
        }
        // Shortest first; `total_cmp` keeps a NaN from making the order
        // depend on the sort implementation.
        let order = |left: &(f64, (u32, u32)), right: &(f64, (u32, u32))| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        };
        if edges.len() > cap {
            edges.select_nth_unstable_by(cap, order);
            edges.truncate(cap);
        }
        edges.sort_unstable_by(order);
        edges.into_iter().map(|(_, edge)| edge).collect()
    }

    /// Stamp every footprint group and return the stamp, so an edge walk can
    /// visit each edge once without a set.
    pub(super) fn mark_region_groups(&mut self, region: &[SurfacePoint]) -> u32 {
        let inside = self.next_stamp();
        for point in region {
            self.group_stamp[point.group as usize] = inside;
        }
        inside
    }

    /// Absolute `dab_topo_ops` ceiling for the current stage. `op_stage_limit
    /// == 0` means the policy's whole per-dab budget (the live-dab path).
    pub(super) fn topology_operation_open(
        &self,
        policy: &RemeshPolicy,
        journal: &TopoJournal,
    ) -> bool {
        let bytes = journal.encoded_size_words() as u64 * 4;
        self.dab_topo_ops < self.op_budget(policy)
            && bytes < policy.max_journal_bytes_per_stroke as u64
            && bytes.saturating_sub(self.stroke_topo_bytes)
                < policy.max_journal_bytes_per_dab as u64
    }

    pub(super) fn op_budget(&self, policy: &RemeshPolicy) -> usize {
        if self.op_stage_limit == 0 {
            policy.max_operations_per_dab
        } else {
            self.op_stage_base
                .saturating_add(self.op_stage_limit)
                .min(policy.max_operations_per_dab)
        }
    }

    /// Give each stage a share of the per-dab budget, counted from the work
    /// already spent. Splitting, flipping and collapsing share one counter, and
    /// the per-stage floor leaves each operation a chance to run.
    pub(super) fn begin_op_stage(&mut self, share: usize) {
        self.op_stage_base = self.dab_topo_ops;
        self.op_stage_limit = share.max(1);
    }

    pub(super) fn end_op_stage(&mut self) {
        self.op_stage_base = 0;
        self.op_stage_limit = 0;
    }
}

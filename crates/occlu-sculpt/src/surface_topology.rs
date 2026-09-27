//! Welded surface topology: groups, adjacency, and the overlay rows a
//! topology edit appends.
//!
//! It knows vertex ids, triangle corners and welded groups, and nothing about
//! what the surface represents. Every operator indexes its surface through
//! the same structure.
//!
//! Overlay maps sit beside the frozen base arrays: base rows are never
//! rewritten, and an edit records its new rows here so the base stays exactly
//! as the session opened it.

use crate::hash::FxHashMap;

#[derive(Clone, Copy, Debug)]
/// A welded group and its distance from a dab centre.
pub struct SurfacePoint {
    /// The welded group this point belongs to.
    pub group: u32,
    /// Distance from the dab centre, millimetres.
    pub distance: f64,
}

/// Exact duplicate STL corners share one surface group while the caller's
/// original vertex and index buffers remain unchanged.
///
/// Groups appended after construction (local brush densification) live in
/// overlay maps beside the frozen base arrays: base rows are never rewritten,
/// appended rows are addressed past the base group count, and patched rows
/// replace base rows wholesale. Undo truncates the appended tail and restores
/// patched rows from its journal, so the base build stays valid for the whole
/// session.
pub struct SurfaceTopology {
    vertex_group: Vec<u32>,
    member_off: Vec<u32>,
    members: Vec<u32>,
    adj_off: Vec<u32>,
    adj: Vec<u32>,
    triangles: Vec<[u32; 3]>,
    triangle_off: Vec<u32>,
    incident_triangles: Vec<u32>,
    base_groups: u32,
    added_members: Vec<u32>,
    // Every hot adjacency read goes through these once a stroke has edited
    // topology, so they use the fixed short-key hash, not the keyed default.
    neighbor_overlay: FxHashMap<u32, Vec<u32>>,
    incident_overlay: FxHashMap<u32, Vec<u32>>,
}

/// The faces on one edge, read off one end's incident row.
///
/// A manifold edge carries one or two faces. The inline capacity is generous;
/// a longer fan keeps its first entries, and every consumer already refuses
/// an edge with more than two faces.
#[derive(Clone, Copy, Debug)]
pub struct EdgeFaces {
    len: usize,
    faces: [u32; EdgeFaces::CAPACITY],
}

impl EdgeFaces {
    const CAPACITY: usize = 8;
}

impl std::ops::Deref for EdgeFaces {
    type Target = [u32];

    fn deref(&self) -> &[u32] {
        &self.faces[..self.len]
    }
}

impl SurfaceTopology {
    /// Weld `tris` over `verts` into groups and build adjacency.
    // welding and adjacency construction are one pass over the faces.
    #[allow(clippy::too_many_lines)]
    pub fn new(verts: &[f32], tris: &[u32]) -> SurfaceTopology {
        let vertex_count = verts.len() / 3;
        // One entry per vertex makes the hasher the loop here, and a fixed
        // hash also keeps group ids identical between runs instead of merely
        // deterministic per seed.
        let mut key_to_group: FxHashMap<(u32, u32, u32), u32> = FxHashMap::with_capacity_and_hasher(
            vertex_count,
            crate::hash::FxBuildHasher::default(),
        );
        let mut vertex_group = vec![0u32; vertex_count];
        let mut group_count = 0u32;
        for (index, point) in verts.as_chunks::<3>().0.iter().enumerate() {
            let key = (point[0].to_bits(), point[1].to_bits(), point[2].to_bits());
            let group = *key_to_group.entry(key).or_insert_with(|| {
                let next = group_count;
                group_count += 1;
                next
            });
            vertex_group[index] = group;
        }

        let mut member_off = vec![0u32; group_count as usize + 1];
        for &group in &vertex_group {
            member_off[group as usize + 1] += 1;
        }
        for index in 0..group_count as usize {
            member_off[index + 1] += member_off[index];
        }
        let mut members = vec![0u32; vertex_count];
        let mut member_cursor = member_off.clone();
        for (vertex, &group) in vertex_group.iter().enumerate() {
            let slot = member_cursor[group as usize] as usize;
            members[slot] = vertex as u32;
            member_cursor[group as usize] += 1;
        }

        let mut triangles = Vec::with_capacity(tris.len() / 3);
        for triangle in tris.as_chunks::<3>().0 {
            triangles.push([
                vertex_group[triangle[0] as usize],
                vertex_group[triangle[1] as usize],
                vertex_group[triangle[2] as usize],
            ]);
        }
        // Gather neighbour rows per group and sort each row to keep adjacency
        // unique and deterministic.
        let mut raw_off = vec![0u32; group_count as usize + 1];
        for groups in &triangles {
            for (a, b) in [
                (groups[0], groups[1]),
                (groups[1], groups[2]),
                (groups[2], groups[0]),
            ] {
                if a != b {
                    raw_off[a as usize + 1] += 1;
                    raw_off[b as usize + 1] += 1;
                }
            }
        }
        for index in 0..group_count as usize {
            raw_off[index + 1] += raw_off[index];
        }
        let mut raw = vec![0u32; raw_off[group_count as usize] as usize];
        let mut raw_cursor = raw_off.clone();
        for groups in &triangles {
            for (a, b) in [
                (groups[0], groups[1]),
                (groups[1], groups[2]),
                (groups[2], groups[0]),
            ] {
                if a != b {
                    raw[raw_cursor[a as usize] as usize] = b;
                    raw_cursor[a as usize] += 1;
                    raw[raw_cursor[b as usize] as usize] = a;
                    raw_cursor[b as usize] += 1;
                }
            }
        }
        let mut adj_off = vec![0u32; group_count as usize + 1];
        let mut adj = Vec::with_capacity(raw.len() / 2 + group_count as usize);
        for group in 0..group_count as usize {
            let row = &mut raw[raw_off[group] as usize..raw_off[group + 1] as usize];
            row.sort_unstable();
            let mut last = u32::MAX;
            for &neighbor in row.iter() {
                if neighbor != last {
                    adj.push(neighbor);
                    last = neighbor;
                }
            }
            adj_off[group + 1] = adj.len() as u32;
        }

        let mut triangle_off = vec![0u32; group_count as usize + 1];
        for triangle in &triangles {
            for &group in triangle {
                triangle_off[group as usize + 1] += 1;
            }
        }
        for group in 0..group_count as usize {
            triangle_off[group + 1] += triangle_off[group];
        }
        let mut incident_triangles = vec![0u32; triangles.len() * 3];
        let mut triangle_cursor = triangle_off.clone();
        for (triangle_index, triangle) in triangles.iter().enumerate() {
            for &group in triangle {
                let slot = triangle_cursor[group as usize] as usize;
                incident_triangles[slot] = triangle_index as u32;
                triangle_cursor[group as usize] += 1;
            }
        }
        SurfaceTopology {
            vertex_group,
            member_off,
            members,
            adj_off,
            adj,
            triangles,
            triangle_off,
            incident_triangles,
            base_groups: group_count,
            added_members: Vec::new(),
            neighbor_overlay: FxHashMap::default(),
            incident_overlay: FxHashMap::default(),
        }
    }

    /// Number of welded groups, base plus appended.
    pub fn group_count(&self) -> usize {
        self.base_groups as usize + self.added_members.len()
    }

    /// Groups present at construction; appended densification groups start here.
    pub fn base_group_count(&self) -> u32 {
        self.base_groups
    }

    /// Number of face slots currently stored.
    pub fn triangle_len(&self) -> u32 {
        self.triangles.len() as u32
    }

    /// Truncate the dense live face prefix (collapse swap-delete). Group
    /// rows are journaled separately; this only shortens the face list.
    pub fn truncate_triangles(&mut self, len: u32) {
        self.triangles.truncate(len as usize);
    }

    #[inline]
    /// The welded group a vertex belongs to.
    pub fn group_of(&self, vertex: u32) -> u32 {
        self.vertex_group[vertex as usize]
    }

    #[inline]
    /// Every vertex id that shares this group's position.
    pub fn members(&self, group: u32) -> &[u32] {
        if group < self.base_groups {
            let start = self.member_off[group as usize] as usize;
            let end = self.member_off[group as usize + 1] as usize;
            &self.members[start..end]
        } else {
            let slot = (group - self.base_groups) as usize;
            &self.added_members[slot..=slot]
        }
    }

    #[inline]
    /// The lowest vertex id in a group, used as its position carrier.
    pub fn representative(&self, group: u32) -> u32 {
        if group < self.base_groups {
            self.members[self.member_off[group as usize] as usize]
        } else {
            self.added_members[(group - self.base_groups) as usize]
        }
    }

    #[inline]
    fn base_neighbors(&self, group: u32) -> &[u32] {
        let start = self.adj_off[group as usize] as usize;
        let end = self.adj_off[group as usize + 1] as usize;
        &self.adj[start..end]
    }

    #[inline]
    fn base_incident(&self, group: u32) -> &[u32] {
        let start = self.triangle_off[group as usize] as usize;
        let end = self.triangle_off[group as usize + 1] as usize;
        &self.incident_triangles[start..end]
    }

    /// The BASE (opening-mesh) incident faces of a group, ignoring every
    /// overlay rewrite. The surroundings sampler walks this row so the target
    /// cannot follow the session's own edits.
    pub fn base_incident_triangles(&self, group: u32) -> &[u32] {
        if group < self.base_group_count() {
            self.base_incident(group)
        } else {
            &[]
        }
    }

    #[inline]
    /// One-ring group neighbours, following any densification overlay.
    ///
    /// An appended group with no derived row, or a retired group with a cleared
    /// row, has no neighbours because neither has an opening-mesh row.
    pub fn neighbors(&self, group: u32) -> &[u32] {
        // Densification overlays are empty for every session that never
        // densified, and every per-vertex hot loop (relax passes, region
        // flood, rollback scan) funnels through here: skip the hash lookup
        // when there is no overlay row to find.
        if self.neighbor_overlay.is_empty() && group < self.base_groups {
            return self.base_neighbors(group);
        }
        self.neighbors_or_empty(group)
    }

    #[inline]
    /// Faces incident to a group, following any densification overlay.
    pub fn incident_triangles(&self, group: u32) -> &[u32] {
        // Same overlay-empty shortcut as `neighbors`: the rollback scan and
        // the maintenance scope walk read this row for every touched group.
        if self.incident_overlay.is_empty() {
            return self.base_incident(group);
        }
        if let Some(row) = self.incident_overlay.get(&group) {
            return row;
        }
        self.base_incident(group)
    }

    /// Incident row without trapping on groups that have no row yet (fresh
    /// densification groups ahead of their first patch).
    pub fn incident_or_empty(&self, group: u32) -> Vec<u32> {
        if let Some(row) = self.incident_overlay.get(&group) {
            return row.clone();
        }
        if group < self.base_groups {
            return self.base_incident(group).to_vec();
        }
        Vec::new()
    }

    /// Incident row, borrowed; empty for a fresh group that has no row yet.
    pub fn incident_triangles_or_empty(&self, group: u32) -> &[u32] {
        if let Some(row) = self.incident_overlay.get(&group) {
            return row;
        }
        if group < self.base_groups {
            return self.base_incident(group);
        }
        &[]
    }

    /// Neighbor row, borrowed; empty for a fresh group that has no row yet.
    pub fn neighbors_or_empty(&self, group: u32) -> &[u32] {
        if let Some(row) = self.neighbor_overlay.get(&group) {
            return row;
        }
        if group < self.base_groups {
            return self.base_neighbors(group);
        }
        &[]
    }

    /// The faces carrying the edge between `a` and `b`, in face order. The
    /// edge's faces are the faces of `a` that also name `b`, so they are read
    /// off `a`'s incident row: no edge table is stored, patched or journaled.
    pub fn edge_triangles(&self, a: u32, b: u32) -> EdgeFaces {
        let mut edge = EdgeFaces {
            len: 0,
            faces: [0; EdgeFaces::CAPACITY],
        };
        if a == b {
            return edge;
        }
        for &triangle in self.incident_triangles(a) {
            if edge.len == EdgeFaces::CAPACITY {
                break;
            }
            if self
                .triangles
                .get(triangle as usize)
                .is_some_and(|corners| corners.contains(&b))
            {
                edge.faces[edge.len] = triangle;
                edge.len += 1;
            }
        }
        edge
    }

    #[inline]
    /// The three corners of a face slot, or `None` past the live prefix.
    pub fn triangle(&self, triangle: u32) -> Option<[u32; 3]> {
        self.triangles.get(triangle as usize).copied()
    }

    /// Append one welded group holding a single new vertex. Returns the group id.
    /// Room for `groups` appended groups and `triangles` appended faces, and
    /// for `rows` overlay rows of each kind. A growing array or map copies
    /// itself whole inside the edit that crosses its capacity.
    pub fn reserve_growth(&mut self, groups: usize, triangles: usize, rows: usize) {
        self.vertex_group.reserve(groups);
        self.added_members.reserve(groups);
        self.triangles.reserve(triangles);
        self.neighbor_overlay.reserve(rows);
        self.incident_overlay.reserve(rows);
    }

    /// Append a single-vertex group and return its id.
    pub fn append_group(&mut self, vertex: u32) -> u32 {
        let group = self.base_groups + self.added_members.len() as u32;
        self.vertex_group.push(group);
        self.added_members.push(vertex);
        group
    }

    /// Replace a neighbor row wholesale (overlay rows are kept sorted).
    pub fn set_neighbors(&mut self, group: u32, mut row: Vec<u32>) {
        row.sort_unstable();
        row.dedup();
        self.neighbor_overlay.insert(group, row);
    }

    /// Replace an incident-triangle row wholesale.
    pub fn set_incident(&mut self, group: u32, mut row: Vec<u32>) {
        row.sort_unstable();
        row.dedup();
        self.incident_overlay.insert(group, row);
    }

    /// Rewrite one triangle's group triple in place (a rewired corner keeps
    /// its triangle id; brand-new faces use [`Self::append_triangle`]).
    ///
    /// extends the row list when `triangle` is past its current end. A journal
    /// inverse restores the dense prefix by growing the session's own triangle
    /// array and writing each saved slot back; the group triples must reach the
    /// same length, or the two arrays disagree about how many faces exist and
    /// the next undo fails closed (the session then refuses a legitimate undo
    /// and the operator's history stops working). Skipping the write silently
    /// leaves exactly that divergence.
    pub fn rewrite_triangle(&mut self, triangle: u32, corners: [u32; 3]) {
        let slot = triangle as usize;
        if self.triangles.len() <= slot {
            self.triangles.resize(slot + 1, [0; 3]);
        }
        self.triangles[slot] = corners;
    }

    /// Append one triangle's group triple. Returns the triangle id.
    pub fn append_triangle(&mut self, corners: [u32; 3]) -> u32 {
        let id = self.triangles.len() as u32;
        self.triangles.push(corners);
        id
    }

    /// Drop every appended group and triangle tail, and every overlay row
    /// touching an appended group. Patched base rows are restored by the
    /// caller from its journal before or after this call.
    pub fn truncate_appended(&mut self, base_groups: u32, base_triangles: u32) {
        let base = self.base_groups;
        self.added_members
            .truncate(base_groups.saturating_sub(base) as usize);
        self.vertex_group.retain(|&group| group < base_groups);
        self.neighbor_overlay
            .retain(|&group, _| group < base_groups);
        self.incident_overlay
            .retain(|&group, _| group < base_groups);
        self.triangles.truncate(base_triangles as usize);
    }

    /// Expand an explicit set of mesh vertices over exactly `rings` connected
    /// topology one-rings. The returned ids are welded surface groups, so
    /// duplicate STL corners at the same position are protected together.
    pub fn vertex_ring_groups(&self, vertices: &[u32], rings: u32) -> Option<Vec<u32>> {
        let mut selected = vec![false; self.group_count()];
        let mut frontier = Vec::new();
        for &vertex in vertices {
            let &group = self.vertex_group.get(vertex as usize)?;
            if !selected[group as usize] {
                selected[group as usize] = true;
                frontier.push(group);
            }
        }
        for _ in 0..rings {
            let mut next = Vec::new();
            for &group in &frontier {
                for &neighbor in self.neighbors(group) {
                    if !selected[neighbor as usize] {
                        selected[neighbor as usize] = true;
                        next.push(neighbor);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        Some(
            selected
                .into_iter()
                .enumerate()
                .filter_map(|(group, included)| included.then_some(group as u32))
                .collect(),
        )
    }
}

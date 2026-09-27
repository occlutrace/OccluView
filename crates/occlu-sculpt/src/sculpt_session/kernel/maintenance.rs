//! Incremental brush maintenance: indices, budgets and normals after a dab.

use super::*;

impl SculptSession {
    /// Post-dab upkeep, all dirty-region-only: relocate moved groups in the
    /// brush grid and ray buckets (cheap span-compare skip — most sub-cell
    /// moves are free), refresh the step budget, recompute normals for the
    /// moved groups and their one-ring, and report the dirty vertices.
    pub(crate) fn after_dab_maintenance(&mut self) -> Vec<u32> {
        // Changed groups: snapshot vs current.
        let mut changed = std::mem::take(&mut self.region_candidates);
        changed.clear();
        for index in 0..self.dab_groups.len() {
            let group = self.dab_groups[index];
            let pre = self.pre_pos[group as usize];
            let rep = self.topology.representative(group) as usize * 3;
            if self.verts[rep] != pre[0]
                || self.verts[rep + 1] != pre[1]
                || self.verts[rep + 2] != pre[2]
            {
                changed.push(group);
            }
        }
        if changed.is_empty() && self.topo_touched.is_empty() {
            self.region_candidates = changed;
            return Vec::new();
        }
        // The remesh cycle leaves the ray grid to this one pass. Merges shorten
        // the face list, so the ids past its end leave the grid first.
        if self.rays.triangle_cells.len() > self.live_tris as usize {
            self.rays.drop_triangles(self.live_tris);
        }
        // Pure rewires move no vertex, so the snapshot compare above misses
        // them: their incident triangles still change corners, normals, and
        // ray spans. Refresh those rows explicitly (footprint-bounded).
        let topo_groups = std::mem::take(&mut self.topo_touched);
        if !topo_groups.is_empty() {
            let topo_tri_generation = self.next_tri_stamp();
            let mut topo_triangles: Vec<u32> = Vec::new();
            for &group in &topo_groups {
                for &triangle in self.topology.incident_triangles(group) {
                    if self.tri_marks[triangle as usize] != topo_tri_generation {
                        self.tri_marks[triangle as usize] = topo_tri_generation;
                        topo_triangles.push(triangle);
                    }
                }
            }
            if !topo_triangles.is_empty() {
                self.rays
                    .update_triangles(&self.verts, &self.tris, &topo_triangles);
            }
        }
        // Faces a caller's own pick tree must re-test against the live
        // positions: every incident face of a moved or rewired group.
        let dirty_generation = self.next_tri_stamp();
        let mut dirty = std::mem::take(&mut self.dab_dirty_triangles);
        dirty.clear();
        for &group in changed.iter().chain(topo_groups.iter()) {
            for &triangle in self.topology.incident_triangles(group) {
                if self.tri_marks[triangle as usize] != dirty_generation {
                    self.tri_marks[triangle as usize] = dirty_generation;
                    dirty.push(triangle);
                }
            }
        }
        dirty.sort_unstable();
        dirty.dedup();
        self.dab_dirty_triangles = dirty;
        // Update only triangles whose cell span changed.
        let tri_generation = self.next_tri_stamp();
        let mut moved_triangles = std::mem::take(&mut self.normal_scope);
        moved_triangles.clear();
        for &group in &changed {
            for &triangle in self.topology.incident_triangles(group) {
                if self.tri_marks[triangle as usize] != tri_generation {
                    self.tri_marks[triangle as usize] = tri_generation;
                    let Some(corners) = self.topology.triangle(triangle) else {
                        continue;
                    };
                    let old_span = self.rays.span_for_points(
                        self.pre_group(corners[0]),
                        self.pre_group(corners[1]),
                        self.pre_group(corners[2]),
                    );
                    let new_span = self.rays.span_for_points(
                        self.group_v(corners[0]),
                        self.group_v(corners[1]),
                        self.group_v(corners[2]),
                    );
                    if new_span != old_span {
                        moved_triangles.push(triangle);
                    }
                }
            }
        }
        if !moved_triangles.is_empty() {
            moved_triangles.sort_unstable();
            // The grid widens to hold whatever moved; it never re-bins the mesh
            // mid-stroke.
            self.rays
                .update_triangles(&self.verts, &self.tris, &moved_triangles);
        }
        self.normal_scope = moved_triangles;
        // Dirty-region normals: the moved groups, the rewired groups, and
        // their one-ring around them.
        let mut scope_seed = changed.clone();
        scope_seed.extend_from_slice(&topo_groups);
        let scope = self.collect_normal_scope(&scope_seed);
        self.refresh_step_budget(&scope);
        self.refresh_scope_normals(&scope);
        // Report members of the scope (positions and/or normals changed).
        let mut moved: Vec<u32> = Vec::with_capacity(scope.len() * 2);
        for &group in &scope {
            moved.extend_from_slice(self.topology.members(group));
        }
        self.normal_scope = scope;
        self.region_candidates = changed;
        moved.sort_unstable();
        moved
    }

    /// The moved groups plus their welded one-ring, deduped via a stamp
    /// with a stamp.
    pub(crate) fn collect_normal_scope(&mut self, groups: &[u32]) -> Vec<u32> {
        let generation = self.next_stamp();
        let mut scope = std::mem::take(&mut self.normal_scope);
        scope.clear();
        for &group in groups {
            if self.group_stamp[group as usize] != generation {
                self.group_stamp[group as usize] = generation;
                scope.push(group);
            }
            let neighbors = self.topology.neighbors(group);
            for &neighbor in neighbors {
                if self.group_stamp[neighbor as usize] != generation {
                    self.group_stamp[neighbor as usize] = generation;
                    scope.push(neighbor);
                }
            }
        }
        scope
    }

    /// Conflict-free area-weighted normal recompute for exactly the scope
    /// groups — each group reads only its own incident faces, with no face dedup.
    /// The same loop refreshes the per-group area used by the brush normal.
    /// Threaded twin: per-group results depend only on that group's faces,
    /// so any worker count collects the identical sequence.
    pub(crate) fn refresh_scope_normals(&mut self, scope: &[u32]) {
        #[cfg(feature = "parallel")]
        if scope.len() >= PAR_FLOOR {
            use rayon::prelude::*;
            let mut computed = std::mem::take(&mut self.normal_scratch);
            computed.resize(scope.len(), (0.0, None));
            computed
                .par_iter_mut()
                .zip(scope.par_iter())
                .for_each(|(slot, &group)| {
                    let mut sum = DVec3::ZERO;
                    let mut area = 0.0f64;
                    for &triangle in self.topology.incident_triangles(group) {
                        let Some([a, b, c]) = self.topology.triangle(triangle) else {
                            continue;
                        };
                        let origin = self.group_v(a);
                        let face = (self.group_v(b) - origin).cross(self.group_v(c) - origin);
                        sum += face;
                        area += face.length() / 6.0;
                    }
                    let normal = sum.normalize_or_zero();
                    *slot = (area as f32, (normal.length() > 1e-12).then_some(normal));
                });
            for (&group, &(area, normal)) in scope.iter().zip(&computed) {
                self.group_area[group as usize] = area;
                let Some(normal) = normal else {
                    continue;
                };
                let member_count = self.topology.members(group).len();
                for member_index in 0..member_count {
                    let vertex = self.topology.members(group)[member_index];
                    let k = vertex as usize * 3;
                    self.normals[k] = normal.x as f32;
                    self.normals[k + 1] = normal.y as f32;
                    self.normals[k + 2] = normal.z as f32;
                }
            }
            self.normal_scratch = computed;
            return;
        }
        for &group in scope {
            let mut sum = DVec3::ZERO;
            let mut area = 0.0f64;
            for &triangle in self.topology.incident_triangles(group) {
                let Some([a, b, c]) = self.topology.triangle(triangle) else {
                    continue;
                };
                let origin = self.group_v(a);
                let face = (self.group_v(b) - origin).cross(self.group_v(c) - origin);
                sum += face;
                area += face.length() / 6.0;
            }
            self.group_area[group as usize] = area as f32;
            let normal = sum.normalize_or_zero();
            if normal.length() <= 1e-12 {
                continue;
            }
            let member_count = self.topology.members(group).len();
            for member_index in 0..member_count {
                let vertex = self.topology.members(group)[member_index];
                let k = vertex as usize * 3;
                self.normals[k] = normal.x as f32;
                self.normals[k + 1] = normal.y as f32;
                self.normals[k + 2] = normal.z as f32;
            }
        }
    }

    /// One full pass computing every group's Voronoi-approximate area (1/3 of
    /// incident triangle areas, one share per corner occurrence — identical
    /// accumulation to [`Self::refresh_scope_normals`]) — session open only.
    pub(crate) fn compute_all_group_areas(&self) -> Vec<f32> {
        let mut areas = vec![0.0f32; self.topology.group_count()];
        for t in self.tris.as_chunks::<3>().0 {
            let (a, b, c) = (self.v(t[0]), self.v(t[1]), self.v(t[2]));
            let area = (b - a).cross(c - a).length() as f32 / 6.0;
            for &vi in t {
                let group = self.topology.group_of(vi) as usize;
                areas[group] += area;
            }
        }
        areas
    }
}

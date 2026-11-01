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

    /// Refresh welded brush normals and split display normals for the scope.
    pub(crate) fn refresh_scope_normals(&mut self, scope: &[u32]) {
        if scope.is_empty() {
            return;
        }
        self.prepare_normal_faces(scope);
        self.refresh_cached_brush_scope_normals(scope);
        self.refresh_cached_display_scope_normals(scope);
        self.clear_normal_face_slots();
    }

    fn prepare_normal_faces(&mut self, scope: &[u32]) {
        self.normal_triangles.clear();
        let generation = self.next_tri_stamp();
        for &group in scope {
            for &triangle in self.topology.incident_triangles(group) {
                let index = triangle as usize;
                if self.tri_marks[index] != generation {
                    self.tri_marks[index] = generation;
                    self.normal_triangles.push(triangle);
                }
            }
        }
        self.compute_normal_face_data();
        self.normal_face_slots.resize(self.tris.len() / 3, u32::MAX);
        for (slot, &triangle) in self.normal_triangles.iter().enumerate() {
            self.normal_face_slots[triangle as usize] = slot as u32;
        }
    }

    fn compute_normal_face_data(&mut self) {
        self.normal_group_faces
            .resize(self.normal_triangles.len(), DVec3::ZERO);
        self.normal_display_faces
            .resize(self.normal_triangles.len(), glam::Vec3::ZERO);
        let topology = &self.topology;
        let positions = &self.verts;
        let triangles = &self.tris;
        #[cfg(feature = "parallel")]
        if self.normal_triangles.len() >= PAR_FLOOR {
            use rayon::prelude::*;
            self.normal_group_faces
                .par_iter_mut()
                .zip(self.normal_display_faces.par_iter_mut())
                .zip(self.normal_triangles.par_iter())
                .for_each(|((group_face, display_face), &triangle)| {
                    (*group_face, *display_face) =
                        normal_face_data(topology, positions, triangles, triangle);
                });
            return;
        }
        for ((group_face, display_face), &triangle) in self
            .normal_group_faces
            .iter_mut()
            .zip(&mut self.normal_display_faces)
            .zip(&self.normal_triangles)
        {
            (*group_face, *display_face) =
                normal_face_data(topology, positions, triangles, triangle);
        }
    }

    fn clear_normal_face_slots(&mut self) {
        for &triangle in &self.normal_triangles {
            self.normal_face_slots[triangle as usize] = u32::MAX;
        }
    }

    fn refresh_cached_brush_scope_normals(&mut self, scope: &[u32]) {
        #[cfg(feature = "parallel")]
        if scope.len() >= PAR_FLOOR {
            use rayon::prelude::*;
            let mut computed = std::mem::take(&mut self.normal_scratch);
            computed.resize(scope.len(), (0.0, None));
            computed
                .par_iter_mut()
                .zip(scope.par_iter())
                .for_each(|(slot, &group)| *slot = self.cached_brush_normal(group));
            self.publish_brush_normals(scope, &computed);
            self.normal_scratch = computed;
            return;
        }
        let computed: Vec<(f32, Option<DVec3>)> = scope
            .iter()
            .map(|&group| self.cached_brush_normal(group))
            .collect();
        self.publish_brush_normals(scope, &computed);
    }

    fn cached_brush_normal(&self, group: u32) -> (f32, Option<DVec3>) {
        let mut sum = DVec3::ZERO;
        let mut area = 0.0f64;
        for &triangle in self.topology.incident_triangles(group) {
            let slot = self.normal_face_slots[triangle as usize];
            if slot == u32::MAX {
                continue;
            }
            let face = self.normal_group_faces[slot as usize];
            sum += face;
            area += face.length() / 6.0;
        }
        let normal = sum.normalize_or_zero();
        (area as f32, (normal.length() > 1e-12).then_some(normal))
    }

    fn publish_brush_normals(&mut self, scope: &[u32], computed: &[(f32, Option<DVec3>)]) {
        for (&group, &(area, normal)) in scope.iter().zip(computed) {
            self.group_area[group as usize] = area;
            let Some(normal) = normal else {
                continue;
            };
            for &vertex in self.topology.members(group) {
                let offset = vertex as usize * 3;
                self.brush_normals[offset..offset + 3].copy_from_slice(&[
                    normal.x as f32,
                    normal.y as f32,
                    normal.z as f32,
                ]);
            }
        }
    }

    fn refresh_cached_display_scope_normals(&mut self, scope: &[u32]) {
        use glam::Vec3;
        use occluview_geometry::average_duplicate_normal_group;

        self.display_normals.resize(self.verts.len(), 0.0);
        for &group in scope {
            for &vertex in self.topology.members(group) {
                let offset = vertex as usize * 3;
                self.display_normals[offset..offset + 3].fill(0.0);
            }
        }
        let topology = &self.topology;
        let triangles = &self.tris;
        let face_slots = &self.normal_face_slots;
        let face_normals = &self.normal_display_faces;
        let brush_normals = &self.brush_normals;
        for &group in scope {
            for &triangle in topology.incident_triangles(group) {
                let offset = triangle as usize * 3;
                let Some(raw) = triangles.get(offset..offset + 3) else {
                    continue;
                };
                let face_slot = face_slots[triangle as usize];
                if face_slot == u32::MAX {
                    continue;
                }
                let face = face_normals[face_slot as usize];
                for &vertex in raw {
                    if topology.group_of(vertex) != group {
                        continue;
                    }
                    let offset = vertex as usize * 3;
                    self.display_normals[offset] += face.x;
                    self.display_normals[offset + 1] += face.y;
                    self.display_normals[offset + 2] += face.z;
                }
            }
        }
        let display_normals = &mut self.display_normals;
        let output = &mut self.normal_member_output;
        for &group in scope {
            let members = topology.members(group);
            for &vertex in members {
                let offset = vertex as usize * 3;
                let normal = Vec3::new(
                    display_normals[offset],
                    display_normals[offset + 1],
                    display_normals[offset + 2],
                )
                .normalize_or_zero();
                // A vertex whose incident faces are all filtered out of the
                // display field accumulates nothing, and publishing that zero
                // shades it as a spike. Fall back to the welded brush normal,
                // which is the geometry the brush itself works from.
                let normal = if normal.length_squared() > f32::EPSILON {
                    normal
                } else {
                    brush_normals
                        .get(offset..offset + 3)
                        .map_or(Vec3::ZERO, |normal| {
                            Vec3::new(normal[0], normal[1], normal[2]).normalize_or_zero()
                        })
                };
                display_normals[offset..offset + 3].copy_from_slice(&normal.to_array());
            }
            output.resize(members.len(), Vec3::ZERO);
            let output = &mut output[..members.len()];
            output.fill(Vec3::ZERO);
            average_duplicate_normal_group(
                members.len(),
                |slot| {
                    let offset = members[slot] as usize * 3;
                    Vec3::new(
                        display_normals[offset],
                        display_normals[offset + 1],
                        display_normals[offset + 2],
                    )
                },
                output,
            );
            for (slot, &vertex) in members.iter().enumerate() {
                let offset = vertex as usize * 3;
                let own = Vec3::new(
                    display_normals[offset],
                    display_normals[offset + 1],
                    display_normals[offset + 2],
                );
                let normal = if output[slot].length_squared() > f32::EPSILON {
                    output[slot]
                } else {
                    own
                };
                display_normals[offset..offset + 3].copy_from_slice(&normal.to_array());
            }
        }
    }

    /// Refresh normals used by the geometry kernel before respace. The
    /// display-only split normals wait until post-dab maintenance, after the
    /// final positions for the dab are fixed.
    pub(crate) fn refresh_brush_scope_normals(&mut self, scope: &[u32]) {
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
                    self.brush_normals[k] = normal.x as f32;
                    self.brush_normals[k + 1] = normal.y as f32;
                    self.brush_normals[k + 2] = normal.z as f32;
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
            for &vertex in self.topology.members(group) {
                let offset = vertex as usize * 3;
                self.brush_normals[offset..offset + 3].copy_from_slice(&[
                    normal.x as f32,
                    normal.y as f32,
                    normal.z as f32,
                ]);
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

fn normal_face_data(
    topology: &SurfaceTopology,
    positions: &[f32],
    triangles: &[u32],
    triangle: u32,
) -> (DVec3, glam::Vec3) {
    use glam::Vec3;
    use occluview_geometry::facet_contributes_normal;

    let Some(groups) = topology.triangle(triangle) else {
        return (DVec3::ZERO, Vec3::ZERO);
    };
    let group_position = |group: u32| {
        let offset = topology.representative(group) as usize * 3;
        DVec3::new(
            f64::from(positions[offset]),
            f64::from(positions[offset + 1]),
            f64::from(positions[offset + 2]),
        )
    };
    let a = group_position(groups[0]);
    let group_face = (group_position(groups[1]) - a).cross(group_position(groups[2]) - a);
    let offset = triangle as usize * 3;
    let Some(raw) = triangles.get(offset..offset + 3) else {
        return (group_face, Vec3::ZERO);
    };
    let raw_position = |vertex: u32| {
        let offset = vertex as usize * 3;
        positions
            .get(offset..offset + 3)
            .map(|point| Vec3::new(point[0], point[1], point[2]))
    };
    let (Some(a), Some(b), Some(c)) = (
        raw_position(raw[0]),
        raw_position(raw[1]),
        raw_position(raw[2]),
    ) else {
        return (group_face, Vec3::ZERO);
    };
    let display_face = (b - a).cross(c - a);
    let longest_edge_sq = (b - a)
        .length_squared()
        .max((c - b).length_squared())
        .max((a - c).length_squared());
    let display_face = if display_face.is_finite()
        && facet_contributes_normal(longest_edge_sq, display_face.length_squared())
    {
        display_face
    } else {
        Vec3::ZERO
    };
    (group_face, display_face)
}

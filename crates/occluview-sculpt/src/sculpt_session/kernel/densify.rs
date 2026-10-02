//! Split: the first stage of the isotropic cycle, run under the live dab.
//!
//! A split is a pure topology change — the new vertex sits exactly on the old
//! edge, so the surface does not move and no displacement guard is involved.
//! Midpoint-exact splits preserve the shape the brush just made while they
//! restore usable sampling. Splits are capped per dab and per session, and
//! fully journaled: every overlay-row patch, triangle rewire and appended id is
//! recorded with its before/after values, so undo truncates the tail and redo
//! replays it deterministically.

use super::*;
use crate::hash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// Require each split child to meet the shared quality floor used by later
/// topology operators. Healthy coarse patches pass; needles do not.
const SPLIT_MIN_CHILD_QUALITY: f64 = crate::SLIVER_QUALITY_FLOOR;

use super::topology_journal::*;

fn midpoint_f32(a: DVec3, b: DVec3) -> Option<DVec3> {
    let mid = (a + b) * 0.5;
    // Same rounding law as open-time preparation: rounded edge midpoints,
    // never projected or relaxed.
    let rounded = DVec3::new(
        mid.x as f32 as f64,
        mid.y as f32 as f64,
        mid.z as f32 as f64,
    );
    if !rounded.is_finite() {
        return None;
    }
    if (rounded - a).length() == 0.0 || (rounded - b).length() == 0.0 {
        return None;
    }
    Some(rounded)
}

fn position_bits(p: DVec3) -> (u32, u32, u32) {
    crate::surface_topology::position_key([p.x as f32, p.y as f32, p.z as f32])
}

impl SculptSession {
    /// Actionable rewire plan for the welded edge (a, b): every incident
    /// triangle that still carries the edge, with its slot. Empty when the
    /// edge went stale (an earlier sweep already rewired it).
    fn rewire_plan(&self, a_group: u32, b_group: u32) -> Vec<(u32, usize, (u32, u32), [u32; 3])> {
        let mut plan = Vec::new();
        for tri in self.topology.edge_triangles(a_group, b_group).to_vec() {
            let offset = tri as usize * 3;
            let Some(before) = self
                .tris
                .get(offset..offset + 3)
                .map(|corners| [corners[0], corners[1], corners[2]])
            else {
                continue;
            };
            let groups = [
                self.topology.group_of(before[0]),
                self.topology.group_of(before[1]),
                self.topology.group_of(before[2]),
            ];
            for s in 0..3 {
                let pair = (groups[s], groups[(s + 1) % 3]);
                if pair == (a_group, b_group) || pair == (b_group, a_group) {
                    plan.push((tri, s, pair, before));
                    break;
                }
            }
        }
        plan
    }

    /// The two halves of one cut, from the stored cyclic order, preserve each
    /// face's winding when the edge midpoint replaces a corner.
    // the two corners, the midpoint and the rewiring buffers are one split plan.
    #[allow(clippy::too_many_arguments)]
    fn split_corners(
        before: [u32; 3],
        slot: usize,
        pair: (u32, u32),
        a_group: u32,
        b_group: u32,
        split_vertex: u32,
    ) -> ([u32; 3], [u32; 3]) {
        if pair == (a_group, b_group) {
            let mut inplace = before;
            inplace[slot] = split_vertex;
            (
                inplace,
                [before[slot], split_vertex, before[(slot + 2) % 3]],
            )
        } else {
            let mut inplace = before;
            inplace[(slot + 1) % 3] = split_vertex;
            (
                inplace,
                [split_vertex, before[(slot + 1) % 3], before[(slot + 2) % 3]],
            )
        }
    }

    /// Triangles of the welded edge (a, b) split at `split_vertex` /
    /// `split_group`: rewire in place (winding-preserving, see unit tests),
    /// append the second halves, patch rows. The split point is either a
    /// brand-new midpoint vertex or a snapped exact-position existing group.
    /// Refuses the whole edge when any child would be born below the
    /// quality floor: a half-split edge is a T-junction crack, so there is
    /// no per-triangle fallback.
    #[allow(clippy::too_many_arguments)]
    fn rewire_edge_split(
        &mut self,
        journal: &mut TopoJournal,
        a_group: u32,
        b_group: u32,
        split_vertex: u32,
        split_group: u32,
        new_tris: &mut Vec<u32>,
    ) {
        let plan = self.rewire_plan(a_group, b_group);
        let mut scope: Vec<u32> = vec![a_group, b_group, split_group];
        for (tri, s, pair, before) in plan {
            let offset = tri as usize * 3;
            let groups = [
                self.topology.group_of(before[0]),
                self.topology.group_of(before[1]),
                self.topology.group_of(before[2]),
            ];
            // The stored cyclic order decides the cut (see `split_corners`).
            let (in_place, added) =
                Self::split_corners(before, s, pair, a_group, b_group, split_vertex);
            self.tris[offset] = in_place[0];
            self.tris[offset + 1] = in_place[1];
            self.tris[offset + 2] = in_place[2];
            // The group triple follows the same cut as the vertex triple:
            // recompute it from the written vertices instead of trusting any
            // derivation, so soup duplicates can never disagree with topology.
            let written = [
                self.topology.group_of(in_place[0]),
                self.topology.group_of(in_place[1]),
                self.topology.group_of(in_place[2]),
            ];
            self.topology.rewrite_triangle(tri, written);
            let new_id = self.topology.append_triangle([
                self.topology.group_of(added[0]),
                self.topology.group_of(added[1]),
                self.topology.group_of(added[2]),
            ]);
            let new_offset = self.tris.len();
            self.tris.extend_from_slice(&added);
            debug_assert_eq!(new_id as usize, new_offset / 3);
            journal.push_rewire(TopoRewire {
                tri,
                before,
                after: in_place,
            });
            journal.push_added_tri(added);
            // Appends extend the dense live prefix in journal order: later
            // collapse swaps never touch the appended region (enforced by
            // the collapse candidate filter), so the display mirror and the
            // bucket rows stay aligned without an order table.
            self.live_tris += 1;
            journal.live_tris += 1;
            let origin = self.face_origin[tri as usize];
            self.face_origin.push(origin);
            journal.added_origins.push(origin);
            new_tris.push(new_id);
            // Incident rows, maintained precisely: a rewired triangle leaves
            // the rows of corners it no longer touches (stale membership
            // breaks the solver's symmetry and diverges the conjugate
            // gradients), and joins the rows of its new corners.
            let added_triple = [
                self.topology.group_of(added[0]),
                self.topology.group_of(added[1]),
                self.topology.group_of(added[2]),
            ];
            for &member in written.iter().chain(added_triple.iter()) {
                if !scope.contains(&member) {
                    scope.push(member);
                }
            }
            for &member in groups
                .iter()
                .chain(written.iter())
                .chain(added_triple.iter())
            {
                // Membership is per triangle id: the in-place id stays only
                // when the rewritten triple still touches the member (the
                // appended id is handled by the loop below, never here).
                let mut row = self.topology.incident_or_empty(member);
                let was_listed = row.contains(&tri);
                if written.contains(&member) && !was_listed {
                    row.push(tri);
                    Self::set_incident_row(&mut self.topology, member, row);
                } else if !written.contains(&member) && was_listed {
                    row.retain(|&candidate| candidate != tri);
                    Self::set_incident_row(&mut self.topology, member, row);
                }
            }
            for &member in &added_triple {
                let mut row = self.topology.incident_or_empty(member);
                if !row.contains(&new_id) {
                    row.push(new_id);
                    Self::set_incident_row(&mut self.topology, member, row);
                }
            }
        }
        Self::refresh_neighbor_rows(&mut self.topology, &scope);
    }

    /// Validate children against their live parent. Material coordinates are
    /// carried for wall protection; they are not the shape of a remeshed face.
    fn split_children_pass_quality(&self, a_group: u32, b_group: u32, mid: DVec3) -> bool {
        let position = |corner: u32| {
            if corner == u32::MAX {
                mid
            } else {
                self.v(corner)
            }
        };
        for (_, slot, pair, before) in self.rewire_plan(a_group, b_group) {
            let (in_place, added) =
                Self::split_corners(before, slot, pair, a_group, b_group, u32::MAX);
            let parent = before.map(position);
            let parent_area = triangle_cross(parent).length();
            let quality_floor = triangle_quality(parent, parent_area).min(SPLIT_MIN_CHILD_QUALITY);
            for child in [in_place, added] {
                let points = child.map(position);
                let area = triangle_cross(points).length();
                if area < parent_area * guards::MIN_SESSION_AREA_RATIO
                    || triangle_quality(points, area) + 1e-12 < quality_floor
                    || !Self::triangle_final_is_safe(parent, points)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Region-independent T-junction weld: an exact-position surface vertex
    /// anywhere in the spatial index (not just the path-connected region)
    /// welds instead of duplicating. Equal positions are already one
    /// surface point under the session's exact-weld law; minting a twin
    /// would crack under the next dab. Fully masked vertices never weld:
    /// protection gains no new incident faces through the back door.
    fn grid_snapped_group(&self, mid: DVec3, a_group: u32, b_group: u32) -> Option<u32> {
        let mut nearby = Vec::new();
        self.brush_grid.query_radius(mid, 1e-4, &mut nearby);
        let wanted = position_bits(mid);
        let mut found = None;
        for candidate in nearby {
            if candidate == a_group || candidate == b_group {
                continue;
            }
            if position_bits(self.group_v(candidate)) == wanted
                && self.sheet_component.get(candidate as usize).copied()
                    == self.sheet_component.get(a_group as usize).copied()
                && !self
                    .group_retired
                    .get(candidate as usize)
                    .copied()
                    .unwrap_or(true)
            {
                found = Some(found.map_or(candidate, |best: u32| best.min(candidate)));
            }
        }
        found
    }

    /// Split the welded edge (a, b) at its midpoint. Returns the split
    /// group (new, or a snapped exact-position existing one).
    // the edge, its groups, the target list and the plan buffers are one split step.
    #[allow(clippy::too_many_arguments)]
    fn split_welded_edge(
        &mut self,
        journal: &mut TopoJournal,
        snap: &HashMap<(u32, u32, u32), u32>,
        a_group: u32,
        b_group: u32,
        new_tris: &mut Vec<u32>,
        fresh_groups: &mut Vec<(u32, u32)>,
    ) -> Option<u32> {
        if self.topology.edge_triangles(a_group, b_group).len() > 2 {
            return None;
        }
        // Plan first: a stale edge (or a midpoint that rounds onto an
        // endpoint) mints nothing — no vertex without a rewire behind it,
        // so a skipped split can never strand a row-less group.
        if self.rewire_plan(a_group, b_group).is_empty() {
            return None;
        }
        let mid = midpoint_f32(self.group_v(a_group), self.group_v(b_group))?;
        let reference_mid = stored_position(
            (self.reference_group_v(a_group) + self.reference_group_v(b_group)) * 0.5,
        );
        if !self.split_children_pass_quality(a_group, b_group, mid) {
            return None;
        }
        if let Some(existing) = snap
            .get(&position_bits(mid))
            .copied()
            .filter(|&existing| {
                existing != a_group
                    && existing != b_group
                    && self.sheet_component.get(existing as usize).copied()
                        == self.sheet_component.get(a_group as usize).copied()
                    && !self
                        .group_retired
                        .get(existing as usize)
                        .copied()
                        .unwrap_or(true)
                    && self.weld_is_manifold_preserving(existing, a_group, b_group)
            })
            .or_else(|| {
                self.grid_snapped_group(mid, a_group, b_group)
                    .filter(|&existing| {
                        self.weld_is_manifold_preserving(existing, a_group, b_group)
                    })
            })
        {
            // Never mint a duplicate at an exact current-space weld.
            let vertex = self.topology.representative(existing);
            self.rewire_edge_split(journal, a_group, b_group, vertex, existing, new_tris);
            return Some(existing);
        }
        let vertex = (self.verts.len() / 3) as u32;
        self.dab_added_parents.push((
            vertex,
            self.topology.representative(a_group),
            self.topology.representative(b_group),
        ));
        self.verts
            .extend_from_slice(&[mid.x as f32, mid.y as f32, mid.z as f32]);
        self.brush_normals.extend_from_slice(&[0.0, 0.0, 0.0]);
        self.stroke_mark.push(0);
        self.material_mark.push(0);
        self.dirty_marks.push(0);
        self.reference_verts.extend_from_slice(&[
            reference_mid.x as f32,
            reference_mid.y as f32,
            reference_mid.z as f32,
        ]);
        // Birth must not reset the parent's consumed wall reserve. Interpolate
        // the opening material frame, never the already sculpted position.
        let reference_normal =
            (self.reference_group_n(a_group) + self.reference_group_n(b_group)).normalize_or_zero();
        let reference_normal = if reference_normal.length() > 1e-12 {
            reference_normal
        } else {
            self.reference_group_n(a_group)
        };
        self.reference_normals.extend_from_slice(&[
            reference_normal.x as f32,
            reference_normal.y as f32,
            reference_normal.z as f32,
        ]);
        let group = self.topology.append_group(vertex);
        self.grow_group_slot();
        // Splits never cross sheets: the child inherits the edge component
        // (the candidate gate below already proved both endpoints share it).
        self.sheet_component[group as usize] = self.sheet_component[a_group as usize];
        self.inherit_stroke_normal(group, a_group, b_group);
        self.inherit_wall_reading(group, a_group, b_group);
        // The record must sit at the vertex's true chronological position:
        // the split rewires faces onto this group immediately, so an event
        // pushed later would replay the rewires before the vertex exists.
        // Its evidence (normal/budget/area) is filled in by the finalize
        // pass, which mutates this same record in place.
        let record = journal.added_verts.len() as u32;
        journal.push_added_vert(TopoAddedVert {
            vertex,
            group,
            pos: [mid.x as f32, mid.y as f32, mid.z as f32],
            nrm: [0.0; 3],
            ref_nrm: [0.0; 3],
            reference: [
                reference_mid.x as f32,
                reference_mid.y as f32,
                reference_mid.z as f32,
            ],
            budget: 0.0,
            area: 0.0,
            component: self.sheet_component[group as usize],
        });
        fresh_groups.push((group, record));
        self.rewire_edge_split(journal, a_group, b_group, vertex, group, new_tris);
        Some(group)
    }
}

impl SculptSession {
    /// A split midpoint may only weld onto an existing group when the weld
    /// keeps one fan: the group is isolated, or it already touches both
    /// endpoints, so inserting it into the edge closes its own fan instead of
    /// merging two wedges at one vertex into a bowtie. Otherwise the split
    /// mints a fresh midpoint.
    fn weld_is_manifold_preserving(&self, existing: u32, a: u32, b: u32) -> bool {
        if self
            .group_retired
            .get(existing as usize)
            .copied()
            .unwrap_or(true)
        {
            return false;
        }
        if self.topology.incident_triangles(existing).is_empty() {
            return true;
        }
        self.topology.neighbors(existing).contains(&a)
            && self.topology.neighbors(existing).contains(&b)
    }

    // the footprint sweep is one loop over candidate edges.
    #[allow(clippy::too_many_lines)]
    pub(super) fn densify_footprint_inner(
        &mut self,
        dab: &Dab,
        policy: &RemeshPolicy,
        journal: &mut TopoJournal,
    ) {
        let target = self.target_mm(dab.radius, policy);
        let Some(target) = target else {
            return;
        };
        if target <= 1e-9 {
            return;
        }
        // Two ceilings, both live. The stroke one bounds a single gesture; the
        // session one bounds the whole session, because the stroke baseline is
        // re-armed on every `start_stroke` and would otherwise let an operator
        // grow the mesh without limit by lifting the pointer between strokes.
        let stroke_base = journal.base_groups.saturating_sub(self.stroke_retired_base);
        if policy.remaining_group_growth(
            self.live_group_count(),
            stroke_base,
            self.session_base_groups,
        ) == 0
        {
            return;
        }
        // Footprint groups only: the context band stays untouched (its
        // vertices would take zero weight and only cost growth).
        let mut region_set: HashSet<u32> = HashSet::default();
        let mut region_list: Vec<u32> = Vec::new();
        for point in &self.region_points {
            if region_set.insert(point.group) {
                region_list.push(point.group);
            }
        }
        if region_list.is_empty() {
            return;
        }
        let mut snap: HashMap<(u32, u32, u32), u32> = HashMap::default();
        for &group in &region_list {
            snap.entry(position_bits(self.group_v(group)))
                .or_insert(group);
        }
        let tri_base = (self.tris.len() / 3) as u32;
        let rewired_mark = journal.rewired.len();
        let mut fresh: Vec<(u32, u32, u32, u32)> = Vec::new();
        let mut new_tris: Vec<u32> = Vec::new();
        for _ in 0..policy.max_densify_sweeps {
            if !self.topology_operation_open(policy, journal) {
                break;
            }
            if policy.remaining_group_growth(
                self.live_group_count(),
                stroke_base,
                self.session_base_groups,
            ) == 0
            {
                break;
            }
            let mut seen: HashSet<(u32, u32)> = HashSet::default();
            let mut candidates: Vec<(f64, (u32, u32))> = Vec::new();
            for &group in &region_list {
                for &neighbor in self.topology.neighbors(group) {
                    // Either endpoint may sit outside the path-connected
                    // region: what matters is the midpoint under the cursor
                    // (a disc-straddling coarse edge is exactly the seam the
                    // operator is pointing at). The outside endpoint keeps
                    // its rows patched but takes no weight and never moves.
                    let key = if group <= neighbor {
                        (group, neighbor)
                    } else {
                        (neighbor, group)
                    };
                    if !seen.insert(key) {
                        continue;
                    }
                    // Retired groups are gone: no incident faces, no splits.
                    // Separate sheets never fuse through a split: both
                    // endpoints and any snap target share one component.
                    if self
                        .group_retired
                        .get(group as usize)
                        .copied()
                        .unwrap_or(true)
                        || self
                            .group_retired
                            .get(neighbor as usize)
                            .copied()
                            .unwrap_or(true)
                        || self.sheet_component.get(group as usize).copied()
                            != self.sheet_component.get(neighbor as usize).copied()
                    {
                        continue;
                    }
                    // The shared topology budget (split, collapse and flip draw
                    // one counter) is tested once per sweep, at the top of the
                    // sweep loop. Nothing in this collection loop spends it, so
                    // a second test here could never fire.
                    let length = (self.group_v(group) - self.group_v(neighbor)).length();
                    if !policy.should_split(length, target) {
                        continue;
                    }
                    // Collect all candidates before sorting so the operation
                    // budget selects the longest edges across the full region.
                    candidates.push((length, key));
                }
            }
            if candidates.is_empty() {
                break;
            }
            // Longest first, then the cap: the budget goes to the edges that
            // most need splitting. `total_cmp` keeps a NaN from making the
            // order depend on the sort implementation.
            let order = |left: &(f64, (u32, u32)), right: &(f64, (u32, u32))| {
                right
                    .0
                    .total_cmp(&left.0)
                    .then_with(|| left.1.cmp(&right.1))
            };
            if candidates.len() > policy.max_candidates_per_dab {
                candidates.select_nth_unstable_by(policy.max_candidates_per_dab, order);
                candidates.truncate(policy.max_candidates_per_dab);
            }
            candidates.sort_unstable_by(order);
            let mut progressed = false;
            for (_, (a, b)) in candidates {
                if !self.topology_operation_open(policy, journal) {
                    break;
                }
                if !self.topology.neighbors(a).contains(&b) {
                    continue;
                }
                let mut sweep_new_tris = Vec::new();
                let mut sweep_fresh = Vec::new();
                let Some(split) = self.split_welded_edge(
                    journal,
                    &snap,
                    a,
                    b,
                    &mut sweep_new_tris,
                    &mut sweep_fresh,
                ) else {
                    continue;
                };
                self.dab_topo_ops += 1;
                progressed = true;
                new_tris.extend(sweep_new_tris.iter().copied());
                for (fresh_group, record) in sweep_fresh {
                    let position = self.group_v(fresh_group);
                    snap.entry(position_bits(position)).or_insert(fresh_group);
                    if region_set.insert(fresh_group) {
                        region_list.push(fresh_group);
                        self.region_points.push(SurfacePoint {
                            group: fresh_group,
                            distance: (position - dab.center).length(),
                        });
                    }
                    fresh.push((fresh_group, a, b, record));
                }
                if region_set.insert(split) {
                    region_list.push(split);
                    let position = self.group_v(split);
                    self.region_points.push(SurfacePoint {
                        group: split,
                        distance: (position - dab.center).length(),
                    });
                }
            }
            if !progressed {
                break;
            }
        }
        if fresh.is_empty() && new_tris.is_empty() {
            return;
        }
        // Per-triangle slots for every appended face id.
        while (self.tri_marks.len() as u32) < self.topology.triangle_len() {
            self.tri_marks.push(0);
            self.ray_test_marks.push(0);
        }
        // Index appended faces first: later sweeps may rewire earlier
        // sweeps' appends, so every id below must be addressable before any
        // cell update runs.
        self.rays
            .insert_triangles(&self.verts, &self.tris, tri_base, self.live_tris);
        // Rewired faces move cells: ids from this dab's journal slice
        // (later sweeps can rewire earlier appends — all indexed above).
        {
            let mut rewired: Vec<u32> = journal.rewired[rewired_mark..]
                .iter()
                .map(|item| item.tri)
                .collect();
            rewired.sort_unstable();
            rewired.dedup();
            if !rewired.is_empty() {
                self.rays
                    .update_triangles(&self.verts, &self.tris, &rewired);
            }
        }
        // Finalize minted groups: face-averaged normal (endpoint fallback),
        // Voronoi-share area, ring budget, grid slot, and journal evidence. The
        // split record is appended when the group is minted, then updated here.
        for &(group, a, b, record) in &fresh {
            let mut sum = DVec3::ZERO;
            let mut area = 0.0f64;
            for &tri in &self.topology.incident_or_empty(group) {
                let Some([x, y, z]) = self.topology.triangle(tri) else {
                    continue;
                };
                let face =
                    (self.group_v(y) - self.group_v(x)).cross(self.group_v(z) - self.group_v(x));
                sum += face;
                area += face.length() / 6.0;
            }
            let mut normal = sum.normalize_or_zero();
            if normal.length() <= 1e-12 {
                normal = (self.group_n(a) + self.group_n(b)).normalize_or_zero();
            }
            let vertex = self.topology.representative(group) as usize * 3;
            if normal.length() > 1e-12 {
                self.brush_normals[vertex] = normal.x as f32;
                self.brush_normals[vertex + 1] = normal.y as f32;
                self.brush_normals[vertex + 2] = normal.z as f32;
            }
            // Subdivision changes sampling, not the physical dose the surface
            // may accept. Inherit the parent field and only ratchet upward if
            // this split actually spans a longer safe edge; seeding the child
            // from its new half-edge can throttle the next Add operation.
            let inherited_budget =
                f32::midpoint(self.step_budget[a as usize], self.step_budget[b as usize]);
            let budget = inherited_budget.max(self.shortest_incident_edge(group) as f32);
            self.step_budget[group as usize] = budget;
            self.group_area[group as usize] = area as f32;
            self.brush_grid.insert(group, self.group_v(group));
            let position = self.group_v(group);
            snap.entry(position_bits(position)).or_insert(group);
            if let Some(entry) = journal.added_verts.get_mut(record as usize) {
                entry.nrm = [
                    self.brush_normals[vertex],
                    self.brush_normals[vertex + 1],
                    self.brush_normals[vertex + 2],
                ];
                entry.ref_nrm = [
                    self.reference_normals[vertex],
                    self.reference_normals[vertex + 1],
                    self.reference_normals[vertex + 2],
                ];
                entry.budget = budget;
                entry.area = area as f32;
                entry.component = self.sheet_component[group as usize];
            }
        }
        // Refresh per-group areas before the solve uses them in its mass
        // matrix, so split-induced incident changes use current masses.
        {
            let fresh_groups: Vec<u32> = fresh.iter().map(|(group, _, _, _)| *group).collect();
            let scope = self.collect_normal_scope(&fresh_groups);
            self.refresh_scope_normals(&scope);
            // A split rewires faces, so a vertex whose position did not move
            // can still change its display normal. Join the maintenance dirty
            // scope exactly like a flip or collapse does; without this the GPU
            // report omits vertices the rewire reshaded.
            self.topo_touched.extend_from_slice(&scope);
            self.normal_scope = scope;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_weld_matches_signed_zero_coordinates() {
        let session = SculptSession::new(
            vec![-1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 2, 3, 2, 1, 3],
        )
        .expect("valid mesh fixture");
        assert_eq!(
            session.grid_snapped_group(DVec3::new(-0.0, 0.0, 0.0), 0, 1),
            Some(2)
        );
    }

    #[test]
    fn split_refuses_non_manifold_edge_without_partial_rewires() {
        for faces in [3u32, 9] {
            let mut verts = vec![-1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            let mut tris = Vec::new();
            for face in 0..faces {
                let angle = f64::from(face) * std::f64::consts::TAU / f64::from(faces);
                verts.extend_from_slice(&[0.0, angle.cos() as f32, angle.sin() as f32]);
                tris.extend_from_slice(&[0, 1, face + 2]);
            }
            let mut session =
                SculptSession::new(verts.clone(), tris.clone()).expect("valid mesh fixture");
            session.start_stroke();
            let mut journal = std::mem::take(&mut session.topo_journal);
            let mut added_faces = Vec::new();
            let mut fresh_groups = Vec::new();
            assert_eq!(
                session.split_welded_edge(
                    &mut journal,
                    &HashMap::default(),
                    0,
                    1,
                    &mut added_faces,
                    &mut fresh_groups,
                ),
                None,
                "an edge with {faces} faces must remain untouched",
            );
            assert_eq!(session.verts, verts);
            assert_eq!(session.faces(), tris);
            assert!(journal.is_empty());
            assert!(added_faces.is_empty());
            assert!(fresh_groups.is_empty());
        }
    }
}

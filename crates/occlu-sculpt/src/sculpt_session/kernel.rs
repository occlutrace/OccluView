//! Brush kernels: the per-dab region build (grid query plus a bounded surface
//! shortest path) and the incremental maintenance every dab performs (ray
//! buckets, brush grid, step budget, dirty-region normals). The dab-shape
//! solvers live beside this file: clay, flatten and the preserve skirt in
//! `dabs`, Smooth in `smooth`, and shape-preserving Relax in `relax`.

use super::*;
use occlu_geometry_math::closest_point_on_triangle;

mod dabs;
mod densify;
pub(super) mod topology_journal;
mod topology_rows;

mod isotropic;
mod maintenance;
mod path;
use path::{compounded_share, layer_depth, smooth_rate};
mod remesh;
pub(super) use remesh::input_spacing_mm;
mod relax;
mod smooth;
#[cfg(test)]
mod tests;

pub use topology_journal::{TopoJournal, TopoSlice};

/// Auto-smooth rim-taper width as a fraction of the radius.
const AUTOSMOOTH_RIM_TAPER: f64 = 0.35;
/// Taubin auto-smooth pairs per Add/Remove dab.
const CLAY_AUTOSMOOTH_PASSES: usize = 2;
/// Taubin λ/μ for the clay auto-smooth: a shrink pass then an inflate pass
/// removes grain without the volume loss of a plain Laplacian.
const TAUBIN_LAMBDA: f64 = 0.36;
const TAUBIN_MU: f64 = -0.38;
/// Minimum work size for independent Rayon loops over session data.
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) const PAR_FLOOR: usize = 8192;
/// Grid cells spanned by one brush radius.
pub(super) const GRID_CELLS_ACROSS_RADIUS: f64 = 4.0;
/// Sheet guard for region growth across a fold: a quarter-turn ridge
/// (dot ~= 0) remains traversable while a reverse face (dot ~= -1) does not.
const SHEET_COMPAT_DOT: f64 = -0.35;
impl SculptSession {
    /// Apply one dab; returns every vertex whose position or display normal
    /// changed. Returning the normal-only one-ring keeps GPU shading in sync.
    pub fn dab(&mut self, dab: &Dab) -> Vec<u32> {
        self.dab_dirty_triangles.clear();
        self.dab_added_parents.clear();
        if !self.prepare_dab(dab) {
            return Vec::new();
        }

        // Deformation and one isotropic cycle form a single live dab. The
        // remesh uses the shape produced by this dab, and relaxation uses the
        // connectivity produced by that remesh.
        let region_points = std::mem::take(&mut self.region_points);
        self.assign_sheet_axes(dab, &region_points);
        let facing = facing_sign(
            self.hit_triangle.and_then(|t| self.triangle_normal(t)),
            dab.view,
        );
        self.live_kin.vx = dab.view.x as f32;
        self.live_kin.vy = dab.view.y as f32;
        self.live_kin.vz = dab.view.z as f32;
        self.live_kin.facing = facing as f32;
        self.live_kin.mode = dab.mode as u32;
        self.live_kin.radius = dab.radius as f32;
        self.live_kin.strength = dab.strength as f32;
        let region_len = region_points.len();
        // A dab that commits no movement at all is the "the brush does
        // nothing" the operator reports; the weights, not the region, are
        // usually why, so count both.
        match dab.mode {
            BrushMode::Smooth => self.dab_smooth(dab, &region_points),
            BrushMode::Deposit => self.dab_clay(dab, &region_points, facing, 1.0),
            BrushMode::Erode => self.dab_clay(dab, &region_points, facing, -1.0),
            BrushMode::Flatten => self.dab_flatten(dab, &region_points),
            BrushMode::Relax => self.dab_relax(dab, &region_points),
        }
        self.region_points = region_points;
        // Smooth, Deposit, Erode and Relax commit their field through
        // `commit_even_layer`, which validates the same triangles against
        // the same predicates and tapers or zeroes a rejecting face's corners
        // continuously. Flatten is the exception: `dab_flatten` writes group
        // positions directly and has no layer rollback. No second rollback
        // runs on top of the layer commit: a scalar pass over the whole region would also
        // touch vertices this dab never moved, so a pre-existing sliver
        // anywhere in the footprint could reset the dab's real work.
        //
        // One full pass rather than `any(..)`: `any` short-circuits on the
        // first group past the threshold, so the running maximum it leaves
        // behind is the first measurable move rather than the dab's largest.
        // That value is published as `peak_move_mm`, and a diagnostic that
        // under-reports the movement it exists to report is worse than no
        // diagnostic. The boolean falls out of the same number.
        let mut peak = 0.0f64;
        for &group in &self.dab_groups {
            let travel = (self.group_v(group) - self.pre_group(group)).length();
            if travel > peak {
                peak = travel;
            }
        }
        let moved_any = peak > 1e-12;
        // one cycle of the isotropic loop over the footprint this dab just
        // deformed. It runs after the displacement is committed, so it repairs
        // the surface the operator actually made, and it is bounded by the
        // policy's per-dab operation count so a pointer sample stays inside its
        // frame. The cycle's own split/collapse/flip move `topo_touched`, which
        // the maintenance pass below already refreshes.
        //
        // Topology eligibility is independent of shape displacement: a dense
        // patch can need retessellation even when the shape guard rejects the
        // position field.
        if self.live_topology_open() {
            let policy = self.step_remesh_policy();
            let target = self.target_mm(dab.radius, &policy);
            if let Some(target) = target {
                self.dab_topo_ops = 0;
                self.isotropic_cycle(dab, &policy, target);
            }
        }
        self.live_kin.remesh_ops = self.dab_topo_ops as u32;
        let journal = &self.topo_journal;
        let topology_changed = (
            journal.rewired.len(),
            journal.collapsed.len(),
            journal.added_verts.len(),
            journal.added_tris.len(),
        ) != self.dab_topo_mark;
        if topology_changed {
            self.topo_revision = self.topo_revision.next();
        }
        self.live_kin.peak_move_mm = self.live_kin.peak_move_mm.max(peak as f32);
        diag::bump(|diag| {
            if !moved_any {
                diag.no_move_dabs += 1;
            }
            diag.region_points_total += region_len as u32;
        });
        self.after_dab_maintenance()
    }

    pub(super) fn prepare_dab(&mut self, dab: &Dab) -> bool {
        // The wall axis belongs to one dab. Erode then guarantees its frozen
        // probe is ready even for direct library callers that skipped the
        // worker's explicit background preparation hook.
        self.wall_facing = None;
        if !dab.center.is_finite()
            || !dab.view.is_finite()
            || !dab.radius.is_finite()
            || dab.radius <= 0.0
            || !dab.strength.is_finite()
            || dab.strength <= 0.0
        {
            return false;
        }
        let seed_is_current = self.hit_triangle.is_some_and(|triangle| {
            let Some([a, b, c]) = self.topology.triangle(triangle) else {
                return false;
            };
            let closest = closest_point_on_triangle(
                dab.center,
                self.group_v(a),
                self.group_v(b),
                self.group_v(c),
            );
            (closest - dab.center).length() <= (dab.radius * 1e-3).max(1e-4)
        });
        if !seed_is_current {
            // Direct/native callers may provide a surface point without first
            // using the picking API. A previous pick can also belong to a
            // different sheet, so validate it against this dab's center.
            let view = dab.view.normalize_or_zero();
            let origin = dab.center - (view * (dab.radius * 2.0 + 1.0));
            let _ = self.raycast(origin, view);
        }
        let Some(seed_triangle) = self.hit_triangle else {
            diag::bump(|diag| diag.seed_missing += 1);
            return false;
        };
        // One sheet reference for the whole dab: the triangle the ray hit. It
        // checks region growth for every mode, and it is what tells clay which
        // way "front" points (see `facing_sign`).
        let sheet_normal = self.triangle_normal(seed_triangle);
        // Every mode floods exactly what it paints: explicit passes read
        // only the welded one-ring, so Smooth needs no wider context band.
        // The falloff reaches zero at the paint edge and the held rim blends
        // the dab into the surface around it.
        let flood_radius = dab.radius;
        self.build_region(seed_triangle, dab.center, flood_radius, sheet_normal);
        if self.region_points.is_empty() {
            diag::bump(|diag| diag.region_empty += 1);
            return false;
        }
        // Publish marks always: a skipped topology dab must still return
        // the current live count and revision, never a stale slice.
        {
            let journal = &self.topo_journal;
            self.dab_topo_mark = (
                journal.rewired.len(),
                journal.collapsed.len(),
                journal.added_verts.len(),
                journal.added_tris.len(),
            );
        }
        // Snapshot the region's pre-dab positions for the inversion rollback
        // and the deferred grid maintenance.
        self.snapshot_region();
        true
    }

    /// Keep the brush grid usable for a dab of `radius`: rebuild (cell size
    /// matched to radius) only when the radius changed enough to make the old
    /// cell size too coarse or fine.
    fn sync_grid(&mut self, radius: f64) {
        if self.brush_grid_radius <= 0.0 {
            self.brush_grid_radius = radius;
            return;
        }
        if (0.6..=1.7).contains(&(radius / self.brush_grid_radius)) {
            return;
        }
        let cell = (radius / GRID_CELLS_ACROSS_RADIUS).max(1e-6);
        self.rebuild_brush_grid(cell);
        self.brush_grid_radius = radius;
    }

    /// Connected spatial brush footprint. Path length orders sheet transport;
    /// only spatial distance bounds the support, exactly as it bounds the
    /// stamp. Cutting the walk at one radius left a nonzero falloff cliff
    /// inside folded anatomy. The local edge guard still bars reverse seams.
    // the bounded surface flood is one graph walk.
    #[allow(clippy::too_many_lines)]
    pub(super) fn build_region(
        &mut self,
        seed_triangle: u32,
        center: DVec3,
        radius: f64,
        sheet_normal: Option<DVec3>,
    ) {
        let mut points = std::mem::take(&mut self.region_points);
        points.clear();
        if radius <= 0.0 || !radius.is_finite() {
            self.region_points = points;
            return;
        }
        let Some(seed_groups) = self.topology.triangle(seed_triangle) else {
            self.region_points = points;
            return;
        };
        self.sync_grid(radius);
        let mut candidates = std::mem::take(&mut self.region_candidates);
        let swept = self.path_active();
        if swept {
            self.path_candidates(radius, &mut candidates);
        } else {
            self.brush_grid
                .query_radius(center, radius, &mut candidates);
        }
        let in_disc = self.next_stamp();
        for &group in &candidates {
            if self.support_distance(self.group_v(group), center) <= radius {
                self.group_stamp[group as usize] = in_disc;
                self.region_distance[group as usize] = f64::INFINITY;
            }
        }
        let mut queue = std::mem::take(&mut self.region_heap);
        queue.clear();
        let seed_normal = sheet_normal.unwrap_or(DVec3::ZERO).normalize_or_zero();
        // A swept footprint grows from every face the pointer crossed, not
        // only from its end: a long step over a fold must still reach its
        // start through its own sheet.
        let mut seeds: Vec<(u32, DVec3)> = seed_groups
            .into_iter()
            .map(|group| (group, seed_normal))
            .collect();
        if swept {
            for index in 0..self.dab_path.len() {
                let triangle = self.dab_path[index].triangle;
                let Some(corners) = self.topology.triangle(triangle) else {
                    continue;
                };
                let normal = self
                    .triangle_normal(triangle)
                    .unwrap_or(seed_normal)
                    .normalize_or_zero();
                seeds.extend(corners.into_iter().map(|group| (group, normal)));
            }
        }
        for (group, normal) in seeds {
            if self.group_stamp[group as usize] == in_disc {
                let distance = self.support_distance(self.group_v(group), center);
                if distance + 1e-12 >= self.region_distance[group as usize] {
                    continue;
                }
                self.region_distance[group as usize] = distance;
                self.region_normal[group as usize] =
                    [normal.x as f32, normal.y as f32, normal.z as f32];
                queue.push(RegionQueueEntry { group, distance });
            }
        }
        while let Some(RegionQueueEntry { group, distance }) = queue.pop() {
            if distance > self.region_distance[group as usize] + 1e-12 {
                continue;
            }
            let carried = self.region_normal[group as usize];
            let carried = DVec3::new(carried[0] as f64, carried[1] as f64, carried[2] as f64);
            // Positions never move mid-flood, so the popped group's position
            // is loop-invariant: read it once, not once per relaxed edge.
            let here = self.group_v(group);
            let neighbor_count = self.topology.neighbors(group).len();
            for slot in 0..neighbor_count {
                let neighbor = self.topology.neighbors(group)[slot];
                // Retired groups hold no faces: the flood routes around
                // them, so no brush weight can ever address one.
                if self
                    .group_retired
                    .get(neighbor as usize)
                    .copied()
                    .unwrap_or(true)
                {
                    continue;
                }
                if self.group_stamp[neighbor as usize] != in_disc {
                    continue;
                }
                let next = distance + (self.group_v(neighbor) - here).length();
                if next + 1e-12 >= self.region_distance[neighbor as usize] {
                    continue;
                }
                let Some(next_normal) = self.carried_edge_normal(group, neighbor, carried) else {
                    continue;
                };
                if next + 1e-12 < self.region_distance[neighbor as usize] {
                    self.region_distance[neighbor as usize] = next;
                    self.region_normal[neighbor as usize] = [
                        next_normal.x as f32,
                        next_normal.y as f32,
                        next_normal.z as f32,
                    ];
                    queue.push(RegionQueueEntry {
                        group: neighbor,
                        distance: next,
                    });
                }
            }
        }
        for &group in &candidates {
            let distance = self.region_distance[group as usize];
            if self.group_stamp[group as usize] == in_disc && distance.is_finite() {
                points.push(SurfacePoint {
                    group,
                    // The path distance chooses the connected surface patch;
                    // the tip law remains isotropic in the hit's local metric
                    // and therefore does not inherit triangle-edge directions.
                    distance: self.support_distance(self.group_v(group), center),
                });
            }
        }
        self.region_heap = queue;
        self.region_candidates = candidates;
        self.region_points = points;
    }

    /// Transport the selected sheet normal one edge at a time. A rounded or
    /// right-angle surface advances gradually; a welded route into an opposite
    /// sheet has no face compatible with the normal carried to that edge.
    fn carried_edge_normal(&self, a: u32, b: u32, carried: DVec3) -> Option<DVec3> {
        let mut best = None;
        let mut best_dot = f64::NEG_INFINITY;
        for &triangle in self.topology.edge_triangles(a, b).iter() {
            let Some(normal) = self.triangle_normal(triangle) else {
                continue;
            };
            let normal = normal.normalize_or_zero();
            let dot = normal.dot(carried);
            if (carried.length() <= 1e-12 || dot >= SHEET_COMPAT_DOT) && dot > best_dot {
                best = Some(normal);
                best_dot = dot;
            }
        }
        best
    }

    /// Snapshot the region's pre-dab positions for the inversion rollback and
    /// the deferred grid/bucket maintenance.
    fn snapshot_region(&mut self) {
        let generation = self.next_snapshot_stamp();
        self.dab_groups.clear();
        let count = self.region_points.len();
        for index in 0..count {
            let group = self.region_points[index].group;
            self.snapshot_stamp[group as usize] = generation;
            let rep = self.topology.representative(group) as usize * 3;
            self.pre_pos[group as usize] =
                [self.verts[rep], self.verts[rep + 1], self.verts[rep + 2]];
            self.dab_groups.push(group);
        }
    }

    /// Falloff-share collection over a dab region, serial or threaded by
    /// bundle and size. The weighing closure reads shared state only, so any
    /// worker count collects the identical sequence. Indexed writes reuse
    /// the caller's capacity; stable compaction preserves encounter order.
    pub(super) fn weigh_region_into(
        region: &[SurfacePoint],
        out: &mut Vec<(u32, f64)>,
        weigh: impl Fn(SurfacePoint) -> f64 + Sync,
    ) {
        out.clear();
        #[cfg(feature = "parallel")]
        {
            if region.len() >= PAR_FLOOR {
                use rayon::prelude::*;
                out.resize(region.len(), (0, 0.0));
                out.par_iter_mut()
                    .zip(region.par_iter())
                    .for_each(|(slot, &point)| *slot = (point.group, weigh(point)));
                out.retain(|&(_, weight)| weight > 0.0);
                return;
            }
        }
        for &point in region {
            let w = weigh(point);
            if w > 0.0 {
                out.push((point.group, w));
            }
        }
    }

    /// Weight shared by every brush: tip falloff times the selected sheet's
    /// camera-independent support.
    pub(super) fn weight(&self, point: SurfacePoint, dab: &Dab) -> f64 {
        let position = self.group_v(point.group);
        let f = if self.path_active() {
            self.path_mean_stamp(position, dab.radius)
        } else {
            Self::stamp_weight_for(
                self.brush_tip,
                position - dab.center,
                point.distance,
                self.dab_axis,
                dab.radius,
            )
        };
        if f <= 0.0 {
            return 0.0;
        }
        f * self.sheet_share(point.group)
    }

    /// Move every soup member of a group, recording pre-stroke positions.
    pub(super) fn write_group_position(&mut self, group: u32, position: DVec3) {
        let member_count = self.topology.members(group).len();
        for member_index in 0..member_count {
            let vertex = self.topology.members(group)[member_index];
            self.record_stroke_vertex(vertex);
            self.set_v(vertex, position);
        }
        self.brush_grid.relocate(group, stored_position(position));
    }

    /// Area- and falloff-weighted sheet axis of a brush footprint.
    fn brush_normal(&self, weighted: &[(u32, f64)]) -> DVec3 {
        let mut sum = DVec3::ZERO;
        for &(group, weight) in weighted {
            let axis = self.sheet_axis_of(group).normalize_or_zero();
            if axis.length_squared() <= 1e-24 {
                continue;
            }
            let weight = weight * self.group_area[group as usize].max(1e-12) as f64;
            sum += axis * weight;
        }
        let normal = sum.normalize_or_zero();
        if normal.length_squared() > 1e-24 {
            normal
        } else {
            self.pointer_sheet_axis()
        }
    }

    fn stamp_weight_for(
        tip: TipStamp,
        offset: DVec3,
        distance: f64,
        axis: Option<DVec3>,
        radius: f64,
    ) -> f64 {
        if distance >= radius || radius <= 0.0 || !distance.is_finite() || !radius.is_finite() {
            return 0.0;
        }
        crate::stamp_weight(tip, offset, distance, axis, radius)
    }
}

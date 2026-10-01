//! Sculpt session lifecycle and state: construction, per-vertex accessors,
//! area-weighted normals, stroke begin/end, and undo.

use super::*;
use crate::TopologyRevision;

const WALL_SDF_CAP_MM: f64 = 10.0;
const WALL_SDF_RAYS: usize = 8;
const WALL_SDF_CONE_DEG: f64 = 30.0;
const WALL_REPROBE_DRIFT_MM: f64 = 0.25;

/// Exact kernel-owned state on both sides of a completed stroke.
#[derive(Default)]
pub struct StrokeRecord {
    /// Vertex ids whose position this stroke changed, ascending.
    pub indices: Vec<u32>,
    /// Positions before the stroke, three floats per entry.
    pub before: Vec<f32>,
    /// Positions after the stroke, three floats per entry.
    pub after: Vec<f32>,
    /// Vertices whose display normal this stroke changed, and those normals in
    /// the same order. Rewired corners reach the display in the dab that
    /// rewired them, so this carries the vertices this stroke MOVED. This is
    /// display-only and never drives undo: it is separate from
    /// `indices`/`before` precisely so an undo cannot move a vertex the stroke
    /// never displaced.
    pub normal_indices: Vec<u32>,
    /// Normals after the stroke, three floats per entry.
    pub normal_values: Vec<f32>,
    /// The topology journal this stroke wrote.
    pub journal: TopoJournal,
    /// Per-stroke dab diagnostics, counters only (see [`DabDiagnostics`]).
    pub dab: DabDiagnostics,
}

impl SculptSession {
    /// Open a session over a welded triangle mesh in millimetres.
    // session construction initialises every buffer in one place.
    #[allow(clippy::too_many_lines)]
    pub fn new(verts: Vec<f32>, tris: Vec<u32>) -> SculptSession {
        let nv = verts.len() / 3;
        let triangle_count = tris.len() / 3;
        let topology = SurfaceTopology::new(&verts, &tris);
        let rays = TriBuckets::build(&verts, &tris, 2.0);
        let reference_verts = verts.clone();
        let opening_verts = verts.clone();
        let opening_tris = tris.clone();
        let group_count = topology.group_count();
        let live_tris = tris.len() as u32 / 3;
        let face_origin: Vec<u32> = (0..live_tris).collect();
        let sheet_component = flood_sheet_components(&topology);
        let brush_grid = GroupGrid::build(GroupPositions {
            verts: &verts,
            topology: &topology,
            next: 0,
        });
        let input_spacing_mm = kernel::input_spacing_mm(&reference_verts, &tris);
        let mut s = SculptSession {
            brush_normals: vec![0.0; verts.len()],
            display_normals: vec![0.0; verts.len()],
            normal_member_output: Vec::new(),
            normal_triangles: Vec::new(),
            normal_face_slots: vec![u32::MAX; triangle_count],
            normal_group_faces: Vec::new(),
            normal_display_faces: Vec::new(),
            reference_normals: vec![0.0; verts.len()],
            brush_grid,
            brush_grid_radius: 0.0,
            group_stamp: vec![0; group_count],
            stamp_generation: 0,
            selection_stamp: vec![0; group_count],
            selection_generation: 0,
            dab_groups: Vec::new(),
            pre_pos: vec![[0.0; 3]; group_count],
            snapshot_generation: 0,
            snapshot_stamp: vec![0; group_count],
            step_budget: Vec::new(),
            stroke_mark: vec![0; nv],
            material_mark: vec![0; nv],
            stroke_epoch: 0,
            stroke_indices: Vec::new(),
            stroke_positions: Vec::new(),
            stroke_path: None,
            spine: Vec::new(),
            sheet_axis: vec![[0.0; 3]; group_count],
            sheet_axis_mark: vec![u32::MAX; group_count],
            sheet_axis_epoch: 0,
            stroke_normal: vec![[0.0; 3]; group_count],
            stroke_normal_mark: vec![u32::MAX; group_count],
            sheet_votes: Vec::new(),
            dab_path: Vec::new(),
            dab_path_spacing: 0.0,
            path_scratch: Vec::new(),
            stroke_clip_planes: Vec::new(),
            dirty_marks: vec![0; nv],
            dirty_epoch: 0,
            dirty_touched: Vec::new(),
            tri_marks: vec![0; triangle_count],
            tri_epoch: 0,
            region_points: Vec::new(),
            region_candidates: Vec::new(),
            region_heap: BinaryHeap::new(),
            region_distance: vec![f64::INFINITY; group_count],
            region_normal: vec![[0.0; 3]; group_count],
            rollback_marks: vec![0; group_count],
            rollback_epoch: 0,
            rollback_factor: vec![1.0; group_count],
            weights: Vec::new(),
            proposals: Vec::new(),
            layer_triangles: Vec::new(),
            layer_origins: Vec::new(),
            layer_groups: Vec::new(),
            layer_unsafe: Vec::new(),
            layer_controls: Vec::new(),
            layer_affected: Vec::new(),
            #[cfg(feature = "parallel")]
            normal_scratch: Vec::new(),
            #[cfg(feature = "parallel")]
            budget_scratch: Vec::new(),
            dab_dose: 1.0,
            session_base_groups: group_count as u32,
            retired_groups: 0,
            stroke_retired_base: 0,
            brush_tip: TipStamp::Ball,
            dab_axis: None,
            dab_axis_memory: None,
            preserve_skirt: false,
            normal_scope: Vec::new(),
            group_area: Vec::new(),
            fairing_scratch: crate::FairingScratch::default(),
            denoise_amount: vec![0.0; group_count],
            denoise_pass: Vec::new(),
            topo_journal: TopoJournal::default(),
            dab_topo_mark: (0, 0, 0, 0),
            live_tris,
            face_origin,
            sheet_component,
            group_retired: vec![false; group_count],
            topo_revision: TopologyRevision(0),
            dab_topo_ops: 0,
            op_stage_base: 0,
            op_stage_limit: 0,
            stroke_topo_bytes: 0,
            topo_budget_open: true,
            remesh_armed: false,
            live_kin: LiveKinematics::default(),
            live_trace_audit: false,
            topo_touched: Vec::new(),
            dab_dirty_triangles: Vec::new(),
            dab_added_parents: Vec::new(),
            verts,
            reference_verts,
            opening_verts,
            opening_tris,
            wall_probe: None,
            reference_wall_mm: vec![f32::NAN; group_count],
            reference_wall_at: vec![[f32::NAN; 3]; group_count],
            wall_facing: None,
            input_spacing_mm,

            tris,
            topology,
            rays,
            ray_test_marks: vec![0; triangle_count],
            ray_test_epoch: 0,
            hit_triangle: None,
        };
        s.refresh_all_normals();
        s.reference_normals.clone_from(&s.brush_normals);
        s.step_budget = s.compute_step_budget();
        s.group_area = s.compute_all_group_areas();
        s.reserve_session_growth();
        s
    }

    /// Reserve the growth one session is admitted, before the first stroke.
    ///
    /// Every per-vertex, per-group and per-face array grows by doubling, and a
    /// doubling copies the whole array inside the step that crosses it. All of
    /// them share one length, so the first split of a session doubled some
    /// thirty arrays in one step, and the adjacency overlays rehashed whole
    /// every few strokes. Reserving here moves that cost to the open.
    fn reserve_session_growth(&mut self) {
        let groups = RemeshPolicy::standard().max_added_groups_per_session;
        let faces = 2 * groups;
        // Overlay rows are the groups a session edits, base or appended; a
        // session that edits more than this rehashes once more, not per stroke.
        let rows = groups / 2;
        self.verts.reserve(3 * groups);
        self.brush_normals.reserve(3 * groups);
        self.display_normals.reserve(3 * groups);
        self.reference_verts.reserve(3 * groups);
        self.reference_normals.reserve(3 * groups);
        self.reference_wall_mm.reserve(groups);
        self.reference_wall_at.reserve(groups);
        self.stroke_mark.reserve(groups);
        self.material_mark.reserve(groups);
        self.sheet_axis.reserve(groups);
        self.sheet_axis_mark.reserve(groups);
        self.stroke_normal.reserve(groups);
        self.stroke_normal_mark.reserve(groups);
        self.sheet_votes.reserve(groups / 2);
        self.dirty_marks.reserve(groups);
        self.sheet_component.reserve(groups);
        self.group_retired.reserve(groups);
        self.group_stamp.reserve(groups);
        self.snapshot_stamp.reserve(groups);
        self.selection_stamp.reserve(groups);
        self.pre_pos.reserve(groups);
        self.step_budget.reserve(groups);
        self.region_distance.reserve(groups);
        self.region_normal.reserve(groups);
        self.rollback_marks.reserve(groups);
        self.rollback_factor.reserve(groups);
        self.group_area.reserve(groups);
        self.denoise_amount.reserve(groups);
        self.tris.reserve(3 * faces);
        self.face_origin.reserve(faces);
        self.tri_marks.reserve(faces);
        self.ray_test_marks.reserve(faces);
        self.topology.reserve_growth(groups, faces, rows);
        self.rays.reserve_growth(faces);
    }

    /// The current fenced topology revision.
    pub fn topology_revision(&self) -> u32 {
        self.topo_revision.0
    }

    /// Live vertex count.
    pub fn vertex_count(&self) -> usize {
        self.verts.len() / 3
    }

    /// Select the tip stamp every following dab uses.
    pub fn set_brush_tip(&mut self, tip: TipStamp) {
        self.brush_tip = tip;
    }

    /// Set the stroke bearing the knife stamp follows for the next dab, in
    /// mesh-local space. `None`, a non-finite value or a zero vector leaves the
    /// knife with its narrow radial footprint instead of an empty dab.
    pub fn set_dab_axis(&mut self, axis: Option<DVec3>) {
        self.dab_axis = axis.filter(|value| value.is_finite() && value.length() > 1e-12);
    }

    /// Arm the Ctrl shape-preserve skirt for the next dab: motion continues
    /// past the footprint with a smoothstep falloff instead of ending at
    /// the rim. Transient by contract — the worker sets it on every dab
    /// from the live modifier key (like the tip stamp), so true can never
    /// leak from one dab into the next; the default is off.
    pub fn set_preserve_skirt(&mut self, on: bool) {
        self.preserve_skirt = on;
    }

    #[inline]
    pub(super) fn v(&self, i: u32) -> DVec3 {
        let k = i as usize * 3;
        DVec3::new(
            self.verts[k] as f64,
            self.verts[k + 1] as f64,
            self.verts[k + 2] as f64,
        )
    }

    pub(super) fn set_v(&mut self, i: u32, p: DVec3) {
        let k = i as usize * 3;
        self.verts[k] = p.x as f32;
        self.verts[k + 1] = p.y as f32;
        self.verts[k + 2] = p.z as f32;
    }

    #[inline]
    fn n(&self, i: u32) -> DVec3 {
        let k = i as usize * 3;
        DVec3::new(
            self.brush_normals[k] as f64,
            self.brush_normals[k + 1] as f64,
            self.brush_normals[k + 2] as f64,
        )
    }

    pub(super) fn display_n(&self, i: u32) -> DVec3 {
        let k = i as usize * 3;
        DVec3::new(
            self.display_normals[k] as f64,
            self.display_normals[k + 1] as f64,
            self.display_normals[k + 2] as f64,
        )
    }

    pub(super) fn triangle_normal(&self, triangle: u32) -> Option<DVec3> {
        let offset = triangle as usize * 3;
        let indices = self.tris.get(offset..offset + 3)?;
        let normal = (self.v(indices[1]) - self.v(indices[0]))
            .cross(self.v(indices[2]) - self.v(indices[0]))
            .normalize_or_zero();
        (normal.length() > 1e-12).then_some(normal)
    }

    /// Hand out the next stamp generation for the shared group-stamp buffer,
    /// resetting it on the rare u32 wrap so a stale stamp can never
    /// masquerade as current.
    pub(super) fn next_stamp(&mut self) -> u32 {
        self.stamp_generation = self.stamp_generation.wrapping_add(1);
        if self.stamp_generation == 0 {
            self.group_stamp.iter_mut().for_each(|s| *s = 0);
            self.stamp_generation = 1;
        }
        self.stamp_generation
    }

    pub(super) fn next_snapshot_stamp(&mut self) -> u32 {
        self.snapshot_generation = self.snapshot_generation.wrapping_add(1);
        if self.snapshot_generation == 0 {
            self.snapshot_stamp.fill(0);
            self.snapshot_generation = 1;
        }
        self.snapshot_generation
    }

    pub(super) fn next_selection_stamp(&mut self) -> u32 {
        self.selection_generation = self.selection_generation.wrapping_add(1);
        if self.selection_generation == 0 {
            self.selection_stamp.iter_mut().for_each(|s| *s = 0);
            self.selection_generation = 1;
        }
        self.selection_generation
    }

    pub(super) fn next_tri_stamp(&mut self) -> u32 {
        self.tri_epoch = self.tri_epoch.wrapping_add(1);
        if self.tri_epoch == 0 {
            self.tri_marks.iter_mut().for_each(|s| *s = 0);
            self.tri_epoch = 1;
        }
        self.tri_epoch
    }

    /// Refresh welded brush normals and per-vertex shading normals.
    pub(super) fn refresh_all_normals(&mut self) {
        let mut group_normals = vec![DVec3::new(0.0, 0.0, 0.0); self.topology.group_count()];
        let tris = std::mem::take(&mut self.tris);
        for t in tris.as_chunks::<3>().0 {
            let (a, b, c) = (self.v(t[0]), self.v(t[1]), self.v(t[2]));
            let fnrm = (b - a).cross(c - a);
            for &vi in t {
                let group = self.topology.group_of(vi) as usize;
                group_normals[group] += fnrm;
            }
        }
        self.tris = tris;
        for (group, sum) in group_normals.into_iter().enumerate() {
            let normal = sum.normalize_or_zero();
            for &vertex in self.topology.members(group as u32) {
                let k = vertex as usize * 3;
                self.brush_normals[k] = normal.x as f32;
                self.brush_normals[k + 1] = normal.y as f32;
                self.brush_normals[k + 2] = normal.z as f32;
            }
        }
        self.refresh_all_display_normals();
    }

    fn refresh_all_display_normals(&mut self) {
        use glam::Vec3;
        use occlu_geometry_math::{accumulate_smooth_normals, average_duplicate_normal_group};

        let vertex_count = self.verts.len() / 3;
        let mut source_normals = accumulate_smooth_normals(vertex_count, &self.tris, |index| {
            let offset = index * 3;
            self.verts
                .get(offset..offset + 3)
                .map(|position| Vec3::new(position[0], position[1], position[2]))
        });
        for normal in &mut source_normals {
            *normal = normal.normalize_or_zero();
        }
        self.display_normals.resize(self.verts.len(), 0.0);
        for group in 0..self.topology.group_count() as u32 {
            let members = self.topology.members(group);
            self.normal_member_output.resize(members.len(), Vec3::ZERO);
            let output = &mut self.normal_member_output[..members.len()];
            output.fill(Vec3::ZERO);
            average_duplicate_normal_group(
                members.len(),
                |slot| source_normals[members[slot] as usize],
                output,
            );
            for (slot, &vertex) in members.iter().enumerate() {
                let normal = if output[slot].length_squared() > f32::EPSILON {
                    output[slot]
                } else {
                    source_normals[vertex as usize]
                };
                if normal.length_squared() > f32::EPSILON {
                    let offset = vertex as usize * 3;
                    self.display_normals[offset..offset + 3].copy_from_slice(&normal.to_array());
                }
            }
        }
    }

    /// Set the pointer time the next dab call stands for, in milliseconds.
    /// All brushes use this dose for both held and traveling rays; the swept
    /// stamp distributes it along the path. Non-finite time means no dose, and
    /// delayed input is capped at one full interval.
    pub fn set_dab_elapsed_ms(&mut self, elapsed_ms: f64) {
        self.dab_dose = if elapsed_ms.is_finite() {
            (elapsed_ms / DWELL_FULL_DOSE_MS).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// Enable the allocation-heavy footprint report for the next pointer
    /// calls. This never changes a geometry verdict; the browser opts in only
    /// while its verbose sculpt diagnostics are enabled.
    pub fn set_live_trace_audit(&mut self, enabled: bool) {
        self.live_trace_audit = enabled;
    }

    /// Record one before-image per vertex and stroke using the session stamp
    /// buffer.
    pub(super) fn record_stroke_vertex(&mut self, vertex: u32) {
        let mark = &mut self.stroke_mark[vertex as usize];
        if *mark == self.stroke_epoch {
            return;
        }
        *mark = self.stroke_epoch;
        let k = vertex as usize * 3;
        self.stroke_indices.push(vertex);
        self.stroke_positions
            .extend_from_slice(&self.verts[k..k + 3]);
    }

    pub(super) fn begin_dirty_batch(&mut self) {
        self.dirty_epoch = self.dirty_epoch.wrapping_add(1);
        if self.dirty_epoch == 0 {
            self.dirty_marks.fill(0);
            self.dirty_epoch = 1;
        }
        self.dirty_touched.clear();
    }

    pub(super) fn merge_dirty_batch(&mut self, moved: &[u32]) {
        for &vertex in moved {
            let mark = &mut self.dirty_marks[vertex as usize];
            if *mark != self.dirty_epoch {
                *mark = self.dirty_epoch;
                self.dirty_touched.push(vertex);
            }
        }
    }

    pub(super) fn finish_dirty_batch(&mut self) -> Vec<u32> {
        self.dirty_touched.sort_unstable();
        let capacity = self.dirty_touched.capacity();
        let finished = std::mem::take(&mut self.dirty_touched);
        self.dirty_touched = Vec::with_capacity(capacity);
        finished
    }

    #[inline]
    pub(super) fn group_v(&self, group: u32) -> DVec3 {
        self.v(self.topology.representative(group))
    }

    #[inline]
    pub(super) fn group_n(&self, group: u32) -> DVec3 {
        self.n(self.topology.representative(group))
    }

    pub(super) fn reference_group_v(&self, group: u32) -> DVec3 {
        let vertex = self.topology.representative(group);
        let offset = vertex as usize * 3;
        DVec3::new(
            self.reference_verts[offset] as f64,
            self.reference_verts[offset + 1] as f64,
            self.reference_verts[offset + 2] as f64,
        )
    }

    pub(super) fn reference_group_n(&self, group: u32) -> DVec3 {
        let vertex = self.topology.representative(group);
        let offset = vertex as usize * 3;
        DVec3::new(
            self.reference_normals[offset] as f64,
            self.reference_normals[offset + 1] as f64,
            self.reference_normals[offset + 2] as f64,
        )
    }

    /// Prepare the immutable opposing-wall ray grid. Callers do this on their
    /// session worker before enabling Remove input; the guard never builds it
    /// from inside a dab, because that measures a whole-mesh distance field
    /// during the first carve and stalls the stroke. Without a prepared probe a
    /// group's opening thickness is unknown and the reserve stays at its widest.
    pub fn prepare_wall_probe(&mut self) {
        self.ensure_wall_probe();
    }

    /// Warm the local wall-thickness memo around the pointer. Returns the
    /// number of fresh group readings computed, bounded by `budget`.
    pub fn prime_wall_region(&mut self, center: DVec3, radius: f64, budget: usize) -> usize {
        self.ensure_wall_probe();
        if !center.is_finite() || !radius.is_finite() || radius <= 0.0 || budget == 0 {
            return 0;
        }
        let mut candidates = std::mem::take(&mut self.region_candidates);
        candidates.clear();
        self.brush_grid
            .query_radius(center, radius, &mut candidates);
        candidates.sort_unstable();
        candidates.dedup();
        let radius_squared = radius * radius;
        let mut computed = 0;
        for group in candidates.iter().copied() {
            if computed >= budget {
                break;
            }
            if self.group_retired[group as usize]
                || !self.reference_wall_mm[group as usize].is_nan()
                || (self.group_v(group) - center).length_squared() > radius_squared
            {
                continue;
            }
            self.reference_group_wall_mm(group);
            computed += 1;
        }
        self.region_candidates = candidates;
        computed
    }

    pub(super) fn ensure_wall_probe(&mut self) {
        if self.wall_probe.is_none() {
            let opening_verts = std::mem::take(&mut self.opening_verts);
            let opening_tris = std::mem::take(&mut self.opening_tris);
            self.wall_probe = Some(SdfProbe::new(
                opening_verts,
                opening_tris,
                WALL_SDF_CAP_MM,
                WALL_SDF_RAYS,
                WALL_SDF_CONE_DEG,
            ));
        }
    }

    /// Opening-surface thickness for one welded group. Probe all soup members
    /// and use the thinnest reading, so a sharp feature inherits its safest
    /// local limit. A memo loses at least its measured drift when the material
    /// point has moved, making reuse conservative.
    pub(super) fn reference_group_wall_mm(&mut self, group: u32) -> f64 {
        let index = group as usize;
        let cached = self.reference_wall_mm[index];
        let material = self.reference_group_v(group);
        if !cached.is_nan() {
            let anchor = self.reference_wall_at[index];
            let drift = (DVec3::new(anchor[0] as f64, anchor[1] as f64, anchor[2] as f64)
                - material)
                .length();
            if drift.is_nan() {
                return cached as f64;
            }
            if drift <= WALL_REPROBE_DRIFT_MM {
                return (cached as f64 - drift).max(0.0);
            }
        }
        let Some(probe) = self.wall_probe.as_ref() else {
            // The probe is built when Remove is armed, never from inside a dab:
            // a whole-mesh SDF build on the first carve stalls the stroke.
            // Until it exists the opening thickness is unknown, and an unknown
            // thickness must not be invented as a thin one, so the reading
            // stays unknown and the cap keeps the reserve at its widest.
            return WALL_SDF_CAP_MM;
        };
        let mut wall = f64::INFINITY;
        for &vertex in self.topology.members(group) {
            let offset = vertex as usize * 3;
            let Some(position) = self.reference_verts.get(offset..offset + 3) else {
                continue;
            };
            let Some(normal) = self.reference_normals.get(offset..offset + 3) else {
                continue;
            };
            let origin = DVec3::new(position[0] as f64, position[1] as f64, position[2] as f64);
            let normal = DVec3::new(normal[0] as f64, normal[1] as f64, normal[2] as f64);
            wall = wall.min(probe.thickness_at_pose(origin, normal) as f64);
        }
        if !wall.is_finite() {
            wall = WALL_SDF_CAP_MM;
        }
        self.reference_wall_mm[index] = wall as f32;
        self.reference_wall_at[index] = [material.x as f32, material.y as f32, material.z as f32];
        wall
    }

    /// Pass the thinner parent's memo to a remesh child. Unknown readings stay
    /// unknown so the new material point is measured against the frozen probe.
    pub(super) fn inherit_wall_reading(&mut self, group: u32, a: u32, b: u32) {
        let (wa, wb) = (
            self.reference_wall_mm[a as usize],
            self.reference_wall_mm[b as usize],
        );
        if wa.is_nan() || wb.is_nan() {
            return;
        }
        let parent = if wa <= wb { a } else { b };
        self.reference_wall_mm[group as usize] = self.reference_wall_mm[parent as usize];
        self.reference_wall_at[group as usize] = self.reference_wall_at[parent as usize];
    }

    /// Live per-vertex shading normals, interleaved xyz.
    pub fn normals(&self) -> &[f32] {
        &self.display_normals
    }

    /// Per-vertex welded normals used by brush geometry, interleaved xyz.
    /// Duplicate vertices in one welded group carry the same value. Unlike
    /// [`Self::normals`], this is not the display-normal field and does not
    /// preserve a rendered crease.
    pub fn brush_normals(&self) -> &[f32] {
        &self.brush_normals
    }

    /// Faces the last [`SculptSession::dab`] may have moved or rewired,
    /// deduplicated and sorted.
    pub fn dab_dirty_triangles(&self) -> &[u32] {
        &self.dab_dirty_triangles
    }

    /// Vertices the last [`SculptSession::dab`] minted, each as
    /// `(child, parent_a, parent_b)` over raw vertex ids. A caller that carries
    /// per-vertex data the kernel does not can blend it from the two parents.
    pub fn dab_added_parents(&self) -> &[(u32, u32, u32)] {
        &self.dab_added_parents
    }

    /// The index buffer, live prefix first.
    ///
    /// The session's connectivity is part of its contract, not an internal:
    /// a product that paints protection over a physical wall, or that has to
    /// state which faces the stroke touched, reads it. `&[u32]` gives the
    /// topology without handing out a mutable handle to it.
    /// The live face array, as the session holds it.
    ///
    /// Read-only, and deliberately not "the committed mesh": a consumer that
    /// wants the surface the operator is sculpting reads it here, and the
    /// module's own protection suite compares a face's corners before and
    /// after a stroke through this.
    pub fn faces(&self) -> &[u32] {
        &self.tris
    }

    /// The welded topology the session walks.
    ///
    /// Read-only: a consumer asks which vertices are neighbours, which faces
    /// carry an edge, and how many live triangles there are. All of that is
    /// already reachable through this session's own operations, so exposing it
    /// adds a view of the contract rather than a way to break it.
    pub fn topology(&self) -> &SurfaceTopology {
        &self.topology
    }

    /// Would this dab move anything?
    ///
    /// The question a caller asks before a stroke. A facet larger than the
    /// smallest brush would otherwise answer wrongly: every corner sits outside
    /// the radius, so the dab moves nothing and the brush reads as dead on that
    /// face. Preparation refines such a facet so this returns true.
    ///
    /// Asked on the footprint the dab would actually move, not on the radius:
    /// a reach test that ignored facing or the tip profile would report a
    /// stroke that the brush cannot deliver.
    pub fn dab_reaches_surface(&mut self, dab: &Dab) -> bool {
        if !self.prepare_dab(dab) {
            return false;
        }
        let region = std::mem::take(&mut self.region_points);
        self.assign_sheet_axes(dab, &region);
        let reaches = region.iter().any(|point| self.weight(*point, dab) > 0.0);
        self.region_points = region;
        reaches
    }

    /// Whether every live face still satisfies the Apply contract against the
    /// session's own reference surface.
    ///
    /// The assertion a bar makes after a clinical protection band, in one call
    /// instead of re-deriving it from the buffers.
    pub fn is_apply_safe(&self) -> bool {
        let point = |verts: &[f32], vertex: u32| {
            let offset = vertex as usize * 3;
            DVec3::new(
                verts[offset] as f64,
                verts[offset + 1] as f64,
                verts[offset + 2] as f64,
            )
        };
        self.tris[..self.live_tris as usize * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .all(|triangle| {
                let vertices = [triangle[0], triangle[1], triangle[2]];
                let baseline = vertices.map(|vertex| point(&self.reference_verts, vertex));
                let stored = vertices.map(|vertex| point(&self.verts, vertex));
                Self::triangle_final_is_safe(baseline, stored)
            })
    }

    /// Accepted face slot each live face descends from, in live order.
    /// Apply inherits typed face data through these origins.
    pub fn face_origins(&self) -> Vec<u32> {
        self.face_origin[..self.live_tris as usize].to_vec()
    }
}

/// Path-connected sheet identity per welded group, flooded once at open.
/// Splits inherit the parent component; collapse and flip require one
/// component across every corner, so separate sheets can never fuse.
fn flood_sheet_components(topology: &SurfaceTopology) -> Vec<u32> {
    let count = topology.group_count();
    let mut component = vec![u32::MAX; count];
    let mut next = 0u32;
    for seed in 0..count as u32 {
        if component[seed as usize] != u32::MAX {
            continue;
        }
        component[seed as usize] = next;
        let mut stack = vec![seed];
        while let Some(group) = stack.pop() {
            for &neighbor in topology.neighbors(group) {
                if component[neighbor as usize] == u32::MAX {
                    component[neighbor as usize] = next;
                    stack.push(neighbor);
                }
            }
        }
        next = next.saturating_add(1);
    }
    component
}

/// Cloneable iterator over group representative positions for the grid build.
pub(crate) struct GroupPositions<'a> {
    verts: &'a [f32],
    topology: &'a SurfaceTopology,
    next: u32,
}

impl Clone for GroupPositions<'_> {
    fn clone(&self) -> Self {
        GroupPositions {
            verts: self.verts,
            topology: self.topology,
            next: 0,
        }
    }
}

impl Iterator for GroupPositions<'_> {
    type Item = DVec3;
    fn next(&mut self) -> Option<DVec3> {
        if (self.next as usize) >= self.topology.group_count() {
            return None;
        }
        let vertex = self.topology.representative(self.next) as usize * 3;
        self.next += 1;
        Some(DVec3::new(
            self.verts[vertex] as f64,
            self.verts[vertex + 1] as f64,
            self.verts[vertex + 2] as f64,
        ))
    }
}

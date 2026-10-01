//! The complete mutable sculpt session and its safety policy.
//!
//! The session owns its brush topology, current-surface raycast, sparse undo,
//! anti-inversion rollback and thin-wall reserve. The operators it shares with
//! the rest of the crate — fairing, tip stamps, the flatten projector, the
//! knot clamp, tangential respace — live outside it, so a session owns
//! budgets, undo and guards, never the math.

use glam::DVec3;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub use crate::TipStamp;

/// Brush time one full dab dose stands for, in milliseconds. Every call uses
/// this interval whether its ray rests or travels; elapsed time is capped
/// against it so a delayed sample cannot release a burst.
pub const DWELL_FULL_DOSE_MS: f64 = 120.0;

/// The same full-dose interval in seconds, for callers that pace in seconds.
pub const DWELL_FULL_DOSE_SECONDS: f64 = DWELL_FULL_DOSE_MS / 1000.0;

mod grid;
mod guards;
mod history;
mod kernel;
mod live_trace;
mod material;
mod ray_buckets;
mod sdf;
mod session;
mod sheet;
pub(crate) use kernel::PAR_FLOOR;
use ray_buckets::TriBuckets;
use sdf::SdfProbe;
use sheet::SpineSample;
/// The counters live in a thread-local block rather than on the session: the
/// per-vertex clamp runs inside immutable-borrow regions (`clamp_step_at`
/// takes `&self`) and the parallel build needs the session itself to stay
/// `Sync`. A session lives on one worker thread, so the block is exactly as
/// session-scoped as the stroke it describes.
pub(super) mod diag {
    use super::DabDiagnostics;
    use std::cell::Cell;

    thread_local! {
        static DAB: Cell<DabDiagnostics> = const { Cell::new(DabDiagnostics::new()) };
    }

    /// Start a stroke's counters over.
    pub(super) fn reset() {
        DAB.with(|slot| slot.set(DabDiagnostics::new()));
    }

    /// Add to the running counters.
    pub(super) fn bump(edit: impl FnOnce(&mut DabDiagnostics)) {
        DAB.with(|slot| {
            let mut current = slot.get();
            edit(&mut current);
            slot.set(current);
        });
    }

    /// Take the counters for the stroke that just ended.
    pub(super) fn take() -> DabDiagnostics {
        DAB.with(|slot| {
            let current = slot.get();
            slot.set(DabDiagnostics::new());
            current
        })
    }

    /// Copy the running counters without resetting them. Live dab traces
    /// subtract this snapshot from the post-call totals so one pointer
    /// segment reports its own refusals, not the whole stroke.
    pub(super) fn peek() -> DabDiagnostics {
        DAB.with(Cell::get)
    }

    /// Put back counters saved with `take`.
    pub(super) fn restore(saved: DabDiagnostics) {
        DAB.with(|slot| slot.set(saved));
    }
}

/// Why a stroke's dabs did or did not move the surface, as counters.
///
/// A pure displacement changes neither the face nor the vertex count, so a
/// report built from those counts cannot separate "the kernel refused this
/// dab" from "the display did not show what the kernel moved". These fields
/// answer that from one pasted line: which refusal fired, how often, and how
/// hard the anti-inversion gain had to scale the dose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DabDiagnostics {
    /// No seed triangle for the dab (stale pick, ray left the surface).
    pub seed_missing: u32,
    /// The region flood came back empty (sheet gate, clip, isolated patch).
    pub region_empty: u32,
    /// Vertices whose step budget is zero or unusable: no move at all.
    pub clamp_zero: u32,
    /// Vertices whose move the per-vertex budget truncated.
    pub clamp_truncated: u32,
    /// Dabs whose dose the anti-inversion gain had to scale down.
    pub gain_scaled_dabs: u32,
    /// Smallest gain a dab ran with, in permille (1000 = unscaled).
    pub gain_min_permille: u32,
    /// Dabs whose anti-inversion waves pulled moved vertices back.
    pub rollback_resets: u32,
    /// Dabs whose region violates a guard before this dab moves anything.
    pub already_unsafe_dabs: u32,
    /// Dabs that committed no movement, including dabs whose weights are zero.
    /// This reports the operator's "the brush does nothing" symptom separately
    /// from per-guard refusals.
    pub no_move_dabs: u32,
    /// Region points the dabs walked, so "no movement" can be read against how
    /// much surface the brush actually covered.
    pub region_points_total: u32,
}

impl DabDiagnostics {
    /// A zeroed set, usable in a `const` context.
    pub const fn new() -> Self {
        Self {
            seed_missing: 0,
            region_empty: 0,
            clamp_zero: 0,
            clamp_truncated: 0,
            gain_scaled_dabs: 0,
            gain_min_permille: 1000,
            rollback_resets: 0,
            already_unsafe_dabs: 0,
            no_move_dabs: 0,
            region_points_total: 0,
        }
    }

    /// Flat wire order, shared with the operator report.
    pub fn encode(self) -> [u32; 10] {
        [
            self.seed_missing,
            self.region_empty,
            self.clamp_zero,
            self.clamp_truncated,
            self.gain_scaled_dabs,
            self.gain_min_permille,
            self.rollback_resets,
            self.already_unsafe_dabs,
            self.no_move_dabs,
            self.region_points_total,
        ]
    }
}

pub(crate) use live_trace::LiveKinematics;
pub use live_trace::{LiveTrace, SCULPT_LIVE_KERNEL};
pub use session::StrokeRecord;
mod stroke;
pub use stroke::tip_dab_spacing_mm;
mod visible_ray;
pub use visible_ray::SculptRayConstraints;
mod warm_up;
pub use warm_up::warm_up_brush_step;

pub use crate::{RemeshPolicy, SurfacePoint, SurfaceTopology, TopologyRevision};
use grid::GroupGrid;
/// Densification journal types, exposed for consumers that mirror a
/// session's topology as it changes.
pub use kernel::{TopoJournal, TopoSlice};

/// Brush modes. The discriminants are part of the wire contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BrushMode {
    /// Add material along the selected sheet axis.
    Deposit = 0,
    /// Remove material against the selected sheet axis.
    Erode = 1,
    /// Explicit local relaxation, a few passes per dab.
    Smooth = 2,
    /// Level toward the selection plane.
    ///
    /// The reference kernel leaves 3 unassigned; the viewer assigns it to
    /// Flatten because its own UI offers the mode. A payload never crosses
    /// between the two kernels, so the discriminant is only read by this
    /// crate's own encoder and decoder, which both name the same thing.
    Flatten = 3,
    /// Gently even small surface detail while preserving the broad form.
    Relax = 4,
}

impl BrushMode {
    /// The mode a wire payload names, or `None` for an unknown value.
    pub fn from_u32(v: u32) -> Option<BrushMode> {
        match v {
            0 => Some(BrushMode::Deposit),
            1 => Some(BrushMode::Erode),
            2 => Some(BrushMode::Smooth),
            3 => Some(BrushMode::Flatten),
            4 => Some(BrushMode::Relax),
            _ => None,
        }
    }
}

/// One dab's inputs. `view` picks the hit and the side of the selected sheet
/// the operator works from. It does not orient brush weights or displacement.
#[derive(Clone, Copy)]
pub struct Dab {
    /// Mesh-local dab centre, millimetres.
    pub center: DVec3,
    /// Falloff radius in mesh-local millimetres.
    pub radius: f64,
    /// 0..1; the default Deposit strength ships at 0.8. Brush time scales the
    /// effect independently through [`SculptSession::set_dab_elapsed_ms`].
    pub strength: f64,
    /// Ray direction from the camera into the scene; used for picking and the
    /// clicked-side sign of Add and Remove.
    pub view: DVec3,
    /// The operation this dab performs.
    pub mode: BrushMode,
}

#[derive(Clone, Copy)]
struct StrokePathState {
    origin: DVec3,
    dir: DVec3,
    view_axis: DVec3,
    hit: DVec3,
    normal: DVec3,
    /// The face under `hit` after the last step's edits, so the next step's
    /// footprint can grow from the start of its path.
    triangle: u32,
    near: f64,
    far: f64,
}

/// One surface point of a swept step and the face the pointer ray hit there.
#[derive(Clone, Copy)]
struct PathSample {
    point: DVec3,
    triangle: u32,
}

#[derive(Clone, Copy)]
struct RegionQueueEntry {
    group: u32,
    distance: f64,
}

impl PartialEq for RegionQueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.group == other.group && self.distance.to_bits() == other.distance.to_bits()
    }
}

impl Eq for RegionQueueEntry {}

impl PartialOrd for RegionQueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RegionQueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.group.cmp(&self.group))
    }
}

/// What one pointer segment produced.
pub struct StrokePathResult {
    /// Where the pointer ray landed on the edited surface, with its welded
    /// brush normal. Public `raycast` methods retain split display normals.
    pub hit: Option<(DVec3, DVec3)>,
    /// Vertices whose position or normal changed, sorted and deduplicated.
    pub moved: Vec<u32>,
    /// Whether the call reached the pointer's end rather than breaking early.
    pub complete: bool,
    /// The ordered journal slice this call published: the split/collapse/flip
    /// records its own isotropic cycle committed, so the display mirror grows
    /// by exactly the operations the kernel ran.
    pub topo: TopoSlice,
    /// Per-call evidence for the operator console: what the kernel did and
    /// what the live surface looks like after this pointer segment.
    pub live: LiveTrace,
}

// The tip stamp owns the production falloff profile; the laws below are the
// session's own shaping rules.
/// Hermite smoothstep ramping 0→1 as `t` goes 0→`edge`, then held at 1 — the
/// clay auto-smooth plateau weight.
fn smoothstep(edge: f64, t: f64) -> f64 {
    if edge <= 0.0 {
        return if t > 0.0 { 1.0 } else { 0.0 };
    }
    let s = (t / edge).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

/// Which way "front" points for this dab, decided ONCE from the triangle the
/// ray hit. The operator sculpts the sheet they clicked, whichever way its
/// triangles happen to be wound; the reverse side of a thin wall keeps its
/// opposite normals and stays culled either way.
fn facing_sign(sheet: Option<DVec3>, view: DVec3) -> f64 {
    match sheet {
        Some(normal) if normal.dot(view) > 0.0 => -1.0,
        _ => 1.0,
    }
}

fn triangle_cross(points: [DVec3; 3]) -> DVec3 {
    (points[1] - points[0]).cross(points[2] - points[0])
}

fn triangle_quality(points: [DVec3; 3], double_area: f64) -> f64 {
    let squared_edges = [(0, 1), (1, 2), (2, 0)]
        .into_iter()
        .map(|(a, b)| {
            let edge = points[b] - points[a];
            edge.dot(edge)
        })
        .sum::<f64>();
    if squared_edges <= 1e-18 {
        0.0
    } else {
        2.0 * 3.0f64.sqrt() * double_area / squared_edges
    }
}

/// Match the coordinates that `set_v` actually stores before running any
/// geometry invariant. WebAssembly exposes f32 buffers, so accepting an f64
/// candidate and rounding only during commit can put the stored triangle on
/// the opposite side of a quality threshold.
fn stored_position(position: DVec3) -> DVec3 {
    DVec3::new(
        position.x as f32 as f64,
        position.y as f32 as f64,
        position.z as f32 as f64,
    )
}

/// Möller–Trumbore, front and back faces; returns t along the ray.
fn ray_tri(orig: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let pv = dir.cross(e2);
    let det = e1.dot(pv);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = orig - a;
    let u = tv.dot(pv) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = dir.dot(qv) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(qv) * inv;
    if t > 1e-9 {
        Some(t)
    } else {
        None
    }
}

/// A prepared, mutable sculpting session over one mesh.
pub struct SculptSession {
    /// Live vertex positions, interleaved xyz in millimetres.
    pub verts: Vec<f32>,
    /// Each vertex's material coordinate: the point of the surface accepted at
    /// session start that the vertex stands for. Safety is measured from that
    /// surface, never from the result of the previous stroke. A brush move
    /// leaves the point where it is; a remesh slide or merge carries it along
    /// the opening surface (see `material.rs`).
    reference_verts: Vec<f32>,
    /// Frozen geometry for opposing-wall probes. `reference_verts` can follow
    /// a remeshed material point; this snapshot is the session-opening shell.
    opening_verts: Vec<f32>,
    opening_tris: Vec<u32>,
    wall_probe: Option<SdfProbe>,
    /// Memoized opposing-wall thickness and the material pose each reading
    /// belongs to. NaN means the group has not been probed yet.
    reference_wall_mm: Vec<f32>,
    reference_wall_at: Vec<[f32; 3]>,
    /// The sheet-axis orientation used by this dab's Remove wall guard.
    wall_facing: Option<f64>,
    /// Stroke stamp of each vertex's first material move, so the journal keeps
    /// exactly one before-image per vertex and stroke.
    material_mark: Vec<u32>,
    /// Area-weighted input scale, frozen before any stroke changes the mesh.
    input_spacing_mm: f64,
    reference_normals: Vec<f32>,
    tris: Vec<u32>,
    topology: SurfaceTopology,
    rays: TriBuckets,
    ray_test_marks: Vec<u32>,
    ray_test_epoch: u32,
    hit_triangle: Option<u32>,
    brush_normals: Vec<f32>,
    display_normals: Vec<f32>,
    normal_member_output: Vec<glam::Vec3>,
    normal_triangles: Vec<u32>,
    normal_face_slots: Vec<u32>,
    normal_group_faces: Vec<DVec3>,
    normal_display_faces: Vec<glam::Vec3>,
    /// Spatial index over group representative positions, cell size matched
    /// to the brush radius; relocated incrementally during strokes.
    brush_grid: GroupGrid,
    /// Brush radius the grid's cell size is tuned for (0 until the first dab).
    brush_grid_radius: f64,
    /// Scratch stamps for local graph traversals.
    group_stamp: Vec<u32>,
    stamp_generation: u32,
    selection_stamp: Vec<u32>,
    selection_generation: u32,
    /// Immutable membership of the pre-dab position snapshot. Traversals must
    /// never invalidate it while deformation, remesh and index upkeep run.
    snapshot_stamp: Vec<u32>,
    dab_groups: Vec<u32>,
    pre_pos: Vec<[f32; 3]>,
    snapshot_generation: u32,
    /// Anti-inversion step budget per group: shortest incident welded edge,
    /// capped at 1 mm, refreshed for the touched region after each dab.
    step_budget: Vec<f32>,
    /// Current stroke's undo record, stamp-deduped per vertex: parallel
    /// index/position arrays in first-touched order.
    stroke_mark: Vec<u32>,
    stroke_epoch: u32,
    stroke_indices: Vec<u32>,
    stroke_positions: Vec<f32>,
    stroke_path: Option<StrokePathState>,
    /// Step spine and per-group sheet axes are transient brush state. Axis
    /// marks fence the one step that assigned each cached value.
    spine: Vec<SpineSample>,
    sheet_axis: Vec<[f32; 3]>,
    sheet_axis_mark: Vec<u32>,
    sheet_axis_epoch: u32,
    /// Each group's shading normal when the current stroke first touched it.
    /// A topology split inherits this value from its edge parents.
    stroke_normal: Vec<[f32; 3]>,
    stroke_normal_mark: Vec<u32>,
    sheet_votes: Vec<(DVec3, DVec3, f64)>,
    /// The path the current step sweeps, from the previous step's end to this
    /// pointer ray. Empty for a press or a hold, which stamp once at the dab
    /// centre.
    dab_path: Vec<PathSample>,
    /// Travel one full dab stands for on the current path, millimetres.
    dab_path_spacing: f64,
    /// Grid query buffer reused by the swept footprint.
    path_scratch: Vec<u32>,
    stroke_clip_planes: Vec<[f64; 4]>,
    dirty_marks: Vec<u32>,
    dirty_epoch: u32,
    dirty_touched: Vec<u32>,
    /// Triangle stamp buffer for per-dab incident-triangle dedup.
    tri_marks: Vec<u32>,
    tri_epoch: u32,
    /// Reusable region, proposal and per-group computation buffers.
    region_points: Vec<SurfacePoint>,
    region_candidates: Vec<u32>,
    region_heap: BinaryHeap<RegionQueueEntry>,
    region_distance: Vec<f64>,
    region_normal: Vec<[f32; 3]>,
    rollback_marks: Vec<u32>,
    rollback_epoch: u32,
    rollback_factor: Vec<f32>,
    weights: Vec<(u32, f64)>,
    proposals: Vec<(u32, DVec3)>,
    /// Deduplicated incident faces reused across whole-layer line search trials.
    layer_triangles: Vec<u32>,
    /// Scratch buffers for proposal origins, groups, rejecting faces, moving
    /// controls and rollback waves. The session reuses their allocation
    /// capacity between dabs.
    layer_origins: Vec<DVec3>,
    layer_groups: Vec<u32>,
    layer_unsafe: Vec<u32>,
    layer_controls: Vec<u32>,
    layer_affected: Vec<u32>,
    #[cfg(feature = "parallel")]
    normal_scratch: Vec<(f32, Option<DVec3>)>,
    #[cfg(feature = "parallel")]
    budget_scratch: Vec<f32>,
    /// Dabs of brush time the current call stands for.
    dab_dose: f64,
    /// Welded groups the session opened with. The per-stroke growth ceiling
    /// resets on every `start_stroke`, so it cannot bound a SESSION; this is the
    /// baseline the session-wide ceiling measures against.
    session_base_groups: u32,
    /// Group slots a merge retired, session-wide. Slots are never reused, so
    /// the live surface is the slot count less these, and that is what the
    /// growth ceilings measure: a remesh that splits and merges in turn adds
    /// slots without adding surface.
    retired_groups: u32,
    /// `retired_groups` when the current stroke started.
    stroke_retired_base: u32,
    brush_tip: TipStamp,
    /// Stroke direction in the stable view plane for the current dab, or
    /// `None` on a stroke's first dab. Orients the knife stamp; set in
    /// `stroke.rs` beside the hit triangle, cleared with the stroke path.
    dab_axis: Option<DVec3>,
    /// Last travel-derived knife bearing, kept across strokes. A press with
    /// no travel yet (a single hold dab) orients along the operator's own
    /// previous gesture instead of a round dimple or a camera accident.
    dab_axis_memory: Option<DVec3>,
    /// Ctrl shape-preserve: extend the dab past its footprint with a
    /// smoothstep skirt instead of ending at the rim. Operator-held per
    /// dab, never persisted.
    preserve_skirt: bool,
    normal_scope: Vec<u32>,
    /// Approximate Voronoi (1/3 incident) area per group, kept fresh for the
    /// touched region — weighs the clay brush normal so a denser-tessellated
    /// side cannot bias the build direction, and is the mass-matrix diagonal
    /// the implicit Smooth solve reads.
    group_area: Vec<f32>,
    /// Reused sparse-system storage for the Smooth dab's cotangent solve, so a
    /// stroke does not reallocate its rows, columns and preconditioner on every
    /// dab.
    fairing_scratch: crate::FairingScratch,
    /// Per-group along-push dose for the clay denoise, indexed by group, plus
    /// the pass buffer it is updated through. Held on the session so a stroke
    /// does not reallocate them per dab.
    denoise_amount: Vec<f64>,
    denoise_pass: Vec<(u32, f64)>,
    /// This stroke's topology journal: appended ids, rewired faces, merges
    /// and retired groups, in order. Every dab writes it, and it is the undo
    /// record for the whole stroke. Adjacency is derived from the faces and
    /// never journaled.
    topo_journal: TopoJournal,
    /// Journal lengths when the current dab started its topology work. The
    /// single revision increment per dab compares against them.
    dab_topo_mark: (usize, usize, usize, usize),
    /// Live faces occupy a dense prefix of `tris`/`topology.triangles` at
    /// all times: collapse swap-deletes into the prefix and truncates the
    /// vec, so the vec length is the live count and the display mirror,
    /// ray buckets, and undo checks share one discipline.
    live_tris: u32,
    /// Accepted face slot each live raw triangle descends from. Splits
    /// inherit the parent origin, swap-delete carries origins with content,
    /// flips keep slots. Apply inherits typed face data through origins,
    /// never proximity.
    face_origin: Vec<u32>,
    /// Path-connected sheet identity per welded group, flooded at open.
    /// Topology ops never cross components: silent sheet-bridging is a
    /// refusal-grade fault, so inadmissible edges are skipped instead.
    sheet_component: Vec<u32>,
    /// Welded groups retired by collapse. Their member slots persist
    /// (coincident with the survivor) and compact only in Apply; every
    /// group-space consumer gates on this bitmap plus the cleared rows.
    group_retired: Vec<bool>,
    /// Fenced topology revision: one increment per dab that commits a
    /// topology operation. The display publish path carries it; a response
    /// that does not follow the last applied revision is a typed fault,
    /// never a partial apply.
    topo_revision: TopologyRevision,
    /// Stroke/session topology journal byte totals for the policy
    /// budgets. Exceeded budgets stand topology down (Smooth continues);
    /// they never refuse a case or drop a finished stroke.
    stroke_topo_bytes: u64,
    /// Last dab's journal cost, used as the next dab's budget estimate.
    /// False once any budget trips; topology work skips while set.
    topo_budget_open: bool,
    /// shared per-dab budget. Split, collapse, and flip draw from the
    /// same counter; exhaustion skips topology work while Smooth continues.
    dab_topo_ops: usize,
    /// Per-stage share of `dab_topo_ops`: split, flip and collapse each get a
    /// slice so no stage can starve the others. `op_stage_limit == 0` means
    /// "use the whole policy budget" (the live-dab path). The pair is
    /// `op_stage_base + op_stage_limit`, so a stage is bounded relative to
    /// what earlier stages already spent, not to zero.
    op_stage_base: usize,
    op_stage_limit: usize,
    /// Topology repair runs only while a stroke is open, so each change has a
    /// bounded journal that belongs to that stroke's history record.
    remesh_armed: bool,
    /// Per-pointer-call clay/remesh evidence packed onto the dab reply.
    live_kin: LiveKinematics,
    /// The footprint soup audit is diagnostic work, not a geometry guard. It
    /// stays off on the production pointer path unless the browser explicitly
    /// asks for the detailed live console.
    live_trace_audit: bool,
    /// Groups whose faces this dab rewires without moving their positions.
    /// They join the normal and ray refresh explicitly because maintenance
    /// detects movement by comparing position snapshots.
    topo_touched: Vec<u32>,
    /// Faces whose corners this dab may have moved or rewired, deduplicated.
    /// A caller that keeps its own raycast tree over the pre-stroke surface
    /// tests these against the live positions instead of rebuilding the tree.
    dab_dirty_triangles: Vec<u32>,
    /// Vertices this dab minted, each with the raw corner ids of the edge it
    /// split. A caller that carries per-vertex data the kernel does not (a
    /// colour, a texture coordinate) blends it from those two corners.
    dab_added_parents: Vec<(u32, u32, u32)>,
}

impl SculptSession {
    /// Rebuild the brush grid over current group positions with an explicit
    /// cell size (a deliberate brush-size change; never per-dab).
    fn rebuild_brush_grid(&mut self, cell_size: f64) {
        let origin = self.brush_grid.origin();
        let topology = &self.topology;
        let verts = &self.verts;
        self.brush_grid = GroupGrid::build_with_cell_size(
            (0..topology.group_count() as u32).map(|group| {
                let vertex = topology.representative(group) as usize * 3;
                DVec3::new(
                    verts[vertex] as f64,
                    verts[vertex + 1] as f64,
                    verts[vertex + 2] as f64,
                )
            }),
            origin,
            cell_size,
        );
    }
}

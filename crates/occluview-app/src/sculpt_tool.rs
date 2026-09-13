//! Interactive sculpt-brush state: dental-CAD-style freeforming (an
//! Add/Remove clay knife and a Smooth relaxer) dragged directly on a scan
//! surface inside the Mesh Editor. This module owns the pure state and
//! math — which tool is armed, the size/intensity sliders and their unit
//! conversions, the persistent per-layer kernel session, and the per-drag
//! dab scheduler — while the
//! egui/viewport glue lives in `app::app_sculpt` and the geometry kernel is
//! [`occluview_core::BrushSession`].

use crate::sculpt_worker::SculptWorker;
use glam::{Affine3A, Vec3};
use occluview_core::{
    mesh_edit_buffers_from_mesh, mesh_from_sculpt_session_like, BrushMode, BrushSession,
    BrushStroke, Mesh, Scene, SceneMeshId, Vertex,
};
use occluview_render::PreparedSceneTopology;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;

/// Brush-size slider bounds/default, in abstract 0..100 units (not mm — the
/// operator asked for a feel slider, not a measurement). Mapped to a mm radius
/// by [`size_to_radius_mm`].
pub(crate) const SCULPT_SIZE_DEFAULT: f32 = 40.0;
pub(crate) const SCULPT_SIZE_MIN: f32 = 1.0;
pub(crate) const SCULPT_SIZE_MAX: f32 = 100.0;
/// Intensity slider bounds/default, 0..100 units, mapped to a 0..1 kernel
/// strength by dividing by 100.
pub(crate) const SCULPT_INTENSITY_DEFAULT: f32 = 50.0;
pub(crate) const SCULPT_INTENSITY_MIN: f32 = 1.0;
pub(crate) const SCULPT_INTENSITY_MAX: f32 = 100.0;
/// Mm radius the size slider maps to at its ends.
const SCULPT_RADIUS_MIN_MM: f32 = 0.4;
const SCULPT_RADIUS_MAX_MM: f32 = 12.0;
/// How much Shift widens the Smooth footprint. Per-dab force cannot climb
/// past the kernel's pass ceiling — a dab converges toward the relaxed patch
/// its own boundary pins — so the honest way to smooth harder is to push that
/// boundary outward and iron a wider patch per dab. Wider dabs also space
/// further apart along the drag, so a Shift stroke queues fewer, larger jobs
/// instead of flooding the worker's bounded apply queue.
pub(crate) const SHIFT_SMOOTH_RADIUS_BOOST: f32 = 1.75;
/// One notch of the mouse wheel changes a slider by this many units.
pub(crate) const SCULPT_WHEEL_STEP: f32 = 6.0;
/// Dab spacing along the drag path, as a fraction of the brush radius: dabs are
/// laid down every `radius * this` of cursor travel so buildup is even and
/// framerate-independent (the arc-length stroke spacing sculpting tools use).
pub(crate) const DAB_SPACING_FRACTION: f32 = 0.15;
/// While the cursor is (near) stationary and the button held, lay a fresh dab
/// this often so a held brush keeps depositing on the same spot at a steady,
/// framerate-independent rate.
pub(crate) const HOLD_DAB_INTERVAL_SEC: f32 = 0.03;
/// Never emit more than this many dabs in one frame. A long cursor jump is
/// sampled across this bounded budget and the scheduler advances to the
/// current cursor, so expensive geometry work cannot accumulate behind input.
pub(crate) const MAX_DABS_PER_FRAME: usize = 8;

/// Map the 0..100 size slider to a mm brush radius (linear across the usable
/// dental range).
pub(crate) fn size_to_radius_mm(size: f32) -> f32 {
    let t = ((size - SCULPT_SIZE_MIN) / (SCULPT_SIZE_MAX - SCULPT_SIZE_MIN)).clamp(0.0, 1.0);
    SCULPT_RADIUS_MIN_MM + t * (SCULPT_RADIUS_MAX_MM - SCULPT_RADIUS_MIN_MM)
}

/// Which sculpt tool button is armed. Only two, per the operator's request:
/// one Add/Remove clay knife (Shift carves instead of builds) and one Smooth
/// relaxer (Shift forces maximum smoothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SculptToolKind {
    /// Clay knife: build material, or carve it away with Shift held.
    AddRemove,
    /// Relaxer: flatten/even the surface, forced to maximum with Shift held.
    Smooth,
}

impl SculptToolKind {
    /// The kernel brush mode for a dab, given whether Shift is held.
    pub(crate) fn brush_mode(self, shift: bool) -> BrushMode {
        match self {
            Self::AddRemove if shift => BrushMode::Remove,
            Self::AddRemove => BrushMode::Add,
            Self::Smooth => BrushMode::Smooth,
        }
    }

    /// The kernel per-dab strength for this tool: the intensity slider for
    /// Add/Remove and Smooth; Shift forces Smooth straight to maximum, the
    /// forced mode the kernel documents. Doubling the slider instead sounded
    /// gentler but saturated: at 50% it was already near the pass ceiling and
    /// at 100% it changed nothing.
    pub(crate) fn dab_strength(self, intensity01: f32, shift: bool) -> f32 {
        match self {
            Self::Smooth if shift => 1.0,
            _ => intensity01.clamp(0.0, 1.0),
        }
    }

    /// The world-space dab radius for this tool: the size slider's mm value,
    /// widened for a Shift-forced Smooth. [`SHIFT_SMOOTH_RADIUS_BOOST`]
    /// explains why the footprint is the lever that actually strengthens
    /// smoothing.
    pub(crate) fn dab_radius_mm(self, base_mm: f32, shift: bool) -> f32 {
        match self {
            Self::Smooth if shift => base_mm * SHIFT_SMOOTH_RADIUS_BOOST,
            _ => base_mm,
        }
    }
}

/// The armed tool plus the persistent kernel session and the live drag.
#[derive(Default)]
pub(crate) struct SculptTool {
    /// The armed tool; `None` = sculpting off, selection gestures own the
    /// primary button again.
    pub(crate) armed: Option<SculptToolKind>,
    /// Persistent background kernel, kept alive across strokes on the same
    /// layer. The UI never runs a dab synchronously.
    pub(crate) worker: Option<SculptWorker>,
    /// Bookkeeping for the drag currently in flight (button held).
    pub(crate) stroke: Option<StrokeState>,
    /// A mesh-edit Done action waits for the background commit before closing
    /// the edit session, so a fast click cannot discard a valid stroke.
    pub(crate) finish_requested: bool,
    /// Queue pressure rejected a stroke boundary. Retry it from the worker
    /// poll once older Apply commands have drained.
    pub(crate) finish_retry: bool,
    /// Undo/redo waits for an asynchronous sculpt completion before swapping
    /// an older scene over the worker's current shadow.
    pub(crate) pending_history: Option<bool>,
    /// The last surface hit acquired by the viewport input pass. The cursor
    /// painter runs after that pass and reuses it for held drags, avoiding a
    /// second BVH traversal on every repaint.
    pub(crate) cursor_hit: Option<occluview_core::ScenePickHit>,
    /// Pointer coordinates belonging to [`Self::cursor_hit`].
    pub(crate) cursor_pointer: Option<[f32; 2]>,
    pending: Option<PendingSculptPreparation>,
    /// Canceled preparation workers are reaped without blocking the UI. They
    /// remain owned here until their non-cancellable kernel phase finishes,
    /// so dropping a receiver never leaves an untracked CPU/RAM worker behind.
    retired_preparations: Vec<thread::JoinHandle<()>>,
}

struct PendingSculptPreparation {
    layer_id: SceneMeshId,
    topology_id: u64,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<SculptSession, String>>,
    thread: thread::JoinHandle<()>,
}

impl SculptTool {
    /// Toggle `kind`: arming it takes over from any other tool; clicking the
    /// armed tool again disarms sculpting. Never drops the prepared session
    /// (same layer, so the next stroke stays instant) but does end any live
    /// drag so a half-applied stroke does not leak between tools.
    pub(crate) fn toggle(&mut self, kind: SculptToolKind) {
        self.stroke = None;
        self.clear_cursor_hit();
        self.armed = if self.armed == Some(kind) {
            None
        } else {
            Some(kind)
        };
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = None;
        self.stroke = None;
        self.clear_cursor_hit();
        self.finish_requested = false;
        self.finish_retry = false;
        self.pending_history = None;
        self.worker = None;
        self.cancel_pending_preparation();
    }

    /// Drop the prepared session and any live stroke while KEEPING the armed
    /// tool. Called whenever the scene geometry changes underneath us (a load,
    /// a delete, another mesh edit, or an undo/redo) — a preserved-`topology_id`
    /// sculpt commit is undone WITHOUT changing the id, so the id alone cannot
    /// tell the geometry reverted; the session must be re-prepared from
    /// the fresh scene on the next stroke.
    pub(crate) fn invalidate_session(&mut self) {
        self.stroke = None;
        self.clear_cursor_hit();
        self.worker = None;
        self.finish_requested = false;
        self.finish_retry = false;
        self.pending_history = None;
        self.cancel_pending_preparation();
    }

    /// Whether a valid worker already covers `layer_id` at
    /// `topology_id` (so no re-prepare is needed for the next stroke).
    pub(crate) fn session_matches(&self, layer_id: SceneMeshId, topology_id: u64) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.layer_id == layer_id && worker.topology_id == topology_id)
    }

    pub(crate) fn clear_cursor_hit(&mut self) {
        self.cursor_hit = None;
        self.cursor_pointer = None;
    }

    pub(crate) fn set_cursor_hit(&mut self, pointer: [f32; 2], hit: occluview_core::ScenePickHit) {
        self.cursor_pointer = Some(pointer);
        self.cursor_hit = Some(hit);
    }

    pub(crate) fn cursor_hit_for(&self, pointer: [f32; 2]) -> Option<occluview_core::ScenePickHit> {
        (self.cursor_pointer == Some(pointer)).then_some(self.cursor_hit?)
    }

    pub(crate) fn pending_matches(&self, layer_id: SceneMeshId, topology_id: u64) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.layer_id == layer_id && pending.topology_id == topology_id
        })
    }

    pub(crate) fn worker_has_pending_work(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_quiescent())
    }

    /// Whether a mesh edit must wait for Sculpt to settle. `worker` can exist
    /// idle for instant next-stroke reuse, so its presence alone is not busy.
    pub(crate) fn is_busy(&self) -> bool {
        self.stroke.is_some()
            || self.pending.is_some()
            || self.worker_has_pending_work()
            || self.finish_requested
            || self.finish_retry
            || self.pending_history.is_some()
    }

    /// Queue the O(n) brush preparation. The worker owns the target mesh
    /// snapshot; the UI only stores a receiver and remains responsive while
    /// welding, adjacency construction, and grid setup run.
    pub(crate) fn queue_preparation(&mut self, scene: Arc<Scene>, index: usize) -> bool {
        self.reap_finished_preparations();
        let Some(entry) = scene.meshes().get(index) else {
            return false;
        };
        if !entry.visible || entry.mesh.is_point_cloud() || entry.mesh.triangle_count() == 0 {
            return false;
        }
        // A single averaged scale is only mathematically valid for a rigid or
        // uniformly scaled transform. Refuse shear/non-uniform placement here
        // instead of applying a direction-dependent brush radius as if it were
        // isotropic.
        if uniform_scene_scale(&entry.transform).is_none() {
            return false;
        }
        let layer_id = entry.id();
        let topology_id = entry.mesh.topology_id();
        if self.session_matches(layer_id, topology_id)
            || self.pending_matches(layer_id, topology_id)
        {
            return self.session_matches(layer_id, topology_id);
        }

        self.cancel_pending_preparation();
        // A previous cancellation may still be inside the O(n) BVH/kernel
        // preparation. Do not launch a second scan-sized worker on a
        // resource-constrained machine; wait for the owned worker to finish
        // and retry next frame.
        if !self.retired_preparations.is_empty() {
            return false;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);

        // The worker gets the one mesh it prepares, not the case it came from.
        // An `Arc<Scene>` alive in a background thread would make every
        // in-place scene edit on the UI thread find a second handle for as long
        // as the preparation runs: the edit would land in a copy the worker
        // never reads, and the container is copied per frame until the worker
        // finishes. Taking the mesh keeps the worker on the geometry it
        // actually needs, which is also why this can run off-thread.
        //
        // The mesh itself is shared, so this is a pointer: the worker warms the
        // very cell the scene will read, which is the point of warming it.
        let mesh = entry.mesh.clone();
        let transform = entry.transform;
        drop(scene);

        let spawned = thread::Builder::new()
            .name("occluview-sculpt-prepare".to_string())
            .spawn(move || {
                if worker_cancel.load(Ordering::Relaxed) {
                    return;
                }
                // Warming the picking tree is an O(n) build inside an
                // `OnceLock`, so it cannot be interrupted once it starts. Skip
                // it when nobody is waiting for this preparation any more: a
                // cancel that lands during the build would otherwise keep the
                // worker (and the layer's memory) alive for the whole build,
                // which is exactly the delay `cancel_pending_preparation`
                // exists to avoid. The build still happens on demand at the
                // first pick of a live session.
                if !worker_cancel.load(Ordering::Relaxed) {
                    mesh.warm_bvh();
                }
                let prepared = {
                    let buffers = mesh_edit_buffers_from_mesh(&mesh);
                    BrushSession::prepare(&buffers).map_err(|error| error.to_string())
                };
                let result = prepared.map(move |session| {
                    let scale = uniform_scene_scale(&transform).unwrap_or(1.0);
                    let shadow = Arc::new(RwLock::new(mesh.vertices().to_vec()));
                    let topology = PreparedSceneTopology::from_mesh(&mesh);
                    SculptSession {
                        layer_id,
                        topology_id,
                        session,
                        base_mesh: mesh,
                        shadow,
                        topology,
                        world_to_local: transform.inverse(),
                        local_per_world: 1.0 / scale,
                        dirty_stroke: false,
                        stroke_start_mesh: None,
                    }
                });
                if !worker_cancel.load(Ordering::Relaxed) {
                    let _ = sender.send(result);
                }
            });
        let Ok(thread) = spawned else {
            return false;
        };
        self.pending = Some(PendingSculptPreparation {
            layer_id,
            topology_id,
            cancel,
            receiver,
            thread,
        });
        false
    }

    pub(crate) fn poll_preparation(&mut self) -> Option<Result<SculptSession, String>> {
        self.reap_finished_preparations();
        let pending = self.pending.take()?;
        match pending.receiver.try_recv() {
            Ok(result) => {
                let _ = pending.thread.join();
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.pending = Some(pending);
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let _ = pending.thread.join();
                Some(Err(
                    "sculpt preparation worker stopped unexpectedly".to_string()
                ))
            }
        }
    }

    fn cancel_pending_preparation(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.cancel.store(true, Ordering::Relaxed);
            self.retired_preparations.push(pending.thread);
        }
        self.reap_finished_preparations();
    }

    fn reap_finished_preparations(&mut self) {
        let mut active = Vec::with_capacity(self.retired_preparations.len());
        for worker in self.retired_preparations.drain(..) {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                active.push(worker);
            }
        }
        self.retired_preparations = active;
    }
}

impl Drop for SculptTool {
    fn drop(&mut self) {
        self.cancel_pending_preparation();
        for worker in self.retired_preparations.drain(..) {
            let _ = worker.join();
        }
    }
}

/// A prepared kernel session over one layer, transferred into the worker.
pub(crate) struct SculptSession {
    /// The sculpted layer's stable identity.
    pub(crate) layer_id: SceneMeshId,
    /// The layer mesh's topology identity when the session was prepared;
    /// re-prepared if it ever changes (any non-sculpt edit, or an undo).
    pub(crate) topology_id: u64,
    /// The geometry kernel.
    pub(crate) session: BrushSession,
    /// Immutable mesh template used to build completed meshes. It is cloned
    /// on the preparation thread, never in the UI commit path.
    pub(crate) base_mesh: Arc<Mesh>,
    /// Display copy of the layer's vertex array, patched per dab from the
    /// kernel and streamed into the prepared GPU vertex buffer for live
    /// feedback; also the source of the final committed mesh.
    pub(crate) shadow: Arc<RwLock<Vec<Vertex>>>,
    /// GPU topology identity of the mesh being sculpted — routes the live
    /// sparse vertex write to the right prepared-scene entry.
    pub(crate) topology: PreparedSceneTopology,
    /// World → mesh-local transform for dab centers and the view direction.
    pub(crate) world_to_local: Affine3A,
    /// Mesh-local mm per world mm (1 / uniform scale), to convert the world
    /// brush radius into the kernel's local units.
    pub(crate) local_per_world: f32,
    /// Whether any dab in the CURRENT stroke actually moved geometry — a stroke
    /// that never touched the surface must not create an undo entry.
    pub(crate) dirty_stroke: bool,
    /// The layer mesh as it stood before the current stroke — the undo
    /// baseline, built off the UI thread. It is a whole MESH, not just
    /// positions: a densifying stroke changes the triangle list too, and undo
    /// has to put the coarse topology back, not just the old coordinates.
    pub(crate) stroke_start_mesh: Option<Arc<Mesh>>,
}

/// A mid-stroke topology change: Smooth densified the surface, so the layer's
/// whole geometry has to be replaced and the GPU scene re-prepared. The mesh
/// carries a FRESH `topology_id`, which is what makes the renderer drop its
/// exactly-sized buffers instead of streaming into buffers that are now too
/// small.
pub(crate) struct SculptRebuild {
    /// The layer's new geometry, ready to swap into the scene.
    pub(crate) mesh: Mesh,
    /// Its GPU topology token, which the worker adopts for later sparse writes.
    pub(crate) topology: PreparedSceneTopology,
}

/// Read-mostly surface state used by the interactive Sculpt raycast. The
/// original mesh owns a stable BVH; the shadow contains the current live
/// vertices, and only triangles touched since the last refit are checked
/// outside that tree.
pub(crate) struct SculptPickState {
    pub(crate) mesh: Arc<Mesh>,
    pub(crate) shadow: Arc<RwLock<Vec<Vertex>>>,
    pub(crate) dirty_triangles: Vec<usize>,
}

/// What one dab produced: either a sparse vertex update, or a whole-layer
/// rebuild when densification changed the topology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DabFailure {
    /// The live display shadow could not be updated. The kernel result is no
    /// longer safe to publish because the worker would otherwise stream stale
    /// vertices and later commit an undo state that never matched the view.
    ShadowPoisoned,
    /// The display shadow no longer has the same shape as the kernel mesh.
    /// Publishing any subset would make the GPU and the undo baseline disagree.
    ShadowShapeMismatch {
        shadow_count: usize,
        live_count: usize,
    },
    /// The kernel returned an id outside its prepared vertex array.
    InvalidVertexIndex {
        vertex_id: usize,
        vertex_count: usize,
    },
    /// A densifying dab changed the kernel topology, but its authoritative
    /// scene mesh could not be rebuilt.
    TopologyRebuild { detail: String },
}

#[derive(Default)]
pub(crate) struct DabOutcome {
    /// Vertex ids whose position or normal changed, for a sparse GPU write.
    /// Empty when `rebuild` is set — the rebuild supersedes it.
    pub(crate) touched: Vec<usize>,
    /// Triangles that must be considered against the live shadow by the
    /// interactive raycast.
    pub(crate) dirty_triangles: Vec<usize>,
    /// Set when this dab grew the mesh.
    pub(crate) rebuild: Option<SculptRebuild>,
    /// Set when the kernel result cannot be published safely. This must abort
    /// the worker; treating it as an empty dab would leave the GPU or undo
    /// history on a stale state.
    pub(crate) failure: Option<DabFailure>,
}

impl SculptSession {
    /// Apply one dab (already built in mesh-local space) and return either the
    /// touched vertex ids — patched into the display shadow — or a whole-layer
    /// rebuild when densification changed the topology. Marks the current
    /// stroke dirty so a stroke that actually changed geometry gets an undo
    /// entry (an empty dab does not).
    #[cfg(test)]
    pub(crate) fn apply_dab(&mut self, stroke: BrushStroke, mode: BrushMode) -> DabOutcome {
        self.apply_dab_inner(stroke, mode, None).unwrap_or_default()
    }

    /// Cancellable worker variant. A cancellation never returns a dab outcome:
    /// the owning worker is being torn down, so its potentially partial session
    /// and shadow must be discarded together.
    pub(crate) fn apply_dab_cancellable(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        cancel: &AtomicBool,
    ) -> Option<DabOutcome> {
        self.apply_dab_inner(stroke, mode, Some(cancel))
    }

    fn apply_dab_inner(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        cancel: Option<&AtomicBool>,
    ) -> Option<DabOutcome> {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return None;
        }
        if self.stroke_start_mesh.is_none() {
            match self.snapshot_mesh() {
                Ok(snapshot) => self.stroke_start_mesh = Some(snapshot),
                Err(failure) => {
                    return Some(DabOutcome {
                        failure: Some(failure),
                        ..DabOutcome::default()
                    });
                }
            }
        }
        let outcome = match cancel {
            Some(cancel) => self
                .session
                .apply_stroke_cancellable(stroke, mode, cancel)?,
            None => self.session.apply_stroke(stroke, mode),
        };
        if outcome.topology_changed() {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return None;
            }
            // Vertex ids the caller already knows stay valid — densification
            // only APPENDS — but the array grew and the triangle list changed,
            // so a sparse write into the old buffers would be a corruption.
            // Hand back the rebuilt layer instead and drop this dab's ids.
            return match self.rebuild_after_densify(cancel) {
                Ok(Some(rebuild)) => Some(DabOutcome {
                    touched: Vec::new(),
                    dirty_triangles: Vec::new(),
                    rebuild: Some(rebuild),
                    failure: None,
                }),
                Ok(None) => None,
                Err(failure) => Some(DabOutcome {
                    touched: Vec::new(),
                    dirty_triangles: Vec::new(),
                    rebuild: None,
                    failure: Some(failure),
                }),
            };
        }
        if outcome.touched_vertices.is_empty() {
            return Some(DabOutcome::default());
        }
        let normal_vertices = outcome.normal_vertices;
        let dirty_triangles = outcome.dirty_triangles;
        if self
            .patch_shadow(&outcome.touched_vertices, &normal_vertices)
            .is_err()
        {
            return Some(DabOutcome {
                touched: Vec::new(),
                dirty_triangles: Vec::new(),
                rebuild: None,
                failure: Some(DabFailure::ShadowPoisoned),
            });
        }
        self.dirty_stroke = true;
        let mut touched = outcome.touched_vertices;
        touched.extend(normal_vertices);
        Some(DabOutcome {
            touched,
            dirty_triangles,
            rebuild: None,
            failure: None,
        })
    }

    /// The layer mesh as the session currently holds it (template + shadow).
    ///
    /// This cold undo baseline defers derived-cache work until restoration.
    fn snapshot_mesh(&self) -> Result<Arc<Mesh>, DabFailure> {
        let shadow = self.shadow.read().map_err(|_| DabFailure::ShadowPoisoned)?;
        let shadow_count = shadow.len();
        let live_count = self.session.vertices().len();
        if shadow_count != live_count || self.base_mesh.vertices().len() != live_count {
            return Err(DabFailure::ShadowShapeMismatch {
                shadow_count,
                live_count,
            });
        }
        self.base_mesh
            .with_sculpted_vertices_uncached(shadow.clone())
            .map(Arc::new)
            .ok_or(DabFailure::ShadowShapeMismatch {
                shadow_count,
                live_count: self.base_mesh.vertices().len(),
            })
    }

    /// Adopt the densified geometry: rebuild the template mesh, resize the
    /// display shadow to match, and take on the new GPU topology token so the
    /// dabs that follow can stream sparsely again.
    fn rebuild_after_densify(
        &mut self,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<SculptRebuild>, DabFailure> {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(None);
        }
        let mesh =
            mesh_from_sculpt_session_like(&self.base_mesh, &self.session).map_err(|error| {
                DabFailure::TopologyRebuild {
                    detail: format!("sculpt topology rebuild failed: {error}"),
                }
            })?;
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(None);
        }
        // Pick-ready before it ships. This mesh replaces the layer in the
        // scene, and the viewport lays a dab only where the cursor HITS the
        // surface — a hit test that refuses to build a scan-sized BVH on the
        // egui thread, by design. Preparation was the only thing that ever
        // warmed one, and a rebuilt layer never re-prepares (the session still
        // matches — that is the point of the rebuild). Shipped cold, the layer
        // was unhittable, so after the first densifying stroke the brush went
        // dead for good. This runs on the worker thread, where an O(n) rebuild
        // has already been paid; a clone shares the warmed tree, so the commit
        // path's refit keeps it alive from here on.
        //
        // The same cancellation rule as preparation applies: this build cannot
        // be interrupted, so do not start it for a stroke the operator has
        // already abandoned. The pick path warms the tree itself when it needs
        // one.
        if !cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            mesh.warm_bvh();
        }
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(None);
        }
        {
            let mut shadow = self
                .shadow
                .write()
                .map_err(|_| DabFailure::ShadowPoisoned)?;
            *shadow = mesh.vertices().to_vec();
        }
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(None);
        }
        let topology = PreparedSceneTopology::from_mesh(&mesh);
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Ok(None);
        }
        self.base_mesh = Arc::new(mesh.clone());
        self.topology = topology;
        self.dirty_stroke = true;
        Ok(Some(SculptRebuild { mesh, topology }))
    }

    /// Copy the kernel's live position and normal for every touched vertex id
    /// into the display shadow. Color and UV are preserved untouched, so
    /// textured/colored scans keep their look while being sculpted.
    pub(crate) fn patch_shadow(
        &mut self,
        moved: &[usize],
        normal_vertices: &[usize],
    ) -> Result<(), DabFailure> {
        let mut shadow = self
            .shadow
            .write()
            .map_err(|_| DabFailure::ShadowPoisoned)?;
        let live = self.session.vertices();
        let shadow_count = shadow.len();
        let live_count = live.len();
        if shadow_count != live_count {
            return Err(DabFailure::ShadowShapeMismatch {
                shadow_count,
                live_count,
            });
        }
        if let Some(vertex_id) = moved
            .iter()
            .chain(normal_vertices.iter())
            .copied()
            .find(|&vertex_id| vertex_id >= live_count)
        {
            return Err(DabFailure::InvalidVertexIndex {
                vertex_id,
                vertex_count: live_count,
            });
        }
        for &vertex_id in moved {
            let source = live[vertex_id];
            let target = &mut shadow[vertex_id];
            target.position = source.position;
            // The kernel normally includes moved vertices in its normal scope.
            // Copying this here as well keeps the position update self-contained
            // if a future kernel mode reports a narrower normal scope.
            target.normal = source.normal;
        }
        for &vertex_id in normal_vertices {
            shadow[vertex_id].normal = live[vertex_id].normal;
        }
        Ok(())
    }
}

/// Bookkeeping for one live drag (button held): the last dab position and the
/// stationary-hold timer that pace the arc-length dab scheduler.
pub(crate) struct StrokeState {
    /// The layer this drag started on; dabs that land on another layer are
    /// ignored so a drag never bleeds across arches.
    pub(crate) layer_id: SceneMeshId,
    /// Mesh-local position of the last laid dab, or `None` before the first.
    pub(crate) last_dab_local: Option<Vec3>,
    /// Seconds accumulated since the last dab while (near) stationary.
    pub(crate) hold_seconds: f32,
}

/// Mean scale of a scene transform's linear part — converts the on-model mm
/// brush radius into mesh-local units. Scene placements are rigid in practice
/// (scale 1), so this is a defensive average, never zero.
pub(crate) fn mean_uniform_scale(transform: &Affine3A) -> f32 {
    let m = transform.matrix3;
    let mean = (m.x_axis.length() + m.y_axis.length() + m.z_axis.length()) / 3.0;
    if mean.is_finite() && mean > f32::EPSILON {
        mean
    } else {
        1.0
    }
}

/// Return the one scalar that converts world millimetres into local
/// millimetres, but only when the linear transform really has an isotropic
/// positive scale. Sculpting under non-uniform scale or shear would require a
/// full inverse metric (and an elliptical brush footprint), not an average.
pub(crate) fn uniform_scene_scale(transform: &Affine3A) -> Option<f32> {
    const RELATIVE_TOLERANCE: f32 = 1.0e-4;
    let m = transform.matrix3;
    let axes = [m.x_axis, m.y_axis, m.z_axis];
    let lengths = axes.map(glam::Vec3A::length);
    let max = lengths.iter().copied().fold(0.0_f32, f32::max);
    let min = lengths.iter().copied().fold(f32::INFINITY, f32::min);
    if !max.is_finite()
        || max <= f32::EPSILON
        || !min.is_finite()
        || (max - min) > max * RELATIVE_TOLERANCE
        || !m.determinant().is_finite()
        || m.determinant() <= f32::EPSILON
    {
        return None;
    }
    for (index, axis) in axes.iter().enumerate() {
        for other in axes.iter().skip(index + 1) {
            if axis.dot(*other).abs() > max * max * RELATIVE_TOLERANCE {
                return None;
            }
        }
    }
    Some((lengths[0] + lengths[1] + lengths[2]) / 3.0)
}

#[cfg(test)]
#[path = "sculpt_tool_tests.rs"]
mod tests;

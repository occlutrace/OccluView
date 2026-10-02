//! State for interactive sculpt brushes.

use crate::sculpt::sculpt_kernel::BrushSession;
use crate::sculpt::sculpt_kernel::{BrushMode, BrushRayStep};
#[cfg(test)]
use crate::sculpt::sculpt_kernel::{BrushStroke, DabDose};
use crate::sculpt::sculpt_worker::SculptWorker;
use eframe::egui;
use glam::Affine3A;
use occluview_core::{Mesh, Scene, SceneMeshId, Vertex};
use occluview_edit::mesh_edit_buffers_from_mesh;
use occluview_render::{PreparedSceneTopology, SculptTopologyDelta};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;

/// Shift Smooth in the donor doubles strength only; brush footprint stays fixed.
/// How much Shift amplifies the selected Smooth strength.
const SHIFT_STRENGTH_GAIN: f32 = 2.0;
const RADIUS_DETENT_RATIO: f64 = 1.2;
const STRENGTH_DETENT_RATIO: f64 = 1.3;
pub(crate) const HOLD_DAB_INTERVAL_SEC: f32 = 0.03;
/// Bound caller-side retries while the worker's smaller queue is saturated.
pub(crate) const MAX_RETAINED_SCULPT_SAMPLES: usize = 64;
/// The available sculpt tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SculptToolKind {
    /// Shape material, or carve it away with Shift held.
    #[default]
    AddRemove,
    /// Relaxer: flatten/even the surface; Shift doubles the selected strength.
    Smooth,
}

impl SculptToolKind {
    /// Catalog strength range/default for this mode, in the kernel's 0..1
    /// units. The slider and persisted values use the same physical scale.
    pub(crate) const fn strength_range(self) -> (f32, f32) {
        match self {
            Self::AddRemove => (0.05, 1.0),
            Self::Smooth => (0.01, 1.0),
        }
    }

    pub(crate) const fn default_strength(self) -> f32 {
        match self {
            Self::AddRemove => 0.35,
            Self::Smooth => 0.15,
        }
    }

    pub(crate) const fn strength_step(self) -> f32 {
        match self {
            Self::AddRemove => 0.05,
            Self::Smooth => 0.01,
        }
    }

    /// Match the donor wheel detent: a 1.3 ratio snapped to the slider step,
    /// with at least one step of progress at the range boundary.
    pub(crate) fn step_strength(self, current: f32, direction: f32) -> f32 {
        let (min, max) = self.strength_range();
        ratio_detent(
            current,
            DetentCatalog {
                min,
                max,
                default: self.default_strength(),
                step: self.strength_step(),
            },
            direction,
            STRENGTH_DETENT_RATIO,
        )
    }

    /// Resolve a dab's mode from the active tool and held modifiers.
    pub(crate) fn brush_mode(self, shift: bool, command: bool) -> BrushMode {
        match self {
            Self::AddRemove if shift && command => BrushMode::Relax,
            Self::AddRemove if shift => BrushMode::Remove,
            Self::AddRemove => BrushMode::Add,
            Self::Smooth => BrushMode::Smooth,
        }
    }

    /// Return the kernel strength for one dab.
    ///
    /// Shift strengthens a Smooth brush rather than replacing the slider with a
    /// maximum pass. The reference tool's Shift amplifies the setting the
    /// operator chose; a jump to the top of the range flattened whole cusps
    /// from the bottom of the slider and left the surface rippled.
    pub(crate) fn dab_strength(self, intensity01: f32, shift: bool) -> f32 {
        let strength = intensity01.clamp(0.0, 1.0);
        match self {
            Self::Smooth if shift => (strength * SHIFT_STRENGTH_GAIN).clamp(0.0, 1.0),
            _ => strength,
        }
    }
}

/// One catalog wheel detent. Mirror the reference's decimal snap before
/// clamping so repeated notches do not accumulate binary-float drift.
#[derive(Clone, Copy)]
struct DetentCatalog {
    min: f32,
    max: f32,
    default: f32,
    step: f32,
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "The value is clamped to the catalog's f32 bounds before conversion."
)]
fn ratio_detent(value: f32, catalog: DetentCatalog, direction: f32, ratio: f64) -> f32 {
    let DetentCatalog {
        min,
        max,
        default,
        step,
    } = catalog;
    // Slider values and step sizes are authored as decimal catalog values.
    // Canonicalize f32 representation noise at the same 1e-6 precision as the
    // reference before doing ratio and step arithmetic in f64.
    let catalog_decimal = |value: f32| (f64::from(value) * 1_000_000.0).round() / 1_000_000.0;
    let min = catalog_decimal(min);
    let max = catalog_decimal(max);
    let step = catalog_decimal(step);
    let current = if value.is_finite() {
        catalog_decimal(value).clamp(min, max)
    } else {
        catalog_decimal(default).clamp(min, max)
    };
    let snap = |candidate: f64| {
        let stepped = (candidate / step).round() * step;
        (stepped * 1_000_000.0).round() / 1_000_000.0
    };
    let scaled = snap(if direction > 0.0 {
        current * ratio
    } else {
        current / ratio
    });
    let minimum_move = snap(if direction > 0.0 {
        current + step
    } else {
        current - step
    });
    let next = if direction > 0.0 {
        minimum_move.max(scaled)
    } else {
        minimum_move.min(scaled)
    };
    next.clamp(min, max) as f32
}

/// The brush tip a dab is stamped with. The wire discriminants are the
/// kernel's own, so the UI, the worker and the display agree on one number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SculptTip {
    /// Spherical falloff: the all-round shaping stamp.
    #[default]
    Ball,
    /// Narrow, travel-aligned blade with a blended transverse shoulder.
    Knife,
    /// Flat plateau with a soft rim, for levelling one face.
    Cylinder,
}

impl SculptTip {
    /// Every tip, in the order the Sculpt panel offers them.
    pub(crate) const ALL: [Self; 3] = [Self::Ball, Self::Knife, Self::Cylinder];

    /// Catalog radius range in physical millimetres.
    pub(crate) const fn radius_range_mm(self) -> (f32, f32) {
        match self {
            Self::Ball => (0.25, 4.0),
            Self::Knife => (0.25, 2.5),
            Self::Cylinder => (0.25, 2.0),
        }
    }

    pub(crate) const fn default_radius_mm(self) -> f32 {
        match self {
            Self::Ball => 0.75,
            Self::Knife | Self::Cylinder => 0.5,
        }
    }

    pub(crate) const fn radius_step_mm() -> f32 {
        0.05
    }

    /// Match the donor wheel detent in the selected tip's physical range.
    pub(crate) fn step_radius_mm(self, current: f32, direction: f32) -> f32 {
        let (min, max) = self.radius_range_mm();
        ratio_detent(
            current,
            DetentCatalog {
                min,
                max,
                default: self.default_radius_mm(),
                step: Self::radius_step_mm(),
            },
            direction,
            RADIUS_DETENT_RATIO,
        )
    }

    /// The kernel's tip discriminant.
    pub(crate) fn kernel_stamp(self) -> u32 {
        match self {
            Self::Ball => 0,
            Self::Knife => 1,
            Self::Cylinder => 2,
        }
    }

    /// Localization key for the tip's name.
    pub(crate) fn label_key(self) -> crate::i18n::MessageId {
        match self {
            Self::Ball => crate::i18n::message_id!("meshedit-sculpt-tip-ball"),
            Self::Knife => crate::i18n::message_id!("meshedit-sculpt-tip-knife"),
            Self::Cylinder => crate::i18n::message_id!("meshedit-sculpt-tip-cylinder"),
        }
    }

    /// Localization key for the tip's explanation.
    pub(crate) fn hint_key(self) -> crate::i18n::MessageId {
        match self {
            Self::Ball => crate::i18n::message_id!("meshedit-sculpt-tip-ball-hint"),
            Self::Knife => crate::i18n::message_id!("meshedit-sculpt-tip-knife-hint"),
            Self::Cylinder => crate::i18n::message_id!("meshedit-sculpt-tip-cylinder-hint"),
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
    /// Presses that arrived while preparation or a previous stroke was
    /// finishing. Their rays and brush settings are captured on the input edge
    /// and completed gestures stay FIFO until the worker can accept them.
    pub(crate) pending_presses: VecDeque<PendingSculptPress>,
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
    /// Last completed raw modifier state, used to replay same-frame pointer
    /// movements around a `ModifiersChanged` event in the correct order.
    pub(crate) pointer_modifiers: egui::Modifiers,
    pending: Option<PendingSculptPreparation>,
    /// Canceled preparation workers are reaped without blocking the UI. They
    /// remain owned here until their non-cancellable kernel phase finishes,
    /// so dropping a receiver never leaves an untracked CPU/RAM worker behind.
    retired_preparations: Vec<thread::JoinHandle<()>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PendingSculptPress {
    pub(crate) layer_id: SceneMeshId,
    pub(crate) topology_id: u64,
    pub(crate) world_to_local: Affine3A,
    pub(crate) local_per_world: f32,
    pub(crate) press_pointer: [f32; 2],
    pub(crate) latest_pointer: [f32; 2],
    pub(crate) start_step: BrushRayStep,
    pub(crate) latest_step: BrushRayStep,
    pub(crate) moved: bool,
    /// An overlay or an offscreen interval was crossed after this press.
    /// The endpoint must start a fresh kernel path, without closing history.
    pub(crate) break_before_latest: bool,
    pub(crate) released: bool,
}

struct PendingSculptPreparation {
    layer_id: SceneMeshId,
    topology_id: u64,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<SculptSession, String>>,
    thread: thread::JoinHandle<()>,
}

impl SculptTool {
    /// Toggle a tool. A mode switch ends the current drag but keeps the
    /// prepared session for the active layer.
    pub(crate) fn toggle(&mut self, kind: SculptToolKind) {
        self.stroke = None;
        self.pending_presses.clear();
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
        self.pending_presses.clear();
        self.clear_cursor_hit();
        self.finish_requested = false;
        self.finish_retry = false;
        self.pending_history = None;
        self.worker = None;
        self.cancel_pending_preparation();
    }

    /// Drop the prepared session and live stroke while keeping the tool armed.
    /// Re-prepare after a scene change even when the topology id is unchanged.
    pub(crate) fn invalidate_session(&mut self) {
        self.stroke = None;
        self.pending_presses.clear();
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
            || !self.pending_presses.is_empty()
            || self.pending.is_some()
            || self.worker_has_pending_work()
            || self.finish_requested
            || self.finish_retry
            || self.pending_history.is_some()
    }

    pub(crate) fn preparation_in_progress(&self) -> bool {
        self.pending.is_some() || !self.retired_preparations.is_empty()
    }

    /// Keep the UI-side backlog in order. A later click never replaces an
    /// accepted press while a scan-sized session is still preparing.
    pub(crate) fn queue_pending_press(&mut self, press: PendingSculptPress) -> bool {
        const MAX_PENDING_SCULPT_PRESSES: usize = 64;
        if self.pending_presses.len() >= MAX_PENDING_SCULPT_PRESSES {
            return false;
        }
        self.pending_presses.push_back(press);
        true
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
        // A scalar brush radius is valid only for a rigid or uniform-scale
        // transform.
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
        // Do not overlap scan-sized preparations while a cancelled worker is
        // still finishing its non-cancellable phase.
        if !self.retired_preparations.is_empty() {
            return false;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);

        // Pass only the target mesh to the preparation worker. Keeping an
        // `Arc<Scene>` there would conflict with in-place scene edits.
        let mesh = entry.mesh.clone();
        let transform = entry.transform;
        drop(scene);

        let spawned = thread::Builder::new()
            .name("occluview-sculpt-prepare".to_string())
            .spawn(move || {
                if worker_cancel.load(Ordering::Relaxed) {
                    return;
                }
                // The BVH build cannot be interrupted once started. Check
                // cancellation before entering it and let picking build it on
                // demand when preparation is no longer needed.
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
                        topology_dirty_stroke: false,
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
    /// Whether the current stroke changed geometry.
    pub(crate) dirty_stroke: bool,
    /// Whether the current stroke changed connectivity.
    pub(crate) topology_dirty_stroke: bool,
    /// The complete pre-stroke mesh used by Undo, including topology.
    pub(crate) stroke_start_mesh: Option<Arc<Mesh>>,
}

/// Read-mostly surface state used by the interactive Sculpt raycast. The
/// original mesh owns a stable BVH; the shadow contains the current live
/// vertices, and only triangles touched since the last refit are checked
/// outside that tree.
pub(crate) struct SculptPickState {
    pub(crate) mesh: Arc<Mesh>,
    pub(crate) shadow: Arc<RwLock<Vec<Vertex>>>,
    pub(crate) dirty_triangles: Vec<usize>,
    /// Current live triangle rows, including appended and rewired faces.
    pub(crate) indices: Vec<u32>,
}

/// What one dab produced for the prepared GPU entry.
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
}

#[derive(Default)]
pub(crate) struct DabOutcome {
    /// Vertex ids whose position or normal changed, for a sparse GPU write.
    /// Empty when the dab made no displayable change.
    pub(crate) touched: Vec<usize>,
    /// Triangles that must be considered against the live shadow by the
    /// interactive raycast.
    pub(crate) dirty_triangles: Vec<usize>,
    /// Vertex and face rows changed by this dab's remesh.
    pub(crate) topology_delta: Option<SculptTopologyDelta>,
    /// Set when the kernel result cannot be published safely. This must abort
    /// the worker; treating it as an empty dab would leave the GPU or undo
    /// history on a stale state.
    pub(crate) failure: Option<DabFailure>,
}

impl SculptSession {
    /// Apply one dab (already built in mesh-local space) and return touched
    /// vertex ids plus any local topology delta. Marks the current
    /// stroke dirty so a stroke that actually changed geometry gets an undo
    /// entry (an empty dab does not).
    #[cfg(test)]
    pub(crate) fn apply_dab(&mut self, stroke: BrushStroke, mode: BrushMode) -> DabOutcome {
        self.apply_dab_inner(stroke, mode, None, SculptTip::Ball, None, DabDose::FULL)
            .unwrap_or_default()
    }

    /// Test-only: one dab with an explicit tip and stroke bearing.
    #[cfg(test)]
    pub(crate) fn apply_dab_tipped(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
    ) -> DabOutcome {
        self.apply_dab_inner(stroke, mode, None, tip, axis, DabDose::FULL)
            .unwrap_or_default()
    }

    /// Cancellable worker variant. A cancellation never returns a dab outcome:
    /// the owning worker is being torn down, so its potentially partial session
    /// and shadow must be discarded together.
    // The cancellation flag rides beside the dab's own arguments.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn apply_dab_cancellable(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        cancel: &AtomicBool,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
    ) -> Option<DabOutcome> {
        self.apply_dab_inner(stroke, mode, Some(cancel), tip, axis, dose)
    }

    // The cancellation flag rides beside the dab's own arguments.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    fn apply_dab_inner(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        cancel: Option<&AtomicBool>,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
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
                .apply_stroke_cancellable_dosed(stroke, mode, tip, axis, dose, cancel)?,
            None => self
                .session
                .apply_stroke_dosed(stroke, mode, tip, axis, dose),
        };
        let topology_changed = outcome.topology_changed();
        if outcome.touched_vertices.is_empty() && !topology_changed {
            return Some(DabOutcome::default());
        }
        let dirty_triangles = outcome.dirty_triangles;
        // The kernel reports every vertex whose position or normal changed in
        // one list, so there is no separate normal-only scope to patch.
        if self
            .patch_shadow(
                &outcome.touched_vertices,
                &[],
                outcome.topology_delta.as_ref(),
            )
            .is_err()
        {
            return Some(DabOutcome {
                touched: Vec::new(),
                dirty_triangles: Vec::new(),
                topology_delta: None,
                failure: Some(DabFailure::ShadowPoisoned),
            });
        }
        self.dirty_stroke = true;
        self.topology_dirty_stroke |= topology_changed;
        let touched = outcome.touched_vertices;
        Some(DabOutcome {
            touched,
            dirty_triangles,
            topology_delta: outcome.topology_delta,
            failure: None,
        })
    }

    /// Execute one real viewport ray sample on the worker. The kernel traces
    /// the segment from its previous sample and returns the complete moved
    /// vertex and topology slice for that step.
    pub(crate) fn apply_ray_step_cancellable(
        &mut self,
        step: &BrushRayStep,
        elapsed_ms: f64,
        cancel: &AtomicBool,
    ) -> Option<DabOutcome> {
        if cancel.load(Ordering::Relaxed) {
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
        let outcome = self
            .session
            .apply_ray_step_cancellable(step, elapsed_ms, cancel)?;
        let topology_changed = outcome.topology_changed();
        if outcome.touched_vertices.is_empty() && !topology_changed {
            return Some(DabOutcome::default());
        }
        let dirty_triangles = outcome.dirty_triangles;
        if self
            .patch_shadow(
                &outcome.touched_vertices,
                &[],
                outcome.topology_delta.as_ref(),
            )
            .is_err()
        {
            return Some(DabOutcome {
                touched: Vec::new(),
                dirty_triangles: Vec::new(),
                topology_delta: None,
                failure: Some(DabFailure::ShadowPoisoned),
            });
        }
        self.dirty_stroke = true;
        self.topology_dirty_stroke |= topology_changed;
        Some(DabOutcome {
            touched: outcome.touched_vertices,
            dirty_triangles,
            topology_delta: outcome.topology_delta,
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

    /// Copy the kernel's live position and normal for every touched vertex id
    /// into the display shadow. Color and UV are preserved untouched, so
    /// textured/colored scans keep their look while being sculpted.
    pub(crate) fn patch_shadow(
        &mut self,
        moved: &[usize],
        normal_vertices: &[usize],
        topology_delta: Option<&SculptTopologyDelta>,
    ) -> Result<(), DabFailure> {
        let mut shadow = self
            .shadow
            .write()
            .map_err(|_| DabFailure::ShadowPoisoned)?;
        let live = self.session.vertices();
        let shadow_count = shadow.len();
        let live_count = live.len();
        let append_count_matches = topology_delta.is_some_and(|delta| {
            delta.base_vertex_count == shadow_count
                && delta
                    .base_vertex_count
                    .checked_add(delta.appended_vertices.len())
                    == Some(live_count)
        });
        if (topology_delta.is_some() && !append_count_matches)
            || (topology_delta.is_none() && shadow_count != live_count)
        {
            return Err(DabFailure::ShadowShapeMismatch {
                shadow_count,
                live_count,
            });
        }
        if topology_delta.is_some() {
            shadow.extend(
                live[shadow_count..]
                    .iter()
                    .copied()
                    .map(crate::sculpt::sculpt_kernel::vertex_from_edit_vertex),
            );
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
            // if a kernel mode reports a narrower normal scope.
            target.normal = source.normal;
        }
        for &vertex_id in normal_vertices {
            shadow[vertex_id].normal = live[vertex_id].normal;
        }
        Ok(())
    }
}

/// Bookkeeping for one live drag (button held): the last submitted viewport
/// ray and the stationary-hold timer used to report dwell to the kernel.
pub(crate) struct StrokeState {
    /// The layer this drag started on; dabs that land on another layer are
    /// ignored so a drag never bleeds across arches.
    pub(crate) layer_id: SceneMeshId,
    /// Latest screen coordinate whose ray the worker accepted.
    pub(crate) last_pointer: [f32; 2],
    /// Latest pointer captured into the worker queue or bounded retry FIFO.
    pub(crate) input_pointer: [f32; 2],
    /// Last ray captured by the input layer. The worker owns authoritative
    /// path continuity; this also drives the cursor while samples are queued.
    pub(crate) last_ray: Option<BrushRayStep>,
    /// Seconds accumulated since the last dab while (near) stationary.
    pub(crate) hold_seconds: f32,
    /// The pointer crossed a region whose samples are not owned by Sculpt.
    /// Queue an ordered kernel path break before the next accepted ray.
    pub(crate) path_break_pending: bool,
    /// The physical release arrived; retained rays must drain before Finish.
    pub(crate) release_pending: bool,
    /// Ordered samples refused by the worker queue. A path break belongs to
    /// the sample after the gap, so it stays attached to that sample.
    pub(crate) retained_samples: VecDeque<RetainedSculptSample>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RetainedSculptSample {
    pub(crate) step: BrushRayStep,
    pub(crate) pointer: [f32; 2],
    pub(crate) break_before: bool,
}

impl StrokeState {
    /// Keep one refused sample without advancing the worker-accepted pointer.
    /// Compatible travel tails collapse to their newest endpoint. Repeated
    /// held samples at the same ray also collapse; worker dispatch timing
    /// determines their dose after they leave this bounded input FIFO.
    pub(crate) fn retain_sample(&mut self, sample: RetainedSculptSample) -> bool {
        if let Some(previous) = self.retained_samples.back_mut() {
            let both_travel = !previous.step.hold && !sample.step.hold;
            let same_stationary_ray = previous.step.hold
                && sample.step.hold
                && crate::sculpt::sculpt_worker::same_ray(&previous.step, &sample.step);
            if !previous.break_before
                && !sample.break_before
                && crate::sculpt::sculpt_worker::same_brush_and_visibility(
                    &previous.step,
                    &sample.step,
                )
                && (both_travel || same_stationary_ray)
            {
                let input_pointer = sample.pointer;
                *previous = sample;
                self.input_pointer = input_pointer;
                return true;
            }
        }
        if self.retained_samples.len() >= MAX_RETAINED_SCULPT_SAMPLES {
            return false;
        }
        self.input_pointer = sample.pointer;
        self.retained_samples.push_back(sample);
        true
    }
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

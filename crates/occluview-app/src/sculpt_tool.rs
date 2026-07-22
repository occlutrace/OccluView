//! Interactive sculpt-brush state: exocad-style freeforming (an Add/Remove clay
//! knife and a Smooth relaxer) dragged directly on a scan surface inside the
//! Mesh Editor. This module owns the pure state and math — which tool is armed,
//! the size/intensity sliders and their unit conversions, the persistent
//! per-layer kernel session, and the per-drag dab scheduler — while the
//! egui/viewport glue lives in `app::app_sculpt` and the geometry kernel is
//! [`occluview_core::BrushSession`].

use crate::sculpt_worker::SculptWorker;
use glam::{Affine3A, Vec3};
use occluview_core::{
    mesh_edit_buffers_from_mesh, BrushMode, BrushSession, BrushStroke, Scene, SceneMeshId, Vertex,
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
/// One notch of the mouse wheel changes a slider by this many units.
pub(crate) const SCULPT_WHEEL_STEP: f32 = 6.0;
/// Dab spacing along the drag path, as a fraction of the brush radius: dabs are
/// laid down every `radius * this` of cursor travel so buildup is even and
/// framerate-independent (Blender's arc-length stroke spacing).
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
    /// Add/Remove and Smooth; Shift doubles Smooth force, capped at 100%.
    pub(crate) fn dab_strength(self, intensity01: f32, shift: bool) -> f32 {
        match self {
            Self::Smooth if shift => (intensity01.clamp(0.0, 1.0) * 2.0).min(1.0),
            _ => intensity01.clamp(0.0, 1.0),
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
    /// Primary was pressed over the viewport while BVH/brush preparation was
    /// still completing. The current pointer is retried on the next frame; a
    /// quick press-and-release is discarded rather than applied late.
    pub(crate) press_pending: bool,
    /// A mesh-edit Done action waits for the background commit before closing
    /// the edit session, so a fast click cannot discard a valid stroke.
    pub(crate) finish_requested: bool,
    /// Undo/redo waits for an asynchronous sculpt completion before swapping
    /// an older scene over the worker's current shadow.
    pub(crate) pending_history: Option<bool>,
    pending: Option<PendingSculptPreparation>,
}

struct PendingSculptPreparation {
    layer_id: SceneMeshId,
    topology_id: u64,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<SculptSession, String>>,
}

impl SculptTool {
    /// Toggle `kind`: arming it takes over from any other tool; clicking the
    /// armed tool again disarms sculpting. Never drops the prepared session
    /// (same layer, so the next stroke stays instant) but does end any live
    /// drag so a half-applied stroke does not leak between tools.
    pub(crate) fn toggle(&mut self, kind: SculptToolKind) {
        self.stroke = None;
        self.press_pending = false;
        self.armed = if self.armed == Some(kind) {
            None
        } else {
            Some(kind)
        };
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = None;
        self.stroke = None;
        self.press_pending = false;
        self.finish_requested = false;
        self.pending_history = None;
        self.worker = None;
        self.cancel_pending_preparation();
    }

    /// Drop the prepared session and any live stroke while KEEPING the armed
    /// tool. Called whenever the scene geometry changes underneath us (a load,
    /// a delete, another mesh edit, or an undo/redo) — a preserved-`topology_id`
    /// sculpt commit is undone WITHOUT changing the id, so the id alone cannot
    /// tell the geometry reverted; the session must simply be re-prepared from
    /// the fresh scene on the next stroke.
    pub(crate) fn invalidate_session(&mut self) {
        self.stroke = None;
        self.worker = None;
        self.press_pending = false;
        self.finish_requested = false;
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

    /// Queue the O(n) brush preparation. The worker owns the target mesh
    /// snapshot; the UI only stores a receiver and remains responsive while
    /// welding, adjacency construction, and grid setup run.
    pub(crate) fn queue_preparation(&mut self, scene: Arc<Scene>, index: usize) -> bool {
        let Some(entry) = scene.meshes().get(index) else {
            return false;
        };
        if !entry.visible || entry.mesh.is_point_cloud() || entry.mesh.triangle_count() == 0 {
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
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let spawned = thread::Builder::new()
            .name("occluview-sculpt-prepare".to_string())
            .spawn(move || {
                let Some(entry) = scene.meshes().get(index) else {
                    return;
                };
                if worker_cancel.load(Ordering::Relaxed) {
                    return;
                }
                entry.mesh.warm_bvh();
                let buffers = mesh_edit_buffers_from_mesh(&entry.mesh);
                let result = BrushSession::prepare(&buffers)
                    .map_err(|error| error.to_string())
                    .map(|session| {
                        let scale = mean_uniform_scale(&entry.transform);
                        SculptSession {
                            layer_id: entry.id(),
                            topology_id: entry.mesh.topology_id(),
                            session,
                            // Keep only the target mesh in the worker. Holding
                            // the whole Scene Arc made the UI clone the full
                            // scene through Arc::make_mut on every commit.
                            base_mesh: entry.mesh.clone(),
                            shadow: Arc::new(RwLock::new(entry.mesh.vertices().to_vec())),
                            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
                            world_to_local: entry.transform.inverse(),
                            local_per_world: 1.0 / scale,
                            dirty_stroke: false,
                            stroke_start_vertices: None,
                        }
                    });
                if !worker_cancel.load(Ordering::Relaxed) {
                    let _ = sender.send(result);
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.pending = Some(PendingSculptPreparation {
            layer_id,
            topology_id,
            cancel,
            receiver,
        });
        false
    }

    pub(crate) fn poll_preparation(&mut self) -> Option<Result<SculptSession, String>> {
        let pending = self.pending.take()?;
        match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => {
                self.pending = Some(pending);
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "sculpt preparation worker stopped unexpectedly".to_string(),
            )),
        }
    }

    fn cancel_pending_preparation(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.cancel.store(true, Ordering::Relaxed);
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
    pub(crate) base_mesh: occluview_core::Mesh,
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
    /// Vertex positions before the current stroke. The worker turns these
    /// into the undo mesh off the UI thread when the stroke finishes.
    pub(crate) stroke_start_vertices: Option<Vec<Vertex>>,
}

impl SculptSession {
    /// Apply one dab (already built in mesh-local space) and return the touched
    /// vertex ids after patching them into the display shadow. Marks the
    /// current stroke dirty so a stroke that actually moved geometry gets an
    /// undo entry (an empty dab does not).
    pub(crate) fn apply_dab(&mut self, stroke: BrushStroke, mode: BrushMode) -> Vec<usize> {
        if self.stroke_start_vertices.is_none() {
            self.stroke_start_vertices = self.shadow.read().ok().map(|shadow| shadow.clone());
        }
        let outcome = self.session.apply_stroke(stroke, mode);
        if outcome.touched_vertices.is_empty() {
            return Vec::new();
        }
        self.patch_shadow(&outcome.touched_vertices);
        self.dirty_stroke = true;
        outcome.touched_vertices
    }

    /// Copy the kernel's live position and normal for every touched vertex id
    /// into the display shadow. Color and UV are preserved untouched, so
    /// textured/colored scans keep their look while being sculpted.
    pub(crate) fn patch_shadow(&mut self, touched: &[usize]) {
        let Ok(mut shadow) = self.shadow.write() else {
            return;
        };
        let live = self.session.vertices();
        for &vertex_id in touched {
            if let (Some(target), Some(source)) = (shadow.get_mut(vertex_id), live.get(vertex_id)) {
                target.position = source.position;
                target.normal = source.normal;
            }
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]
    use super::*;
    use glam::Quat;
    use occluview_core::{Mesh, SceneMesh};

    #[test]
    fn toggling_a_tool_arms_it_and_toggling_again_disarms() {
        let mut tool = SculptTool::default();
        tool.toggle(SculptToolKind::AddRemove);
        assert_eq!(tool.armed, Some(SculptToolKind::AddRemove));
        tool.toggle(SculptToolKind::Smooth);
        assert_eq!(tool.armed, Some(SculptToolKind::Smooth));
        tool.toggle(SculptToolKind::Smooth);
        assert_eq!(tool.armed, None);
    }

    #[test]
    fn shift_flips_add_to_remove_and_forces_smooth() {
        assert_eq!(SculptToolKind::AddRemove.brush_mode(false), BrushMode::Add);
        assert_eq!(
            SculptToolKind::AddRemove.brush_mode(true),
            BrushMode::Remove
        );
        assert_eq!(SculptToolKind::Smooth.brush_mode(true), BrushMode::Smooth);
        // Shift doubles Smooth force; a light Add/Remove uses the intensity
        // slider unchanged.
        assert_eq!(SculptToolKind::Smooth.dab_strength(0.3, true), 0.6);
        assert_eq!(SculptToolKind::AddRemove.dab_strength(0.3, false), 0.3);
    }

    #[test]
    fn size_slider_maps_monotonically_into_the_mm_range() {
        assert!(size_to_radius_mm(SCULPT_SIZE_MIN) < size_to_radius_mm(SCULPT_SIZE_MAX));
        assert!(size_to_radius_mm(SCULPT_SIZE_MIN) >= SCULPT_RADIUS_MIN_MM - 1e-4);
        assert!(size_to_radius_mm(SCULPT_SIZE_MAX) <= SCULPT_RADIUS_MAX_MM + 1e-4);
    }

    #[test]
    fn mean_uniform_scale_reads_a_rigid_transform_as_one() {
        let rigid = Affine3A::from_rotation_translation(
            Quat::from_rotation_y(0.7),
            Vec3::new(3.0, -2.0, 9.0),
        );
        assert!((mean_uniform_scale(&rigid) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mean_uniform_scale_survives_a_degenerate_transform() {
        assert_eq!(mean_uniform_scale(&Affine3A::from_scale(Vec3::ZERO)), 1.0);
    }

    #[test]
    fn persistent_session_accepts_a_second_stroke_after_first_commit() {
        let mesh = Mesh::new(
            Some("sculpt-test".to_string()),
            vec![
                Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
                Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("test mesh");
        let layer_id = SceneMesh::new(mesh.clone()).id();
        let brush = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&mesh)).expect("prepare");
        let mut session = SculptSession {
            layer_id,
            topology_id: mesh.topology_id(),
            session: brush,
            base_mesh: mesh.clone(),
            shadow: Arc::new(RwLock::new(mesh.vertices().to_vec())),
            topology: PreparedSceneTopology::from_mesh(&mesh),
            world_to_local: Affine3A::IDENTITY,
            local_per_world: 1.0,
            dirty_stroke: false,
            stroke_start_vertices: None,
        };
        let stroke = BrushStroke {
            center: [0.0, 0.0, 0.0],
            radius_mm: 2.0,
            strength: 1.0,
            view_dir: [0.0, 0.0, -1.0],
        };
        assert!(!session.apply_dab(stroke, BrushMode::Add).is_empty());
        session.dirty_stroke = false;
        assert!(!session.apply_dab(stroke, BrushMode::Add).is_empty());
    }
}

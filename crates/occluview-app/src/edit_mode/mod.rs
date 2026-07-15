#[cfg(test)]
mod multi_layer_selection_tests;
mod scene_sync;
mod selection;
mod selection_ops;
mod selection_set;
mod session;
#[cfg(test)]
mod session_multi_layer_tests;
#[cfg(test)]
mod session_tests;
pub(crate) mod state;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;
pub(crate) mod undo;
mod undo_snapshot;

use occluview_core::{Scene, SceneMeshId};
pub(crate) use selection::ScreenPolygonSelectionRequest;
use selection_set::FaceSelectionSet;
pub(crate) use state::{EditModeCommand, EditModeState, EditSessionToken, SelectGesture};
// History/session enums are consumed by tests across modules only.
#[cfg(test)]
pub(crate) use state::{BusyFinish, LayerKey};
use undo::UndoStack;
use undo_snapshot::MeshEditUndoSnapshot;
pub(crate) use undo_snapshot::StructuralHistoryStep;

const DEFAULT_UNDO_COUNT: usize = 16;
const DEFAULT_UNDO_BYTES: usize = 512 * 1024 * 1024;

pub(crate) struct EditModeController {
    state: EditModeState,
    undo: UndoStack<MeshEditUndoSnapshot>,
    selections: FaceSelectionSet,
    /// Temporary compatibility target for unchanged app call sites. The
    /// canonical selection state does not use an active target.
    compatibility_layer_id: Option<SceneMeshId>,
    next_token: u64,
    /// Which primary-gesture the selection is in (marquee / lasso / object).
    /// Mutually exclusive: the three cannot be armed at once. Default
    /// [`SelectGesture::Marquee`] leaves the camera free and single-face-clicks;
    /// a session opens on Lasso (see `begin_face_selection`).
    gesture: SelectGesture,
    /// Lasso selection mode: false (default) = surface, only front-facing
    /// triangles inside the outline; true = through-mesh, every enclosed face.
    through_mesh: bool,
    /// Scene captured when an edit session began, so Cancel can revert the whole
    /// session (including structural cut/separate ops that added layers) in one
    /// predictable step. Held only while a session is active.
    baseline_scene: Option<Scene>,
    /// Session-wide dirty marker. The state machine tracks the target of the
    /// current operation, while this survives switches between editable layers.
    session_dirty: bool,
    /// Layer the live edit session targets. A topology-changing op invalidates
    /// the selection mask (sync clears it), so this is what lets
    /// `sync_to_scene` re-arm an empty selection afterwards and keep the
    /// session panel open between ops. Cleared on Done/Cancel; a missing layer
    /// suspends the target while preserving the session baseline for Cancel.
    session_layer_id: Option<SceneMeshId>,
    /// Whether the last `begin_*_edit` actually stored its undo snapshot (an
    /// oversized snapshot is skipped): guards the no-op discard from popping an
    /// unrelated older snapshot.
    last_undo_push_stored: bool,
}

impl Default for EditModeController {
    fn default() -> Self {
        Self::new(DEFAULT_UNDO_COUNT, DEFAULT_UNDO_BYTES)
    }
}

impl EditModeController {
    pub(crate) fn new(max_undo_count: usize, max_undo_bytes: usize) -> Self {
        Self {
            state: EditModeState::default(),
            undo: UndoStack::new(max_undo_count, max_undo_bytes),
            selections: FaceSelectionSet::default(),
            compatibility_layer_id: None,
            next_token: 1,
            gesture: SelectGesture::default(),
            through_mesh: false,
            baseline_scene: None,
            session_dirty: false,
            session_layer_id: None,
            last_undo_push_stored: false,
        }
    }

    fn next_session_token(&mut self) -> EditSessionToken {
        let token = EditSessionToken::new(self.next_token);
        self.next_token = self.next_token.checked_add(1).unwrap_or(1);
        token
    }

    /// Whether the freehand lasso owns the primary viewport drag.
    pub(crate) fn lasso_armed(&self) -> bool {
        matches!(self.gesture, SelectGesture::Lasso)
    }

    /// Whether the Object pick owns the primary click (a stationary click
    /// selects a whole connected component).
    pub(crate) fn object_mode(&self) -> bool {
        matches!(self.gesture, SelectGesture::Object)
    }

    /// Arm (`true`) or disarm the lasso capture. Arming switches to Lasso and so
    /// disarms Object; disarming returns to the default Marquee gesture. Returns
    /// true when the gesture actually changed.
    pub(crate) fn set_lasso_armed(&mut self, armed: bool) -> bool {
        self.set_gesture(if armed {
            SelectGesture::Lasso
        } else {
            SelectGesture::Marquee
        })
    }

    /// Arm (`true`) or disarm Object pick. Arming switches to Object and so
    /// disarms the lasso; disarming returns to the default Marquee gesture.
    /// Returns true when the gesture actually changed.
    pub(crate) fn set_object_mode(&mut self, active: bool) -> bool {
        self.set_gesture(if active {
            SelectGesture::Object
        } else {
            SelectGesture::Marquee
        })
    }

    /// Switch the primary-gesture, reporting whether it changed. The single
    /// choke point that keeps the three modes mutually exclusive.
    fn set_gesture(&mut self, next: SelectGesture) -> bool {
        if self.gesture == next {
            return false;
        }
        self.gesture = next;
        true
    }

    /// Current lasso selection mode (false = surface/front-facing, true = through-mesh).
    pub(crate) fn through_mesh(&self) -> bool {
        self.through_mesh
    }

    /// The layer a live edit session targets, if any. Used by the open-guard
    /// dialog to name what is at stake ("An edit session is active on `layer`").
    /// A closed session that merely left unsaved edits behind reports `None`.
    pub(crate) fn session_layer_id(&self) -> Option<SceneMeshId> {
        self.session_layer_id
    }

    pub(crate) fn has_active_session(&self) -> bool {
        self.baseline_scene.is_some()
    }

    /// Toggle the surface/through-mesh selection mode. Returns true when changed.
    pub(crate) fn set_through_mesh(&mut self, through_mesh: bool) -> bool {
        if self.through_mesh == through_mesh {
            return false;
        }
        self.through_mesh = through_mesh;
        true
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> &EditModeState {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn active_layer(&self) -> Option<LayerKey> {
        self.state.active_layer()
    }

    #[cfg(test)]
    pub(crate) fn undo_len(&self) -> usize {
        self.undo.undo_len()
    }
}

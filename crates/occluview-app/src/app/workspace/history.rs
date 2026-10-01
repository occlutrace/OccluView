//! One bounded command journal shared by every scene in an app workspace.
//!
//! A record lives once in `entries`; scene timelines only contain its command
//! id. A transfer is therefore one command referenced by both timelines, and
//! navigation can validate and advance both sides as one operation.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use occluview_core::SceneMeshId;

use super::id::{PaneId, SceneKey};
use super::layout::WorkspaceLayout;

mod budget;
mod checkpoint;
mod edit;
mod journal;
mod transfer;
#[cfg(test)]
mod tests;

use self::journal::{EditCheckpoint, HistoryEntry, PendingEntry, SceneTimeline};

pub(crate) type WorkspaceHistoryHandle = Rc<RefCell<WorkspaceHistory>>;
pub(crate) type HistoryCommandId = u64;
pub(crate) type EditCheckpointId = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryDirection {
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryStepKind {
    LayerEdit { layer_id: SceneMeshId },
    SceneEdit { layer_id: SceneMeshId },
    Transfer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HistoryStepInfo {
    pub(crate) command_id: HistoryCommandId,
    pub(crate) kind: HistoryStepKind,
    pub(crate) participants: Vec<SceneKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryError {
    EmptyTransfer,
    SameSceneTransfer,
    EditSessionActive(SceneKey),
    UnknownCommand(HistoryCommandId),
    BudgetExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NavigationError {
    UnknownCommand(HistoryCommandId),
    CommandNotAtHead(HistoryCommandId),
    SceneClosed(SceneKey),
    EditSessionActive(SceneKey),
}

#[derive(Clone, Debug)]
pub(crate) struct TransferredLayer {
    pub(crate) layer_id: SceneMeshId,
    pub(crate) source_index: usize,
    pub(crate) destination_index: usize,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) source_hidden_stack_position: Option<usize>,
    pub(crate) destination_hidden_stack_position: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct AutoCreatedDestination {
    pub(crate) pane_id: PaneId,
    pub(crate) name: String,
    pub(crate) previous_layout: WorkspaceLayout,
    pub(crate) previous_saved_split: Option<WorkspaceLayout>,
    pub(crate) layout_with_destination: WorkspaceLayout,
    /// Monotonic user activity that means Undo should keep the empty scene.
    pub(crate) preserve_on_undo: bool,
    /// Captured by Undo and consumed by Redo to restore the same scene state.
    pub(crate) retained_after_undo: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TransferRecord {
    pub(crate) source: SceneKey,
    pub(crate) destination: SceneKey,
    pub(crate) layers: Vec<TransferredLayer>,
    pub(crate) auto_created_destination: Option<AutoCreatedDestination>,
    /// Undo restores `None` when a pre-existing pane had not loaded a scene.
    pub(crate) destination_was_uninitialized: bool,
    pub(crate) source_focused_layer_before: Option<SceneMeshId>,
    pub(crate) destination_focused_layer_before: Option<SceneMeshId>,
    pub(crate) active_scene_before: SceneKey,
    pub(crate) active_scene_after: SceneKey,
}

pub(crate) struct HistorySnapshot<T> {
    pub(crate) value: T,
    pub(crate) bytes: usize,
}

/// The workspace owns one command store and per-scene views over it. Detached
/// controllers used by existing single-scene call sites use `None` as a local
/// timeline; they still go through this same journal implementation.
pub(crate) struct WorkspaceHistory {
    timelines: HashMap<Option<SceneKey>, SceneTimeline>,
    entries: HashMap<HistoryCommandId, HistoryEntry>,
    pending: HashMap<HistoryCommandId, PendingEntry>,
    checkpoints: HashMap<EditCheckpointId, EditCheckpoint>,
    entry_order: VecDeque<HistoryCommandId>,
    next_command_id: Option<HistoryCommandId>,
    next_checkpoint_id: Option<EditCheckpointId>,
    used_bytes: usize,
    max_count: usize,
    max_bytes: usize,
}

impl WorkspaceHistory {
    pub(crate) fn new(max_count: usize, max_bytes: usize) -> Self {
        Self {
            timelines: HashMap::new(),
            entries: HashMap::new(),
            pending: HashMap::new(),
            checkpoints: HashMap::new(),
            entry_order: VecDeque::new(),
            next_command_id: Some(1),
            next_checkpoint_id: Some(1),
            used_bytes: 0,
            max_count,
            max_bytes,
        }
    }

    pub(crate) fn shared(max_count: usize, max_bytes: usize) -> WorkspaceHistoryHandle {
        Rc::new(RefCell::new(Self::new(max_count, max_bytes)))
    }

    pub(crate) fn used_bytes(&self) -> usize {
        self.used_bytes
    }
    #[cfg(test)]
    pub(crate) fn undo_len(&self, scope: Option<SceneKey>) -> usize {
        self.timelines
            .get(&scope)
            .map_or(0, |timeline| timeline.undo.len())
    }
}

impl Default for WorkspaceHistory {
    fn default() -> Self {
        Self::new(16, 512 * 1024 * 1024)
    }
}

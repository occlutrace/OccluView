//! Explicit, identity-scoped commands accepted by the workspace coordinator.

use super::id::{PaneId, SceneKey};
use super::input::PaneTarget;
use super::layout::WorkspaceLayout;
use occluview_core::SceneMeshId;
use std::collections::HashSet;

/// Which side of the anchor scene receives a newly created view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitSide {
    Left,
    Right,
}

/// A destination that is either already live or created atomically with a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferDestination {
    Existing(SceneKey),
    CreateBeside {
        scene: SceneKey,
        pane: PaneId,
        side: SplitSide,
    },
}

/// Ordered, non-empty layer IDs for one workspace transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayerIds(Vec<SceneMeshId>);

impl LayerIds {
    pub(crate) fn new(ids: Vec<SceneMeshId>) -> Result<Self, LayerIdsError> {
        if ids.is_empty() {
            return Err(LayerIdsError::Empty);
        }
        let mut unique = HashSet::with_capacity(ids.len());
        if ids.iter().any(|id| !unique.insert(*id)) {
            return Err(LayerIdsError::Duplicate);
        }
        Ok(Self(ids))
    }

    #[must_use]
    pub(crate) fn one(id: SceneMeshId) -> Self {
        Self(vec![id])
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[SceneMeshId] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerIdsError {
    Empty,
    Duplicate,
}

/// Opaque drag data: geometry remains owned by its source scene until commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerDragPayload {
    pub(crate) source: SceneKey,
    pub(crate) layer: SceneMeshId,
}

/// One user intent. The coordinator validates live keys and commits mutations.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkspaceCommand {
    RetryGraphics,
    CreateScene {
        anchor: SceneKey,
        scene: SceneKey,
        pane: PaneId,
        side: SplitSide,
    },
    Activate(PaneTarget),
    SetLayout(WorkspaceLayout),
    RequestRenameScene {
        scene: SceneKey,
    },
    RenameScene {
        scene: SceneKey,
        name: String,
    },
    CloseScene {
        scene: SceneKey,
    },
    Transfer {
        source: SceneKey,
        destination: TransferDestination,
        layers: LayerIds,
    },
    Undo {
        scene: SceneKey,
    },
    Redo {
        scene: SceneKey,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{LayerIds, LayerIdsError};
    use occluview_core::{Mesh, SceneMesh};

    #[test]
    fn empty_transfer_selection_is_rejected() {
        assert_eq!(LayerIds::new(Vec::new()), Err(LayerIdsError::Empty));
    }

    #[test]
    fn layer_selection_rejects_duplicates_and_preserves_order() {
        let first = SceneMesh::new(Mesh::empty()).id();
        let second = SceneMesh::new(Mesh::empty()).id();
        assert_eq!(
            LayerIds::new(vec![first, second, first]),
            Err(LayerIdsError::Duplicate)
        );
        assert_eq!(
            LayerIds::new(vec![first, second]).unwrap().as_slice(),
            &[first, second]
        );
        assert_eq!(LayerIds::one(first).as_slice(), &[first]);
    }
}

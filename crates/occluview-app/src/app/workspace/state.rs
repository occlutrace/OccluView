//! Owners of scene content and the workspace that coordinates their views.

use super::commands::{LayerDragPayload, LayerDropTarget, WorkspaceCommand};
use super::history::{WorkspaceHistory, WorkspaceHistoryHandle};
use super::id::{IdAllocator, PaneId, SceneId, SceneKey};
use super::input::{InputArbiter, PaneTarget};
use super::layout::WorkspaceLayout;
use crate::app::state_document::DocumentState;
use crate::app::state_render::RenderState;
use crate::app::state_tool::ToolState;
use crate::app::state_ui::SceneUiState;
use crate::live_viewport::SharedLiveViewport;
use eframe::egui;
use std::collections::VecDeque;

pub(in crate::app) struct SceneSession {
    pub(in crate::app) key: SceneKey,
    pub(in crate::app) pane: PaneId,
    pub(in crate::app) name: String,
    pub(in crate::app) document: DocumentState,
    pub(in crate::app) render: RenderState,
    pub(in crate::app) tools: ToolState,
    pub(in crate::app) presentation: SceneUiState,
    pub(in crate::app) preserve_on_transfer_undo: bool,
}

impl SceneSession {
    pub(in crate::app) fn new(
        key: SceneKey,
        pane: PaneId,
        name: String,
        viewport: Option<SharedLiveViewport>,
        history: &WorkspaceHistoryHandle,
    ) -> Self {
        let mut document = DocumentState::new();
        document.edit_mode = crate::edit_mode::EditModeController::new_for_scene(
            history.clone(),
            key,
            16,
            512 * 1024 * 1024,
        );
        Self {
            key,
            pane,
            name,
            document,
            render: RenderState::new(viewport),
            tools: ToolState::new(),
            presentation: SceneUiState::default(),
            preserve_on_transfer_undo: false,
        }
    }

    pub(in crate::app) fn target(&self) -> PaneTarget {
        PaneTarget {
            scene: self.key,
            pane: self.pane,
        }
    }
}

#[derive(Clone)]
pub(in crate::app) struct SceneSummary {
    pub(in crate::app) key: SceneKey,
    pub(in crate::app) pane: PaneId,
    pub(in crate::app) name: String,
}

pub(in crate::app) struct WorkspaceState {
    pub(in crate::app) history: WorkspaceHistoryHandle,
    pub(in crate::app) retired_frame_textures: Vec<egui::TextureHandle>,
    pub(in crate::app) scenes: Vec<SceneSession>,
    pub(in crate::app) ids: IdAllocator,
    pub(in crate::app) input: InputArbiter,
    pub(in crate::app) layout: WorkspaceLayout,
    pub(in crate::app) saved_split: Option<WorkspaceLayout>,
    pub(in crate::app) commands: VecDeque<WorkspaceCommand>,
    pub(in crate::app) layer_drag: Option<LayerDragPayload>,
    /// Destination the live layer drag resolved to last frame. Kept so the
    /// drop highlight can fade and can keep a target across a small outward
    /// movement instead of flickering on and off at the edge band.
    pub(in crate::app) layer_drop_target: Option<LayerDropTarget>,
    pub(in crate::app) scene_tab_rects: Vec<(SceneKey, egui::Rect)>,
    pub(in crate::app) scene_create_rect: Option<egui::Rect>,
    pub(in crate::app) pending_drop: Option<Vec<std::path::PathBuf>>,
    pub(in crate::app) rename: Option<(SceneKey, String)>,
    pub(in crate::app) rename_focus_pending: bool,
    pub(in crate::app) close_request: Option<CloseRequest>,
    pub(in crate::app) save_before_close_pending: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) enum CloseRequest {
    Scene(SceneKey),
    Window,
}

impl WorkspaceState {
    pub(in crate::app) fn new(viewport: Option<SharedLiveViewport>, name: String) -> Self {
        let (ids, key, pane) = IdAllocator::new_with_initial_scene();
        let history = WorkspaceHistory::shared(16, 512 * 1024 * 1024);
        Self {
            scenes: vec![SceneSession::new(key, pane, name, viewport, &history)],
            history,
            retired_frame_textures: Vec::new(),
            ids,
            input: InputArbiter::new(PaneTarget { scene: key, pane }),
            layout: WorkspaceLayout::single(pane),
            saved_split: None,
            commands: VecDeque::new(),
            layer_drag: None,
            layer_drop_target: None,
            scene_tab_rects: Vec::new(),
            scene_create_rect: None,
            pending_drop: None,
            rename: None,
            rename_focus_pending: false,
            close_request: None,
            save_before_close_pending: false,
        }
    }

    pub(in crate::app) fn active_id(&self) -> SceneId {
        self.input.active().scene.id
    }

    pub(in crate::app) fn scene(&self, id: SceneId) -> Option<&SceneSession> {
        self.scenes.iter().find(|scene| scene.key.id == id)
    }

    pub(in crate::app) fn keys(&self) -> Vec<SceneKey> {
        self.scenes.iter().map(|scene| scene.key).collect()
    }

    pub(in crate::app) fn summaries(&self) -> Vec<SceneSummary> {
        let mut summaries: Vec<_> = self
            .scenes
            .iter()
            .map(|scene| SceneSummary {
                key: scene.key,
                pane: scene.pane,
                name: scene.name.clone(),
            })
            .collect();
        let split = match self.layout {
            WorkspaceLayout::SideBySide { .. } => Some(self.layout),
            WorkspaceLayout::Single { .. } => self.saved_split,
        };
        if let Some(WorkspaceLayout::SideBySide { left, .. }) = split {
            summaries.sort_by_key(|scene| scene.pane != left);
        }
        summaries
    }
}

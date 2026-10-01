//! Sculpt session preparation and teardown.

use std::sync::Arc;

use super::super::{egui, SceneContext};
use super::stroke;
use super::stroke::apply_sculpt_wheel_settings;
use crate::sculpt::sculpt_tool::uniform_scene_scale;
use crate::sculpt::sculpt_worker::SculptWorker;
use occluview_core::SceneMeshId;

impl SceneContext<'_> {
    pub(super) fn ensure_sculpt_session_for_layer(
        &mut self,
        scene: &Arc<occluview_core::Scene>,
        index: usize,
        layer_id: SceneMeshId,
    ) -> bool {
        let Some(entry) = scene.meshes().get(index) else {
            return false;
        };
        if entry.id() != layer_id {
            return false;
        }
        if uniform_scene_scale(&entry.transform).is_none() {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-nonuniform-scale")),
            );
            return false;
        }
        if self
            .tools
            .sculpt
            .session_matches(layer_id, entry.mesh.topology_id())
        {
            return true;
        }
        self.tools
            .sculpt
            .queue_preparation(Arc::clone(scene), index)
    }

    /// Prepare the active edit layer as soon as Edit Mesh/Sculpt becomes
    /// available. The one-time O(n) weld/adjacency/grid build stays off the UI
    /// thread and normally completes before the first brush press.
    pub(in crate::app) fn prepare_armed_sculpt_session(&mut self) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let target = self
            .document
            .edit_mode
            .session_layer_id()
            .and_then(|layer_id| {
                scene
                    .meshes()
                    .iter()
                    .position(|entry| entry.id() == layer_id)
            })
            .or_else(|| {
                let mut sculptable = scene.meshes().iter().enumerate().filter(|(_, entry)| {
                    entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
                });
                let first = sculptable.next().map(|(index, _)| index);
                first.filter(|_| sculptable.next().is_none())
            });
        if let Some(index) = target {
            if self
                .tools
                .sculpt
                .queue_preparation(Arc::clone(&scene), index)
            {
                self.scene_ui.status_message = None;
            } else if scene
                .meshes()
                .get(index)
                .is_some_and(|entry| uniform_scene_scale(&entry.transform).is_none())
            {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("sculpt-nonuniform-scale")),
                );
            } else {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("sculpt-preparing")),
                );
            }
        }
    }

    pub(in crate::app) fn poll_sculpt_preparation(&mut self, ctx: &egui::Context) {
        let Some(result) = self.tools.sculpt.poll_preparation() else {
            return;
        };
        match result {
            Ok(session) => {
                let valid = self.document.scene.as_ref().is_some_and(|scene| {
                    scene.meshes().iter().any(|entry| {
                        entry.id() == session.layer_id
                            && entry.mesh.topology_id() == session.topology_id
                    })
                });
                if valid && self.document.edit_mode.has_active_session() {
                    self.tools.sculpt.worker = Some(SculptWorker::spawn(session));
                    if self.tools.sculpt.armed.is_some() {
                        self.scene_ui.status_message = None;
                    }
                    self.render.invalidation.overlay_tools_changed();
                    ctx.request_repaint();
                } else {
                    self.tools.sculpt.pending_presses.clear();
                }
            }
            Err(error) => {
                self.tools.sculpt.pending_presses.clear();
                self.scene_ui.status_message = Some(self.ui.locale.tr_with(
                    crate::i18n::message_id!("sculpt-failed"),
                    &[("detail", error.as_str())],
                ));
                ctx.request_repaint();
            }
        }
    }

    pub(in crate::app) fn sculpt_has_live_work(&self) -> bool {
        self.document.unsaved_sculpt_stroke
    }

    /// Drop any in-flight stroke. If it had uncommitted dabs on the GPU, drop
    /// the persistent session too and force a full re-sync so the on-screen
    /// geometry reverts to the committed scene.
    pub(in crate::app) fn abort_sculpt_stroke(&mut self) {
        self.tools.sculpt.pending_presses.clear();
        let had_stroke = self.tools.sculpt.stroke.take().is_some();
        let had_pending = self.tools.sculpt.worker_has_pending_work();
        if had_stroke || had_pending {
            self.invalidate_sculpt_session_silent();
        }
    }

    pub(in crate::app) fn invalidate_sculpt_session_silent(&mut self) {
        self.document.unsaved_sculpt_stroke = false;
        // Cancel any worker prepared from the pre-edit scene as well as the
        // live GPU shadow. Otherwise a stale background result could become
        // active after an undo, layer removal, or structural mesh edit.
        self.tools.sculpt.invalidate_session();
        self.render.invalidation.sculpt_topology_changed();
    }

    /// Shift/Ctrl + wheel resizes / re-intensifies the brush instead of zooming.
    /// Returns `true` when it consumed the wheel so the caller skips the zoom.
    /// `over_viewport` gates it to the 3D view so a modified scroll over a panel
    /// (Layers, the mesh-editor window) keeps its normal meaning.
    pub(in crate::app) fn adjust_sculpt_brush_from_wheel(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) -> bool {
        let over_viewport = ctx
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|point| self.viewport_press_owned(ctx, response, point));
        let Some(kind) = self.tools.sculpt.armed else {
            return false;
        };
        if !over_viewport || !self.document.edit_mode.has_active_session() {
            return false;
        }
        // Keep the existing camera-wheel behavior during a pointer gesture.
        if self.tools.sculpt.stroke.is_some() || !self.tools.sculpt.pending_presses.is_empty() {
            return false;
        }
        // A released stroke can still be draining its ordered Finish command.
        // Consume modified wheel events during that interval without changing
        // captured brush parameters or zooming the view under pending history.
        if self.tools.sculpt.is_busy() {
            return stroke::has_sculpt_settings_wheel(ctx, self.scene_key);
        }
        if !apply_sculpt_wheel_settings(ctx, self.scene_key, Some(kind)) {
            return false;
        }
        self.render.invalidation.overlay_tools_changed();
        ctx.request_repaint();
        true
    }
}

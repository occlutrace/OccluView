//! UI-side bridge to the persistent sculpt worker.

use super::{egui, EditModeCommand, OccluViewApp};
use crate::sculpt_worker::{SculptCompletion, SculptUpdate};
use occluview_core::{Mesh, SceneMeshId};
use std::sync::Arc;

impl OccluViewApp {
    pub(super) fn complete_pending_mesh_edit_session(&mut self, ctx: &egui::Context) {
        if !self.sculpt.finish_requested || self.sculpt.worker_has_pending_work() {
            return;
        }
        self.sculpt.finish_requested = false;
        self.finish_mesh_edit_session_now(ctx);
    }

    pub(super) fn complete_pending_history_navigation(&mut self, ctx: &egui::Context) {
        let Some(redo) = self.sculpt.pending_history else {
            return;
        };
        if self.sculpt.worker_has_pending_work() {
            return;
        }
        self.sculpt.pending_history = None;
        self.apply_history_navigation_now(redo, ctx);
    }

    /// Drain worker updates and commit completed strokes without making the
    /// viewport wait for geometry work.
    pub(super) fn poll_sculpt_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.sculpt.worker.as_ref() else {
            return;
        };
        let mut updates = Vec::new();
        while let Some(update) = worker.take_update() {
            updates.push(update);
        }
        let mut completions = Vec::new();
        while let Some(completion) = worker.take_completion() {
            completions.push(completion);
        }
        let had_updates = !updates.is_empty();
        let had_completions = !completions.is_empty();
        let error = worker.take_error();
        let needs_repaint = !worker.is_quiescent();
        for update in updates {
            self.flush_sculpt_update(update);
        }
        if let Some(error) = error {
            self.status_message = Some(format!("Sculpt worker stopped: {error}"));
            self.invalidate_sculpt_session_silent();
        }
        for SculptCompletion { before, mesh } in completions {
            if !self.commit_sculpt_result(before, mesh, ctx) {
                self.invalidate_sculpt_session_silent();
                break;
            }
        }
        if had_updates || had_completions {
            self.needs_render = true;
        }
        self.complete_pending_mesh_edit_session(ctx);
        self.complete_pending_history_navigation(ctx);
        if needs_repaint || had_updates || had_completions {
            ctx.request_repaint();
        }
    }

    fn flush_sculpt_update(&mut self, update: SculptUpdate) {
        let Some(worker) = self.sculpt.worker.as_ref() else {
            return;
        };
        let touched = if update.full_sync {
            Vec::new()
        } else {
            let mut touched = update.touched;
            touched.sort_unstable();
            touched.dedup();
            touched
        };
        let shadow = worker.shadow();
        // The worker briefly holds the write lock while it patches a large
        // brush region. Never make the egui frame wait behind that write: skip
        // this GPU upload and repaint on the next frame instead.
        let Ok(shadow) = shadow.try_read() else {
            return;
        };
        if let Some(live_viewport) = self.live_viewport.as_ref() {
            if let Ok(viewport) = live_viewport.lock() {
                let _ = if update.full_sync {
                    viewport.write_scene_vertices(&worker.topology, &shadow)
                } else {
                    viewport.write_scene_vertices_sparse(&worker.topology, &shadow, &touched)
                };
            }
        } else if let (Some(offscreen), Some(prepared)) =
            (self.offscreen.as_ref(), self.prepared_scene.as_ref())
        {
            let _ = if update.full_sync {
                prepared.write_entry_vertices(offscreen.renderer(), &worker.topology, &shadow)
            } else {
                prepared.write_entry_vertices_sparse(
                    offscreen.renderer(),
                    &worker.topology,
                    &shadow,
                    &touched,
                )
            };
        }
    }

    /// Finish the drag: the worker creates the mesh off the UI thread and the
    /// next worker poll installs it as one undoable layer edit.
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) {
        if self.sculpt.stroke.take().is_none() {
            return;
        }
        if self
            .sculpt
            .worker
            .as_ref()
            .is_none_or(|worker| !worker.finish_stroke())
        {
            self.status_message = Some("Sculpt worker is unavailable".to_string());
        }
        ctx.request_repaint();
    }

    fn commit_sculpt_result(&mut self, before: Mesh, sculpted: Mesh, ctx: &egui::Context) -> bool {
        let Some(worker) = self.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let topology_id = worker.topology_id;
        let Some(scene) = self.scene.clone() else {
            return false;
        };
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            return false;
        };
        if entry.mesh.topology_id() != topology_id {
            return false;
        }
        let Some(token) =
            self.edit_mode
                .begin_layer_edit_with_snapshot(entry, before, EditModeCommand::Sculpt)
        else {
            self.status_message = Some("Layer edit already in progress".to_string());
            return false;
        };
        drop(scene);
        if self.commit_sculpt_scene(layer_id, sculpted, ctx) {
            let _ = self.edit_mode.finish_layer_edit_success(token);
            self.mark_mesh_edits_unsaved(layer_id);
            self.status_message = Some("Sculpt applied (Ctrl+Z undoes)".to_string());
            true
        } else {
            let _ = self
                .edit_mode
                .finish_layer_edit_error(token, "sculpt commit failed".to_string());
            false
        }
    }

    fn commit_sculpt_scene(
        &mut self,
        layer_id: SceneMeshId,
        mesh: Mesh,
        ctx: &egui::Context,
    ) -> bool {
        let Some(mut scene_arc) = self.scene.take() else {
            return false;
        };
        {
            let scene = Arc::make_mut(&mut scene_arc);
            let Some(entry) = scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
            else {
                self.scene = Some(scene_arc);
                return false;
            };
            entry.mesh = mesh;
        }
        self.edit_mode.sync_to_scene(&scene_arc);
        self.scene_stats = Some(super::app_render::scene_stats(&scene_arc));
        self.scene = Some(scene_arc);
        self.needs_render = true;
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        }
        ctx.request_repaint();
        true
    }
}

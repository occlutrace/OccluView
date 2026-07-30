//! UI-side bridge to the persistent sculpt worker.

use super::{egui, EditModeCommand, OccluViewApp};
use crate::sculpt_tool::SculptRebuild;
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
        // Topology first: a densifying dab replaced the layer, and any sparse
        // vertex update queued behind it indexes the array that just went away.
        let mut rebuilds = Vec::new();
        while let Some(rebuild) = worker.take_rebuild() {
            rebuilds.push(rebuild);
        }
        let mut updates = Vec::new();
        while let Some(update) = worker.take_update() {
            updates.push(update);
        }
        let mut completions = Vec::new();
        while let Some(completion) = worker.take_completion() {
            completions.push(completion);
        }
        let had_rebuilds = !rebuilds.is_empty();
        let had_updates = !updates.is_empty();
        let had_completions = !completions.is_empty();
        let error = worker.take_error();
        let needs_repaint = !worker.is_quiescent();
        for rebuild in rebuilds {
            if !self.install_sculpt_rebuild(rebuild) {
                self.invalidate_sculpt_session_silent();
                return;
            }
        }
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
        if had_rebuilds || had_updates || had_completions {
            self.needs_render = true;
        }
        self.complete_pending_mesh_edit_session(ctx);
        self.complete_pending_history_navigation(ctx);
        if needs_repaint || had_rebuilds || had_updates || had_completions {
            ctx.request_repaint();
        }
    }

    /// Install a whole-layer rebuild produced mid-stroke by densification.
    ///
    /// This is the ONE sculpt path that changes a layer's `topology_id`: the
    /// mesh grew, so the exactly-sized GPU buffers cannot be streamed into and
    /// the prepared scene has to be rebuilt. It deliberately does NOT open an
    /// undo entry — the stroke is still in flight, and the worker holds the
    /// pre-stroke mesh as the single baseline the eventual commit will use.
    /// Returns `false` if the scene no longer matches, which makes the caller
    /// drop the session rather than sculpt against stale geometry.
    fn install_sculpt_rebuild(&mut self, rebuild: SculptRebuild) -> bool {
        let Some(worker) = self.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let expected = worker.topology_id;
        let new_topology_id = rebuild.mesh.topology_id();
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
            if entry.mesh.topology_id() != expected {
                self.scene = Some(scene_arc);
                return false;
            }
            entry.mesh = rebuild.mesh;
        }
        self.edit_mode.sync_to_scene(&scene_arc);
        self.scene = Some(scene_arc);
        if let Some(worker) = self.sculpt.worker.as_mut() {
            worker.topology_id = new_topology_id;
            worker.topology = rebuild.topology;
        }
        // The uploaded geometry is the wrong SIZE now, so the prepared scene
        // must be rebuilt rather than reconciled.
        self.live_viewport_scene_dirty = self.live_viewport.is_some();
        self.offscreen_scene_dirty = true;
        self.needs_render = true;
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        }
        true
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
        self.scene = Some(scene_arc);
        self.needs_render = true;
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        }
        ctx.request_repaint();
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// Source contract for the densification corruption hazard.
    ///
    /// A dab that densifies replaces the layer's vertex ARRAY and triangle
    /// list. Any sparse vertex ids the worker queued before that point index
    /// the array that just went away, and the prepared GPU buffers are now the
    /// wrong size. So the poll must take and install rebuilds BEFORE it flushes
    /// sparse updates, and installing one must mark the prepared scene for a
    /// full rebuild rather than a uniform-only reconcile.
    #[test]
    fn a_layer_rebuild_is_installed_before_any_sparse_vertex_write() {
        let source = include_str!("app_sculpt_worker.rs").replace("\r\n", "\n");
        let take_rebuild = source
            .find("worker.take_rebuild()")
            .expect("the poll must drain pending layer rebuilds");
        let take_update = source
            .find("worker.take_update()")
            .expect("the poll must drain sparse updates");
        assert!(
            take_rebuild < take_update,
            "rebuilds must be drained before sparse updates"
        );
        let install = source
            .find("self.install_sculpt_rebuild(rebuild)")
            .expect("the poll must install pending rebuilds");
        let flush = source
            .find("self.flush_sculpt_update(update)")
            .expect("the poll must flush sparse updates");
        assert!(
            install < flush,
            "a rebuild must be installed before the frame's sparse writes"
        );
        assert!(
            source.contains("self.offscreen_scene_dirty = true;")
                && source
                    .contains("self.live_viewport_scene_dirty = self.live_viewport.is_some();"),
            "installing a rebuild must force a full prepared-scene rebuild"
        );
    }
}

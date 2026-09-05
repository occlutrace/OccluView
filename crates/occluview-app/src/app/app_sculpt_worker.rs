//! UI-side bridge to the persistent sculpt worker.

use super::{egui, EditModeCommand, OccluViewApp};
use crate::sculpt_tool::SculptRebuild;
use crate::sculpt_worker::{SculptCompletion, SculptFailure, SculptUpdate};
use occluview_core::{Mesh, SceneMeshId};
use std::sync::Arc;

/// Outcome of one GPU-upload attempt. Contention restores the update for
/// retry; a rejected write escalates to a full sync plus a topology rebuild.
pub(super) enum SculptFlushOutcome {
    Applied,
    Deferred,
    GpuRejected,
    NoTarget,
    WorkerGone,
}

/// Render a worker failure at the presentation boundary. The worker returns
/// the typed reason; only this layer owns the English copy.
fn describe_sculpt_failure(failure: &SculptFailure) -> String {
    match failure {
        SculptFailure::WorkerPanicked { message } => {
            format!("sculpt worker panicked: {message}")
        }
        SculptFailure::Spawn { detail } => {
            format!("could not start sculpt worker: {detail}")
        }
        SculptFailure::KernelPool { detail } => {
            format!("could not create sculpt kernel pool: {detail}")
        }
        SculptFailure::MissingUndoBaseline => "sculpt stroke has no undo baseline".to_string(),
        SculptFailure::ShadowPoisoned => "sculpt shadow lock was poisoned".to_string(),
        SculptFailure::VertexCountChanged => "sculpt result changed the vertex count".to_string(),
    }
}

impl OccluViewApp {
    pub(super) fn complete_pending_mesh_edit_session(&mut self, ctx: &egui::Context) {
        if !self.tools.sculpt.finish_requested || self.tools.sculpt.worker_has_pending_work() {
            return;
        }
        self.tools.sculpt.finish_requested = false;
        self.finish_mesh_edit_session_now(ctx);
    }

    pub(super) fn complete_pending_history_navigation(&mut self, ctx: &egui::Context) {
        let Some(redo) = self.tools.sculpt.pending_history else {
            return;
        };
        if self.tools.sculpt.worker_has_pending_work() {
            return;
        }
        self.tools.sculpt.pending_history = None;
        self.apply_history_navigation_now(redo, ctx);
    }

    /// Drain worker updates and commit completed strokes without making the
    /// viewport wait for geometry work.
    pub(super) fn poll_sculpt_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
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
        if let Some(failure) = error {
            let detail = describe_sculpt_failure(&failure);
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr_with("sculpt-worker-stopped", &[("detail", detail.as_str())]),
            );
            self.invalidate_sculpt_session_silent();
        }
        for SculptCompletion { before, mesh } in completions {
            if !self.commit_sculpt_result(before, mesh, ctx) {
                self.invalidate_sculpt_session_silent();
                break;
            }
        }
        if had_rebuilds || had_updates || had_completions {
            // Rebuilds and sparse writes already landed in GPU buffers above;
            // only the repaint is owed here.
            self.render.invalidation.request_redraw();
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
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let expected = worker.topology_id;
        let new_topology_id = rebuild.mesh.topology_id();
        let Some(mut scene_arc) = self.document.scene.take() else {
            return false;
        };
        {
            let scene = super::state_document::taken_scene_mut(&mut scene_arc);
            let Some(entry) = scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
            else {
                self.document.scene = Some(scene_arc);
                return false;
            };
            if entry.mesh.topology_id() != expected {
                self.document.scene = Some(scene_arc);
                return false;
            }
            entry.mesh = Arc::new(rebuild.mesh);
        }
        self.document.edit_mode.sync_to_scene(&scene_arc);
        self.document.scene = Some(scene_arc);
        if let Some(worker) = self.tools.sculpt.worker.as_mut() {
            worker.topology_id = new_topology_id;
            worker.topology = rebuild.topology;
        }
        // The uploaded geometry is the wrong SIZE now, so the prepared scene
        // must be rebuilt rather than reconciled.
        self.render.invalidation.sculpt_topology_changed();
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
        }
        true
    }

    fn flush_sculpt_update(&mut self, update: SculptUpdate) -> SculptFlushOutcome {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            // Single-threaded poll drained this from a live worker, so a
            // missing worker means the session was invalidated mid-poll and
            // teardown owns recovery; the drained delta dies with it.
            return SculptFlushOutcome::WorkerGone;
        };
        let full_sync = update.full_sync;
        // Sort before touching the shadow so the shared read is held only for
        // the upload. Restoring keeps the order; the next flush re-sorts.
        let mut touched = update.touched;
        if !full_sync {
            touched.sort_unstable();
            touched.dedup();
        }
        let shadow = worker.shadow();
        // The worker briefly holds the write lock while it patches a large
        // brush region. Never make the egui frame wait behind that write:
        // restore the drained update and retry on a later frame instead.
        let Ok(shadow) = shadow.try_read() else {
            worker.restore_update(SculptUpdate { touched, full_sync });
            return SculptFlushOutcome::Deferred;
        };
        if let Some(live_viewport) = self.render.live_viewport.as_ref() {
            let Ok(viewport) = live_viewport.try_lock() else {
                worker.restore_update(SculptUpdate { touched, full_sync });
                return SculptFlushOutcome::Deferred;
            };
            let applied = if full_sync {
                viewport.write_scene_vertices(&worker.topology, &shadow)
            } else {
                viewport.write_scene_vertices_sparse(&worker.topology, &shadow, &touched)
            };
            if applied {
                SculptFlushOutcome::Applied
            } else {
                worker.request_full_sync();
                self.render.invalidation.sculpt_topology_changed();
                if self.can_render_cut_view() {
                    self.tools.cut_view.mark_dirty();
                }
                SculptFlushOutcome::GpuRejected
            }
        } else if let (Some(offscreen), Some(prepared)) = (
            self.render.offscreen.as_ref(),
            self.render.prepared_scene.as_ref(),
        ) {
            let applied = if full_sync {
                prepared.write_entry_vertices(offscreen.renderer(), &worker.topology, &shadow)
            } else {
                prepared.write_entry_vertices_sparse(
                    offscreen.renderer(),
                    &worker.topology,
                    &shadow,
                    &touched,
                )
            };
            if applied {
                SculptFlushOutcome::Applied
            } else {
                worker.request_full_sync();
                self.render.invalidation.sculpt_topology_changed();
                if self.can_render_cut_view() {
                    self.tools.cut_view.mark_dirty();
                }
                SculptFlushOutcome::GpuRejected
            }
        } else {
            // No GPU target: the CPU shadow stays authoritative and the
            // commit path sources it, so there is no stale GPU state to fix.
            SculptFlushOutcome::NoTarget
        }
    }

    /// Finish the drag: the worker creates the mesh off the UI thread and the
    /// next worker poll installs it as one undoable layer edit.
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) {
        if self.tools.sculpt.stroke.take().is_none() {
            return;
        }
        if self
            .tools
            .sculpt
            .worker
            .as_ref()
            .is_none_or(|worker| !worker.finish_stroke())
        {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-worker-unavailable"));
        }
        ctx.request_repaint();
    }

    fn commit_sculpt_result(
        &mut self,
        before: Arc<Mesh>,
        sculpted: Mesh,
        ctx: &egui::Context,
    ) -> bool {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let topology_id = worker.topology_id;
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            return false;
        };
        if entry.mesh.topology_id() != topology_id {
            return false;
        }
        let Some(token) = self.document.edit_mode.begin_layer_edit_with_snapshot(
            entry,
            before,
            EditModeCommand::Sculpt,
        ) else {
            self.ui.status_message = Some(self.ui.locale.tr("repair-edit-busy"));
            return false;
        };
        drop(scene);
        if self.commit_sculpt_scene(layer_id, sculpted, ctx) {
            let _ = self.document.edit_mode.finish_layer_edit_success(token);
            self.document.mark_mesh_edits_unsaved(layer_id);
            // Only promise the undo that exists. `begin_layer_edit_with_snapshot`
            // skips an oversized pre-op snapshot -- the edit still applies, but
            // Ctrl+Z will not bring the layer back. Telling the operator
            // otherwise is worse than saying nothing: they find out by pressing
            // it, on work they have already moved on from. Every other mesh-edit
            // status goes through `with_undoable_note` for the same reason.
            self.ui.status_message = Some(if self.document.edit_mode.last_edit_undoable() {
                self.ui.locale.tr("sculpt-applied-undo")
            } else {
                self.ui.locale.tr("sculpt-applied-locked")
            });
            true
        } else {
            let _ = self
                .document
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
        let Some(mut scene_arc) = self.document.scene.take() else {
            return false;
        };
        {
            let scene = super::state_document::taken_scene_mut(&mut scene_arc);
            let Some(entry) = scene
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
            else {
                self.document.scene = Some(scene_arc);
                return false;
            };
            entry.mesh = Arc::new(mesh);
        }
        self.document.edit_mode.sync_to_scene(&scene_arc);
        self.document.scene = Some(scene_arc);
        // The commit swaps the layer's mesh Arc after the stroke's bytes were
        // already pushed to the GPU by sparse writes; only a repaint is owed.
        self.render.invalidation.request_redraw();
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
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
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"))
                .replace("\r\n", "\n");
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
        let install_fn = source
            .find("fn install_sculpt_rebuild(")
            .expect("the rebuild installer must exist");
        assert!(
            source[install_fn..].contains("self.render.invalidation.sculpt_topology_changed();"),
            "installing a rebuild must force a full prepared-scene rebuild"
        );
        // The typed model proves the cause mapping: a topology change stales
        // both scene consumers while sparing the selection overlay.
        let mut topology = crate::invalidation::RenderInvalidation::new();
        topology.sculpt_topology_changed();
        assert!(topology.live_scene_stale() && topology.offscreen_scene_stale());
        assert!(!topology.live_overlay_stale() && !topology.offscreen_overlay_stale());
    }
}

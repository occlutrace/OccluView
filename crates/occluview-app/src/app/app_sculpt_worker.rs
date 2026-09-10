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
/// the typed reason; each variant resolves through the catalog, so no
/// language is hardcoded here. Only the raw technical payloads (panic text,
/// OS spawn errors) travel untranslated inside `{ $detail }`.
fn describe_sculpt_failure(locale: &crate::i18n::LocaleManager, failure: &SculptFailure) -> String {
    match failure {
        SculptFailure::WorkerPanicked { message } => locale.tr_with(
            "sculpt-failure-worker-panicked",
            &[("detail", message.as_str())],
        ),
        SculptFailure::Spawn { detail } => {
            locale.tr_with("sculpt-failure-spawn", &[("detail", detail.as_str())])
        }
        SculptFailure::KernelPool { detail } => {
            locale.tr_with("sculpt-failure-kernel-pool", &[("detail", detail.as_str())])
        }
        SculptFailure::MissingUndoBaseline => locale.text("sculpt-failure-missing-undo-baseline"),
        SculptFailure::ShadowPoisoned => locale.text("sculpt-failure-shadow-poisoned"),
        SculptFailure::VertexCountChanged => locale.text("sculpt-failure-vertex-count-changed"),
        SculptFailure::TopologyRebuild { detail } => locale.tr_with(
            "sculpt-failure-topology-rebuild",
            &[("detail", detail.as_str())],
        ),
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

    fn retry_pending_sculpt_finish(&mut self, ctx: &egui::Context) {
        if self.tools.sculpt.finish_retry {
            let _ = self.commit_sculpt_stroke(ctx);
        }
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
        // Rebuilds, sparse updates, and completions are one ordered snapshot:
        // a worker publication cannot land between separate queue drains and
        // leave a completion or sparse write ahead of its topology rebuild.
        let Ok((mut rebuilds, completions, update)) = worker.take_ordered_outputs() else {
            // A worker publication or frame-path drain is in progress. Retry
            // the whole boundary on the next repaint.
            ctx.request_repaint();
            return;
        };
        let updates = update.into_iter().collect::<Vec<_>>();
        let had_rebuilds = !rebuilds.is_empty();
        let had_updates = !updates.is_empty();
        let had_completions = !completions.is_empty();
        let error = worker.take_error();
        let needs_repaint = !worker.is_quiescent();
        // Preserve the worker's logical order even though the snapshot carries
        // separate rebuild and completion queues. A completion from an older,
        // positions-only stroke already matches the current topology and must
        // commit before a later rebuild. A completion produced AFTER one or
        // more densifying dabs carries the latest fresh topology token, so
        // every intermediate rebuild in the FIFO chain must be installed first
        // or the completion would be committed against the wrong GPU contract.
        for completion in completions {
            let completion_topology_id = completion.mesh.topology_id();
            // The worker's UI-side topology is the current scene state. Walk
            // the FIFO rebuild chain until that state reaches the completion's
            // topology. This handles both cases: a completion with no
            // densification (already-current topology), and several rebuilds
            // in one stroke (all intermediate topologies must be installed).
            while self
                .tools
                .sculpt
                .worker
                .as_ref()
                .is_some_and(|worker| worker.topology_id != completion_topology_id)
            {
                let Some(rebuild) = rebuilds.pop_front() else {
                    self.invalidate_sculpt_session_silent();
                    return;
                };
                if !self.install_sculpt_rebuild(rebuild) {
                    self.invalidate_sculpt_session_silent();
                    return;
                }
            }
            let SculptCompletion { before, mesh } = completion;
            if !self.commit_sculpt_result(before, mesh, ctx) {
                self.invalidate_sculpt_session_silent();
                break;
            }
        }
        while let Some(rebuild) = rebuilds.pop_front() {
            if !self.install_sculpt_rebuild(rebuild) {
                self.invalidate_sculpt_session_silent();
                return;
            }
        }
        for update in updates {
            self.flush_sculpt_update(update);
        }
        // A terminal worker failure does not invalidate already-produced
        // completions. Surface it only after the ordered output above has had
        // a chance to commit; then revoke the worker so no later stale result
        // can reach the scene.
        if let Some(failure) = error {
            let detail = describe_sculpt_failure(&self.ui.locale, &failure);
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr_with("sculpt-worker-stopped", &[("detail", detail.as_str())]),
            );
            self.invalidate_sculpt_session_silent();
            ctx.request_repaint();
        }
        if had_rebuilds || had_updates || had_completions {
            // Rebuilds and sparse writes already landed in GPU buffers above;
            // only the repaint is owed here.
            self.render.invalidation.request_redraw();
        }
        if needs_repaint || had_rebuilds || had_updates || had_completions {
            ctx.request_repaint();
        }
        self.retry_pending_sculpt_finish(ctx);
        self.complete_pending_mesh_edit_session(ctx);
        self.complete_pending_history_navigation(ctx);
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
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) -> bool {
        let Some(stroke) = self.tools.sculpt.stroke.take() else {
            self.tools.sculpt.finish_retry = false;
            return true;
        };
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-worker-unavailable"));
            // The shadow may already contain live dabs, but without the worker
            // there is no trustworthy completion or undo baseline left. Drop
            // both sides together and force the renderer back to the committed
            // scene; keeping the taken StrokeState would leave a retry loop
            // around a session that can never finish.
            self.invalidate_sculpt_session_silent();
            ctx.request_repaint();
            return false;
        };
        if !worker.finish_stroke() {
            // Queue pressure is recoverable. Keep the drag state and let the
            // next frame retry; dropping it here would lose the whole undoable
            // stroke while the visible shadow is still dirty.
            self.tools.sculpt.stroke = Some(stroke);
            self.tools.sculpt.finish_retry = true;
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-worker-unavailable"));
            ctx.request_repaint();
            return false;
        }
        self.tools.sculpt.finish_retry = false;
        ctx.request_repaint();
        true
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
        let take_outputs = source
            .find("worker.take_ordered_outputs()")
            .expect("the poll must drain the ordered output snapshot");
        let take_update = source
            .find("let updates = update.into_iter()")
            .expect("the poll must expose sparse updates from that snapshot");
        assert!(
            take_outputs < take_update,
            "the ordered snapshot must be taken before sparse updates are flushed"
        );
        assert!(
            !source.contains("worker.try_take_rebuild()")
                && !source.contains("worker.take_completion()"),
            "the production poller must not split the topology and completion drains"
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

    /// Source contract for the two-stroke interleave hazard.
    ///
    /// The worker is sequential, so `Finish(old stroke)` then `Apply(new
    /// stroke, densifies)` queues a completion behind a rebuild. Installing
    /// the newer topology first would bump the worker topology id and let
    /// the older, smaller completion mesh fail the topology check or clobber
    /// newer geometry. The output queues therefore walk one topology chain:
    /// current completion first, then only the rebuilds needed to reach the
    /// next completion's topology.
    #[test]
    fn completions_walk_the_topology_chain_before_leftover_rebuilds_install() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"))
                .replace("\r\n", "\n");
        let commit = source
            .find("for completion in completions")
            .expect("the poll must commit finished strokes");
        let chain = source
            .find("while self\n                .tools\n                .sculpt\n                .worker")
            .expect("the poll must walk the current topology toward each completion");
        let installs: Vec<_> = source
            .match_indices("self.install_sculpt_rebuild(rebuild)")
            .map(|(offset, _)| offset)
            .collect();
        assert!(
            installs.len() >= 2,
            "the poll needs matching and leftover rebuild paths"
        );
        assert!(
            commit < chain,
            "completion processing must own the topology walk"
        );
        assert!(
            commit < installs[1],
            "leftover rebuilds must install after ordered completions"
        );
        let failure = source
            .find("if let Some(failure) = error")
            .expect("the poll must surface worker failures");
        assert!(
            commit < failure,
            "terminal worker errors must be surfaced after valid completions"
        );
    }

    /// A completion produced after densification has the same fresh topology
    /// token as the rebuild that was published mid-stroke. That rebuild must
    /// be installed first; otherwise the completion is committed against the
    /// old scene token and the next topology check drops the valid result.
    #[test]
    fn a_same_topology_completion_installs_its_rebuild_before_commit() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"))
                .replace("\r\n", "\n");
        let completion_topology = source
            .find("let completion_topology_id = completion.mesh.topology_id()")
            .expect("the completion topology must be compared with pending rebuilds");
        let matching_rebuild = source
            .find("while self\n                .tools\n                .sculpt\n                .worker")
            .expect("a completion must walk the current topology toward its target");
        let install = source
            .find("self.install_sculpt_rebuild(rebuild)")
            .expect("the matching rebuild must be installed");
        let commit = source
            .find("self.commit_sculpt_result(before, mesh, ctx)")
            .expect("the completion must still be committed");
        assert!(
            completion_topology < matching_rebuild && matching_rebuild < install,
            "a matching rebuild must be selected before it is installed"
        );
        assert!(
            install < commit,
            "the same-topology completion must commit after its rebuild"
        );
    }

    #[test]
    fn a_rejected_finish_keeps_the_stroke_for_a_later_retry() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"));
        let finish = source
            .find("pub(super) fn commit_sculpt_stroke")
            .expect("the stroke finish bridge must exist");
        let body = &source[finish..(finish + 1400).min(source.len())];
        assert!(
            body.contains("let Some(stroke) = self.tools.sculpt.stroke.take()")
                && body.contains("self.tools.sculpt.stroke = Some(stroke)")
                && body.contains("-> bool"),
            "queue backpressure must not silently discard a released stroke"
        );
        let mesh_editor =
            crate::primary_ui_tests::production_source(include_str!("app_mesh_editor.rs"));
        assert!(
            mesh_editor.contains("if !self.commit_sculpt_stroke(ctx)")
                || mesh_editor.contains("!self.commit_sculpt_stroke(ctx)"),
            "Done/history must stop while a finish is waiting for queue capacity"
        );
    }

    #[test]
    fn worker_loss_invalidates_an_active_sculpt_stroke() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"));
        let finish = source
            .find("pub(super) fn commit_sculpt_stroke")
            .expect("the stroke finish bridge must exist");
        let body = &source[finish..(finish + 1400).min(source.len())];
        let worker_guard = body
            .find("let Some(worker) = self.tools.sculpt.worker.as_ref() else")
            .expect("finish must explicitly handle a missing worker");
        let missing_worker_branch = &body[worker_guard..];
        assert!(
            missing_worker_branch.contains("self.invalidate_sculpt_session_silent();"),
            "a missing worker must discard the live shadow and session instead of leaving a stale stroke"
        );
        assert!(
            missing_worker_branch.contains("ctx.request_repaint();"),
            "worker loss must repaint so the reverted scene is visible immediately"
        );
    }
}

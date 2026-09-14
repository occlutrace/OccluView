//! UI-side bridge to the persistent sculpt worker.

use super::{egui, AppErrorAction, AppErrorDialog, EditModeCommand, OccluViewApp};
use crate::sculpt_tool::SculptRebuild;
use crate::sculpt_worker::{SculptCompletion, SculptFailure, SculptUpdate};
use occluview_core::{Mesh, SceneMeshId};
use std::sync::Arc;

/// Outcome of one GPU upload attempt.
pub(super) enum SculptFlushOutcome {
    Applied,
    Deferred,
    GpuRejected,
    NoTarget,
    WorkerGone,
}

/// Resolve a worker failure through the localized error catalog.
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
        SculptFailure::ShadowShapeMismatch => locale.text("sculpt-failure-shadow-shape"),
        SculptFailure::InvalidVertexIndex => locale.text("sculpt-failure-invalid-vertex-index"),
        SculptFailure::WorkerStatePoisoned => locale.text("sculpt-failure-worker-state-poisoned"),
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
        // Rebuilds, sparse updates, and completions must be read as one ordered
        // snapshot so topology changes precede dependent writes.
        let Ok((mut rebuilds, completions, update)) = worker.take_ordered_outputs() else {
            if let Some(failure) = worker.take_error() {
                self.fail_sculpt_session(&failure, ctx);
            }
            // Retry after the concurrent publication or drain completes.
            ctx.request_repaint();
            return;
        };
        let updates = update.into_iter().collect::<Vec<_>>();
        let had_rebuilds = !rebuilds.is_empty();
        let had_updates = !updates.is_empty();
        let had_completions = !completions.is_empty();
        let error = worker.take_error();
        let needs_repaint = !worker.is_quiescent();
        // Preserve worker order across the separate rebuild and completion
        // queues. A completion may depend on one or more preceding rebuilds.
        for completion in completions {
            let completion_topology_id = completion.mesh.topology_id();
            // Install rebuilds until the scene reaches the completion's
            // topology.
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
        // Apply valid output before surfacing a terminal worker failure.
        if let Some(failure) = error {
            self.fail_sculpt_session(&failure, ctx);
        }
        if had_rebuilds || had_updates || had_completions {
            // The GPU buffers changed; schedule a repaint.
            self.render.invalidation.request_redraw();
        }
        if needs_repaint || had_rebuilds || had_updates || had_completions {
            ctx.request_repaint();
        }
        self.retry_pending_sculpt_finish(ctx);
        self.complete_pending_mesh_edit_session(ctx);
        self.complete_pending_history_navigation(ctx);
    }

    pub(super) fn settle_sculpt_work_marker(&mut self) {
        if !self.document.unsaved_sculpt_stroke {
            return;
        }
        if self.tools.sculpt.is_busy() {
            return;
        }
        self.document.unsaved_sculpt_stroke = false;
    }

    /// Install a whole-layer rebuild produced by mid-stroke densification.
    /// The stroke remains pending and uses its pre-stroke mesh as the Undo
    /// baseline. Return `false` when the scene no longer matches the worker.
    fn install_sculpt_rebuild(&mut self, rebuild: SculptRebuild) -> bool {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let expected = worker.topology_id;
        let new_topology_id = rebuild.mesh.topology_id();
        let rebuilt_mesh = Arc::new(rebuild.mesh);
        let Some(mut scene_arc) = self.document.scene.take() else {
            return false;
        };
        let replaced = {
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
            let replaced = Arc::clone(&entry.mesh);
            entry.mesh = Arc::clone(&rebuilt_mesh);
            replaced
        };
        self.tools
            .sculpt
            .note_preview_install(layer_id, new_topology_id, replaced);
        self.document.edit_mode.sync_to_scene(&scene_arc);
        self.document.scene = Some(scene_arc);
        if let Some(worker) = self.tools.sculpt.worker.as_mut() {
            worker.topology_id = new_topology_id;
            worker.topology = rebuild.topology;
        }
        if let Some(worker) = self.tools.sculpt.worker.as_ref() {
            worker.replace_pick_mesh(rebuilt_mesh);
        }
        // Topology changed, so rebuild the prepared scene rather than updating
        // vertex contents in place.
        self.render.invalidation.sculpt_topology_changed();
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
        }
        true
    }

    /// Surface a terminal worker failure and revoke the sculpt session.
    pub(super) fn fail_sculpt_session(&mut self, failure: &SculptFailure, ctx: &egui::Context) {
        let dialog = sculpt_failure_dialog(&self.ui.locale, failure);
        self.ui.status_message = Some(dialog.summary.clone());
        self.ui.app_error = Some(dialog);
        self.tools.sculpt.disarm();
        self.invalidate_sculpt_session_silent();
        ctx.request_repaint();
    }

    fn flush_sculpt_update(&mut self, update: SculptUpdate) -> SculptFlushOutcome {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            // The session was invalidated while the poll was running.
            return SculptFlushOutcome::WorkerGone;
        };
        let full_sync = update.full_sync;
        // Normalize ids before reading the shared shadow.
        let mut touched = update.touched;
        if !full_sync {
            touched.sort_unstable();
            touched.dedup();
        }
        let shadow = worker.shadow();
        // Do not block the UI on a worker write; retry the update next frame.
        let Ok(shadow) = shadow.try_read() else {
            worker.restore_update(SculptUpdate { touched, full_sync });
            return SculptFlushOutcome::Deferred;
        };
        let mut has_target = false;
        let mut rejected = false;
        if let Some(live_viewport) = self.render.live_viewport.as_ref() {
            let Ok(viewport) = live_viewport.try_lock() else {
                worker.restore_update(SculptUpdate { touched, full_sync });
                return SculptFlushOutcome::Deferred;
            };
            if viewport.has_prepared_scene() {
                has_target = true;
                let applied = if full_sync {
                    viewport.write_scene_vertices(&worker.topology, &shadow)
                } else {
                    viewport.write_scene_vertices_sparse(&worker.topology, &shadow, &touched)
                };
                rejected |= !applied;
            }
        }
        if let (Some(offscreen), Some(prepared)) = (
            self.render.offscreen.as_ref(),
            self.render.prepared_scene.as_ref(),
        ) {
            has_target = true;
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
            rejected |= !applied;
        }
        if rejected {
            worker.request_full_sync();
            self.render.invalidation.sculpt_topology_changed();
            if self.can_render_cut_view() {
                self.tools.cut_view.mark_dirty();
            }
            SculptFlushOutcome::GpuRejected
        } else if has_target {
            // Keep Cut View in sync with the sparse update.
            if self.can_render_cut_view() {
                self.tools.cut_view.mark_dirty();
            }
            SculptFlushOutcome::Applied
        } else {
            // The CPU shadow will be uploaded when a target is prepared.
            SculptFlushOutcome::NoTarget
        }
    }

    /// Re-apply the worker shadow after a live scene rebuild.
    pub(super) fn push_sculpt_shadow_live(&self) -> Option<bool> {
        let worker = self.tools.sculpt.worker.as_ref()?;
        let live_viewport = self.render.live_viewport.as_ref()?;
        let viewport = live_viewport.try_lock().ok()?;
        if !viewport.has_prepared_scene() {
            return None;
        }
        let shadow_arc = worker.shadow();
        let shadow = shadow_arc.try_read().ok()?;
        Some(viewport.write_scene_vertices(&worker.topology, &shadow))
    }

    /// Re-apply the worker's current vertices after an offscreen scene
    /// rebuild. This keeps the fallback viewport and Cut View in lockstep
    /// with the live surface during a stroke.
    pub(super) fn push_sculpt_shadow_offscreen(&self) -> Option<bool> {
        let worker = self.tools.sculpt.worker.as_ref()?;
        let offscreen = self.render.offscreen.as_ref()?;
        let prepared = self.render.prepared_scene.as_ref()?;
        let shadow_arc = worker.shadow();
        let shadow = shadow_arc.try_read().ok()?;
        Some(prepared.write_entry_vertices(offscreen.renderer(), &worker.topology, &shadow))
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
            // Without the worker there is no completion or Undo baseline. Drop
            // the shadow and return to the committed scene.
            self.invalidate_sculpt_session_silent();
            ctx.request_repaint();
            return false;
        };
        if !worker.finish_stroke() {
            // Preserve the drag and retry after queue pressure clears.
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
            self.tools.sculpt.clear_preview_baseline();
            let _ = self.document.edit_mode.finish_layer_edit_success(token);
            self.document.mark_mesh_edits_unsaved(layer_id);
            // Report whether the pre-edit snapshot was retained.
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
        // The committed mesh replaced the preview data; invalidate all scene
        // consumers.
        self.render.invalidation.scene_geometry_changed();
        // The in-place mesh replacement invalidates alignment results measured
        // on the previous surface.
        self.invalidate_alignment_for_geometry_changes(&[layer_id]);
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
        }
        ctx.request_repaint();
        true
    }
}

/// Build the dialog for a terminal sculpt failure.
fn sculpt_failure_dialog(
    locale: &crate::i18n::LocaleManager,
    failure: &SculptFailure,
) -> AppErrorDialog {
    let detail = describe_sculpt_failure(locale, failure);
    AppErrorDialog {
        title: locale.tr("sculpt-failed-title"),
        summary: locale.tr_with("sculpt-worker-stopped", &[("detail", detail.as_str())]),
        details: format!("Sculpt worker stopped\n\n{detail}"),
        action: AppErrorAction::None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// Rebuild output must be installed before sparse updates are flushed.
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
        let mut topology = crate::invalidation::RenderInvalidation::new();
        topology.sculpt_topology_changed();
        assert!(topology.live_scene_stale() && topology.offscreen_scene_stale());
        assert!(!topology.live_overlay_stale() && !topology.offscreen_overlay_stale());
    }

    /// A terminal worker failure has to outlive the status line, because the
    /// stroke's geometry is gone and no later result can arrive to explain it.
    #[test]
    fn a_terminal_failure_raises_the_error_dialog() {
        use crate::i18n::LocaleManager;
        use crate::sculpt_worker::SculptFailure;

        let locale = LocaleManager::for_tests();
        let failure = SculptFailure::WorkerStatePoisoned;
        let dialog = super::sculpt_failure_dialog(&locale, &failure);

        assert_eq!(dialog.title, locale.tr("sculpt-failed-title"));
        let detail = super::describe_sculpt_failure(&locale, &failure);
        assert!(
            dialog.summary.contains(&detail),
            "the summary must carry the typed reason: {}",
            dialog.summary
        );
        assert!(
            dialog.details.contains(&detail),
            "the copyable details must carry the failure for a case record"
        );
    }

    /// The failure paths own the whole terminal outcome: dialog, session
    /// revocation, and the brush no longer consuming the primary gesture.
    ///
    /// There are two of them - the ordered-output drain can fail before it
    /// reads a latched error - and only one was wired up, so a poisoned
    /// coordination lock discarded the stroke under a status line that expires
    /// while the brush stayed armed.
    #[test]
    fn every_terminal_failure_exit_raises_the_dialog_and_disarms() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"))
                .replace("\r\n", "\n");
        let helper = source
            .split_once("fn fail_sculpt_session(")
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            !helper.is_empty(),
            "the terminal-failure boundary must exist"
        );
        for required in [
            "self.ui.app_error = Some(dialog)",
            "self.tools.sculpt.disarm()",
            "self.invalidate_sculpt_session_silent()",
        ] {
            assert!(
                helper.contains(required),
                "a terminal failure must run {required}"
            );
        }
        assert_eq!(
            source.matches("self.fail_sculpt_session(").count(),
            2,
            "both poll exits must route through the one terminal-failure boundary"
        );
        assert!(
            !source.contains("let detail = describe_sculpt_failure(&self.ui.locale, &failure);"),
            "no exit may report the failure without the dialog"
        );
    }

    /// A sculpt commit swaps a paired layer's mesh in place, so it never
    /// passes through `set_scene` and the alignment invalidation that lives
    /// there. The heatmap and the refined-match claim were measured against
    /// the pre-stroke surface; both must be revoked by the commit that
    /// replaced it, whichever role the sculpted scan holds.
    #[test]
    fn a_sculpt_commit_revokes_the_alignment_measured_against_the_old_mesh() {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_sculpt_worker.rs"))
                .replace("\r\n", "\n");
        let commit = source
            .split_once("fn commit_sculpt_scene(")
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            !commit.is_empty(),
            "commit_sculpt_scene must exist for this contract to mean anything"
        );
        assert!(
            commit.contains("self.invalidate_alignment_for_geometry_changes(&[layer_id])"),
            "the in-place mesh swap must revoke the alignment that described the old surface"
        );
    }

    /// Completions must advance through their topology chain in order.
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

    /// Install the matching rebuild before a completion produced after
    /// densification.
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

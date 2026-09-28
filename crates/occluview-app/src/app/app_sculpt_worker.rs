//! UI-side bridge to the persistent sculpt worker.

use super::{egui, AppErrorAction, AppErrorDialog, EditModeCommand, OccluViewApp};
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
            crate::i18n::message_id!("sculpt-failure-worker-panicked"),
            &[("detail", message.as_str())],
        ),
        SculptFailure::Spawn { detail } => locale.tr_with(
            crate::i18n::message_id!("sculpt-failure-spawn"),
            &[("detail", detail.as_str())],
        ),
        SculptFailure::KernelPool { detail } => locale.tr_with(
            crate::i18n::message_id!("sculpt-failure-kernel-pool"),
            &[("detail", detail.as_str())],
        ),
        SculptFailure::MissingUndoBaseline => locale.text(crate::i18n::message_id!(
            "sculpt-failure-missing-undo-baseline"
        )),
        SculptFailure::ShadowPoisoned => {
            locale.text(crate::i18n::message_id!("sculpt-failure-shadow-poisoned"))
        }
        SculptFailure::ShadowShapeMismatch => {
            locale.text(crate::i18n::message_id!("sculpt-failure-shadow-shape"))
        }
        SculptFailure::InvalidVertexIndex => locale.text(crate::i18n::message_id!(
            "sculpt-failure-invalid-vertex-index"
        )),
        SculptFailure::WorkerStatePoisoned => locale.text(crate::i18n::message_id!(
            "sculpt-failure-worker-state-poisoned"
        )),
        SculptFailure::VertexCountChanged => locale.text(crate::i18n::message_id!(
            "sculpt-failure-vertex-count-changed"
        )),
        SculptFailure::TopologyRebuild { detail } => locale.tr_with(
            crate::i18n::message_id!("sculpt-failure-topology-rebuild"),
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
        let Ok((topology_deltas, completions, update)) = worker.take_ordered_outputs() else {
            if let Some(failure) = worker.take_error() {
                self.fail_sculpt_session(&failure, ctx);
            }
            // Retry after the concurrent publication or drain completes.
            ctx.request_repaint();
            return;
        };
        let updates = update.into_iter().collect::<Vec<_>>();
        let had_topology_deltas = !topology_deltas.is_empty();
        let had_updates = !updates.is_empty();
        let had_completions = !completions.is_empty();
        let error = worker.take_error();
        let needs_repaint = !worker.is_quiescent();
        let mut needs_full_sync = false;
        for delta in topology_deltas {
            if matches!(
                self.flush_sculpt_topology(delta),
                SculptFlushOutcome::GpuRejected | SculptFlushOutcome::Deferred
            ) {
                if let Some(worker) = self.tools.sculpt.worker.as_ref() {
                    worker.request_full_sync();
                }
                needs_full_sync = true;
                break;
            }
        }
        if needs_full_sync {
            self.flush_sculpt_update(SculptUpdate {
                touched: Vec::new(),
                full_sync: true,
            });
        } else {
            for update in updates {
                self.flush_sculpt_update(update);
            }
        }
        for completion in completions {
            let SculptCompletion { before, mesh } = completion;
            if !self.commit_sculpt_result(before, mesh, ctx) {
                self.invalidate_sculpt_session_silent();
                break;
            }
        }
        // Apply valid output before surfacing a terminal worker failure.
        if let Some(failure) = error {
            self.fail_sculpt_session(&failure, ctx);
        }
        if had_topology_deltas || had_updates || had_completions {
            // The GPU buffers changed; schedule a repaint.
            self.render.invalidation.request_redraw();
        }
        if needs_repaint || had_topology_deltas || had_updates || had_completions {
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

    fn flush_sculpt_topology(
        &mut self,
        delta: occluview_render::SculptTopologyDelta,
    ) -> SculptFlushOutcome {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return SculptFlushOutcome::WorkerGone;
        };
        let topology = worker.topology;
        let mut has_target = false;
        let mut rejected = false;
        let mut deferred = false;
        if let Some(live_viewport) = self.render.live_viewport.as_ref() {
            match live_viewport.try_lock() {
                Ok(mut viewport) if viewport.has_prepared_scene() => {
                    has_target = true;
                    rejected |= viewport
                        .write_scene_sculpt_delta(&topology, &delta)
                        .is_none();
                }
                Ok(_) => {}
                Err(_) => deferred = true,
            }
        }
        if let (Some(offscreen), Some(prepared)) = (
            self.render.offscreen.as_ref(),
            self.render.prepared_scene.as_mut(),
        ) {
            has_target = true;
            rejected |= prepared
                .write_entry_sculpt_delta(offscreen.renderer(), &topology, &delta)
                .is_none();
        }
        if rejected {
            self.render.invalidation.sculpt_topology_changed();
            if self.can_render_cut_view() {
                self.tools.cut_view.mark_dirty();
            }
            SculptFlushOutcome::GpuRejected
        } else if deferred {
            SculptFlushOutcome::Deferred
        } else if has_target {
            if self.can_render_cut_view() {
                self.tools.cut_view.mark_dirty();
            }
            SculptFlushOutcome::Applied
        } else {
            SculptFlushOutcome::NoTarget
        }
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
        let geometry = if full_sync {
            let Some(geometry) = worker.live_geometry() else {
                worker.restore_update(SculptUpdate { touched, full_sync });
                return SculptFlushOutcome::Deferred;
            };
            Some(geometry)
        } else {
            None
        };
        let shadow_arc = worker.shadow();
        let shadow = if full_sync {
            None
        } else {
            let Ok(shadow) = shadow_arc.try_read() else {
                worker.restore_update(SculptUpdate { touched, full_sync });
                return SculptFlushOutcome::Deferred;
            };
            Some(shadow)
        };
        let mut has_target = false;
        let mut rejected = false;
        if let Some(live_viewport) = self.render.live_viewport.as_ref() {
            let Ok(mut viewport) = live_viewport.try_lock() else {
                worker.restore_update(SculptUpdate { touched, full_sync });
                return SculptFlushOutcome::Deferred;
            };
            if viewport.has_prepared_scene() {
                has_target = true;
                let applied = if let Some((vertices, indices)) = geometry.as_ref() {
                    viewport
                        .write_scene_sculpt_geometry(&worker.topology, vertices, indices)
                        .is_some()
                } else {
                    viewport.write_scene_vertices_sparse(
                        &worker.topology,
                        shadow.as_ref().map_or(&[][..], |shadow| shadow.as_slice()),
                        &touched,
                    )
                };
                rejected |= !applied;
            }
        }
        if let (Some(offscreen), Some(prepared)) = (
            self.render.offscreen.as_ref(),
            self.render.prepared_scene.as_mut(),
        ) {
            has_target = true;
            let applied = if full_sync {
                prepared
                    .write_entry_sculpt_geometry(
                        offscreen.renderer(),
                        &worker.topology,
                        geometry.as_ref().map_or(&[], |(vertices, _)| vertices),
                        geometry.as_ref().map_or(&[], |(_, indices)| indices),
                    )
                    .is_some()
            } else {
                prepared.write_entry_vertices_sparse(
                    offscreen.renderer(),
                    &worker.topology,
                    shadow.as_ref().map_or(&[][..], |shadow| shadow.as_slice()),
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

    /// Re-apply current live geometry after a live scene rebuild.
    pub(super) fn push_sculpt_shadow_live(&self) -> Option<bool> {
        let worker = self.tools.sculpt.worker.as_ref()?;
        let live_viewport = self.render.live_viewport.as_ref()?;
        let mut viewport = live_viewport.try_lock().ok()?;
        if !viewport.has_prepared_scene() {
            return None;
        }
        if !worker.has_uncommitted_geometry() {
            return Some(true);
        }
        let (vertices, indices) = worker.live_geometry()?;
        Some(
            viewport
                .write_scene_sculpt_geometry(&worker.topology, &vertices, &indices)
                .is_some(),
        )
    }

    /// Finish the drag: the worker creates the mesh off the UI thread and the
    /// next worker poll installs it as one undoable layer edit.
    pub(super) fn commit_sculpt_stroke(&mut self, ctx: &egui::Context) -> bool {
        let Some(stroke) = self.tools.sculpt.stroke.take() else {
            self.tools.sculpt.finish_retry = false;
            return true;
        };
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-worker-unavailable")),
            );
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
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-worker-unavailable")),
            );
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
        sculpted: Arc<Mesh>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return false;
        };
        let layer_id = worker.layer_id;
        let topology_id = worker.topology_id;
        let committed_topology = occluview_render::PreparedSceneTopology::from_mesh(&sculpted);
        let committed_topology_id = sculpted.topology_id();
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
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("repair-edit-busy")),
            );
            return false;
        };
        drop(scene);
        if self.commit_sculpt_scene(layer_id, sculpted, ctx) {
            if let Some(worker) = self.tools.sculpt.worker.as_mut() {
                worker.topology_id = committed_topology_id;
                worker.topology = committed_topology;
                worker.mark_geometry_committed();
            }
            let _ = self.document.edit_mode.finish_layer_edit_success(token);
            self.document.mark_mesh_edits_unsaved(layer_id);
            // Report whether the pre-edit snapshot was retained.
            self.ui.status_message = Some(if self.document.edit_mode.last_edit_undoable() {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-applied-undo"))
            } else {
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("sculpt-applied-locked"))
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
        mesh: Arc<Mesh>,
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
            entry.mesh = mesh;
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
        title: locale.tr(crate::i18n::message_id!("sculpt-failed-title")),
        summary: locale.tr_with(
            crate::i18n::message_id!("sculpt-worker-stopped"),
            &[("detail", detail.as_str())],
        ),
        details: format!("Sculpt worker stopped\n\n{detail}"),
        action: AppErrorAction::None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// A terminal worker failure has to outlive the status line, because the
    /// stroke's geometry is gone and no later result can arrive to explain it.
    #[test]
    fn a_terminal_failure_raises_the_error_dialog() {
        use crate::i18n::LocaleManager;
        use crate::sculpt_worker::SculptFailure;

        let locale = LocaleManager::for_tests();
        let failure = SculptFailure::WorkerStatePoisoned;
        let dialog = super::sculpt_failure_dialog(&locale, &failure);

        assert_eq!(
            dialog.title,
            locale.tr(crate::i18n::message_id!("sculpt-failed-title"))
        );
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
}

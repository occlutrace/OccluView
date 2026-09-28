use super::super::selection_overlay::selection_overlay_for_scene;
use super::super::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, egui, AppErrorAction,
    AppErrorDialog, GpuCamera, OccluViewApp,
};

impl OccluViewApp {
    pub(in crate::app) fn sync_live_viewport(&mut self) {
        // A rebuild uploads the scan's own colours, so a live deviation map
        // has to be pushed again or it vanishes on the next scene change.
        let restore_deviation = self.align_overlay_is_up();
        let Some(live_viewport) = self.render.live_viewport.clone() else {
            return;
        };
        if self.render.camera.is_none() {
            self.reset_camera_to_home();
        }
        let Some(scene) = self.document.scene.as_ref() else {
            self.clear_live_viewport();
            self.render.invalidation.consume_redraw();
            return;
        };
        let bbox = self.effective_scene_bbox(scene);
        let Some(mut cam) = self.render.camera else {
            return;
        };
        cam.fit_clip_planes_to_bbox(bbox);

        let [width_px, height_px] = self.render.render_extent_px;
        let aspect = f32::from(width_px) / f32::from(height_px.max(1));
        let view = build_view_matrix(&cam);
        let proj = build_proj_matrix(&cam, aspect);
        let gpu_cam = GpuCamera::new(view, proj, camera_studio_light_dir(&cam), cam.eye());
        let clip_plane = self.active_viewport_clip_plane(bbox);
        let selection_overlay_visible = self.selection_overlay_visible();

        let (repush_deviation, scene_rebuilt) = match live_viewport.lock() {
            Ok(mut viewport) => {
                viewport.set_show_ghost(self.persistence.settings.show_cut_ghost);
                let splat_viewport = self.render.live_viewport_px.unwrap_or_else(|| {
                    [
                        u32::from(self.render.render_extent_px[0]),
                        u32::from(self.render.render_extent_px[1]),
                    ]
                });
                viewport.update_view(&gpu_cam, splat_viewport, clip_plane);
                let mut rebuilt = false;
                if self.render.invalidation.live_scene_stale() {
                    let sources = self.prepared_scene_sources(scene);
                    let updates = self.prepared_scene_updates(scene);
                    // Only a real rebuild re-uploads the scan's own colours. A
                    // uniform-only reconcile leaves the map on the GPU where it
                    // was, so pushing it again would move thirty-four
                    // megabytes to write what is already there.
                    rebuilt = viewport.sync_scene(&sources, &updates);
                    self.render.invalidation.consume_live_scene();
                }
                let repush_deviation =
                    (rebuilt && restore_deviation) || self.tools.align.deviation_push_pending;
                if self.render.invalidation.live_overlay_stale() {
                    if selection_overlay_visible {
                        let overlay = selection_overlay_for_scene(scene, &self.document.edit_mode);
                        let sources = overlay.as_ref().map_or_else(
                            Vec::new,
                            super::super::selection_overlay::SelectionOverlayScene::prepared_sources,
                        );
                        viewport.sync_selection_overlay(&sources);
                    } else {
                        viewport.sync_selection_overlay(&[]);
                    }
                    self.render.invalidation.consume_live_overlay();
                }
                self.render.invalidation.consume_redraw();
                (repush_deviation, rebuilt)
            }
            Err(e) => {
                tracing::warn!(error = ?e, "live viewport lock failed");
                (false, false)
            }
        };
        if scene_rebuilt
            && self
                .tools
                .sculpt
                .worker
                .as_ref()
                .is_some_and(|_| self.push_sculpt_shadow_live() != Some(true))
        {
            if let Some(worker) = self.tools.sculpt.worker.as_ref() {
                worker.request_full_sync();
            }
        }
        if repush_deviation {
            // A push before the viewport has a prepared scene writes nowhere.
            // Keep the request standing until one exists, or the very first
            // measurement would come out in the scan's own colours.
            self.tools.align.deviation_push_pending = !self.push_deviation_colors();
        }
    }

    pub(in crate::app) fn clear_live_viewport(&self) {
        let Some(live_viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        if let Ok(mut viewport) = live_viewport.lock() {
            viewport.clear();
        }
    }

    /// Clear a latched graphics fault and try to draw again.
    ///
    /// Offered from the fault dialog. The flag exists so an unacknowledged
    /// fault stops the frame loop from feeding a broken device; it is not a
    /// verdict that the device is gone. A driver reset, a recovered eGPU, or a
    /// rebuilt offscreen device can all leave this latch set on a working
    /// renderer; without this retry the only recovery would be to close the
    /// viewer and lose the scene.
    ///
    /// Nothing is repaired here: the next frame either paints or raises the
    /// fault again, and a new message re-arms the dialog.
    pub(in crate::app) fn retry_gpu_after_fault(&mut self, ctx: &egui::Context) {
        // The offscreen latch is cleared first, whether or not a live viewport
        // exists: without one the offscreen path is the viewport, and skipping
        // it would make the only recovery the UI offers do nothing on the
        // machine that needs it.
        self.render.offscreen_failed = false;
        self.render.offscreen_retry_after = None;
        if let Some(live_viewport) = self.render.live_viewport.as_ref() {
            match live_viewport.lock() {
                Ok(mut viewport) => viewport.clear_gpu_fault(),
                Err(error) => {
                    tracing::warn!(?error, "live viewport lock failed while retrying graphics");
                    return;
                }
            }
        }
        tracing::info!("operator asked to resume drawing after a graphics fault");
        self.ui.status_message = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("gpu-retry-status")),
        );
        self.render.invalidation.request_redraw();
        ctx.request_repaint();
    }

    /// Poll the live viewport's GPU error latch once per frame. wgpu reports
    /// draw/submit validation faults and device-lost events through the
    /// installed handler instead of panicking; surface any message (status
    /// line always, copyable dialog only when no other error is showing, so a
    /// GPU that faults every frame cannot spam modal dialogs).
    pub(in crate::app) fn poll_gpu_errors(&mut self) -> bool {
        let Some(live_viewport) = self.render.live_viewport.as_ref() else {
            return false;
        };
        let error = match live_viewport.lock() {
            Ok(viewport) => viewport.take_gpu_error(),
            Err(e) => {
                tracing::warn!(error = ?e, "live viewport lock failed while polling GPU errors");
                return false;
            }
        };
        let Some(error) = error else {
            return false;
        };
        tracing::error!(gpu_error = %error, "surfacing GPU error to the operator");
        self.ui.status_message = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("gpu-failed-status")),
        );
        if self.ui.app_error.is_none() {
            self.ui.app_error = Some(AppErrorDialog {
                title: self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("gpu-failed-title")),
                summary: self
                    .ui
                    .locale
                    .tr(crate::i18n::message_id!("gpu-failed-summary")),
                details: format!("wgpu uncaptured error\n\n{error}"),
                action: AppErrorAction::RetryGraphics,
            });
        }
        true
    }
}

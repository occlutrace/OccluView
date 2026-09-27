use super::super::selection_overlay::selection_overlay_for_scene;
use super::super::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, egui, AppErrorAction,
    AppErrorDialog, Context, CutTool, GpuCamera, Instant, OccluViewApp, Offscreen, RenderedFrame,
    Result, Scene, ThumbnailSpec, ViewportSpec,
};
use super::scene::transformed_bbox;
use anyhow::Error;
use occluview_core::Aabb;
pub(in crate::app) use occluview_render::RenderError;
use occluview_render::{
    AdapterPolicy, PreparedSceneClipRequest, PreparedSceneTopology, PreparedViewportClipRequest,
    PreparedViewportRequest, RenderDeadline,
};
use std::time::Duration;

const APP_OFFSCREEN_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(8);
/// Maximum wait for one offscreen frame readback.
pub(in crate::app) const APP_OFFSCREEN_RENDER_TIMEOUT: Duration = Duration::from_secs(2);
/// Delay before retrying a readback timeout from a healthy graphics stack.
pub(in crate::app) const OFFSCREEN_RETRY_DELAY: Duration = Duration::from_millis(750);

/// Whether an offscreen failure means the graphics stack itself is unusable.
///
/// A readback deadline is not that: it measures how long this process was
/// willing to wait, and the deadline is a liveness bound rather than a device
/// verdict. The application's own timeout doc says as much. Treating it as
/// terminal would latch the whole offscreen path off for the rest of the
/// session: the section panel would keep showing the previous plane and, with
/// no live viewport, the viewport would stop repainting entirely, on a machine
/// whose GPU is fine, with no dialog or control that could clear it.
fn terminal_offscreen_render_error(error: &RenderError) -> bool {
    matches!(error, RenderError::Surface(_) | RenderError::NoAdapter)
}

/// Whether this failure can be retried at all. Only a healthy stack retried
/// after a deadline is worth another attempt; the caller backs off so a retry
/// cannot become a repaint storm.
fn retryable_offscreen_render_error(error: &RenderError) -> bool {
    matches!(error, RenderError::ReadbackTimeout { .. })
}

impl OccluViewApp {
    /// Whether a selection overlay may be drawn over the current scene.
    ///
    /// Sculpt streams a display-only worker shadow into the prepared scene
    /// while the document remains at its last committed mesh. The selection
    /// overlay has its own GPU geometry and cannot safely follow that shadow
    /// sparsely, so hiding it during Sculpt is safer than showing stale faces.
    /// The mode transition and the sculpt commit invalidate it for a rebuild.
    pub(in crate::app) fn selection_overlay_visible(&self) -> bool {
        self.tools.sculpt.armed.is_none() && self.tools.sculpt.stroke.is_none()
    }

    /// Bounds for the pixels currently shown by the renderer.
    ///
    /// During an active Sculpt stroke the prepared GPU scene contains the
    /// worker shadow, while `Scene::bbox()` still describes the committed
    /// mesh. Replacing only the worker layer's local bounds keeps Cut View,
    /// clipping and camera framing from lagging behind a large displacement.
    pub(in crate::app) fn effective_scene_bbox(&self, scene: &Scene) -> Aabb {
        let Some(worker) = self.tools.sculpt.worker.as_ref() else {
            return scene.bbox();
        };
        let shadow_handle = worker.shadow();
        let Some(shadow) = shadow_handle.try_read().ok() else {
            return scene.bbox();
        };
        let sculpted_local = Aabb::enclose_points(
            shadow
                .iter()
                .map(|vertex| glam::Vec3::from_array(vertex.position)),
        );
        if sculpted_local.is_empty()
            || !sculpted_local.min.is_finite()
            || !sculpted_local.max.is_finite()
        {
            return scene.bbox();
        }
        scene
            .meshes()
            .iter()
            .filter(|entry| entry.visible)
            .map(|entry| {
                let local = if entry.id() == worker.layer_id {
                    sculpted_local
                } else {
                    entry.mesh.bbox_cached()
                };
                transformed_bbox(local, entry.transform)
            })
            .fold(Aabb::EMPTY, Aabb::enclose_box)
    }

    pub(in crate::app) fn render_now(&mut self, ctx: &egui::Context) {
        let render_started_at = Instant::now();
        let (spec, pixels) = match self.render_scene_pixels() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!(error = ?e, "offscreen render failed");
                self.note_offscreen_failure_anyhow(&e);
                let terminal = self.render.offscreen_failed;
                // A retryable failure is transient by definition: report it in
                // the status line and keep the reason where the operator can
                // find it, but do not raise the modal. On a machine that misses
                // the deadline repeatedly, one dialog per attempt would bury the
                // viewport and offer no way out. The terminal case does raise the
                // modal, because the offscreen path is the only viewport there
                // and the operator would otherwise get a blank area with no
                // explanation — and it carries the retry action, which clears
                // this latch, so the dialog is a way out rather than a full
                // stop.
                if terminal {
                    self.ui.app_error = Some(AppErrorDialog {
                        title: self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("render-failed-title")),
                        summary: self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("render-failed-summary")),
                        details: format!("Render failed\n\n{e:#}"),
                        // Retryable here: on a machine where the offscreen path
                        // is the viewport, this dialog is the only surface the
                        // operator sees, and `AppErrorAction::None` would leave
                        // the latch unreachable from the UI.
                        // `retry_gpu_after_fault` clears the offscreen latch
                        // (and the live one when there is a live viewport), so
                        // the button acts on both paths.
                        action: AppErrorAction::RetryGraphics,
                    });
                }
                self.ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("render-failed-status")),
                );
                return;
            }
        };

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [usize::from(spec.size_px[0]), usize::from(spec.size_px[1])],
            &pixels,
        );
        // Reuse one persistent egui texture id: update it in place with
        // `TextureHandle::set` (a texture-`set` delta) rather than
        // `Context::load_texture` (a fresh id whose previous handle, dropped
        // here, emits a texture-`free`). egui-wgpu 0.29 runs `free_texture` —
        // which calls `wgpu::Texture::destroy` — after recording this frame's
        // draws but before `queue.submit`. The second render-pending pass
        // (see `state.rs`) re-renders the viewport image *after* the central
        // panel already painted it, so a fresh id would destroy the just-painted
        // texture mid-frame and `Queue::submit` fails validation ("texture ...
        // has been destroyed"). A stable id never frees a painted texture.
        if let Some(frame) = self.render.rendered.as_mut() {
            frame.texture.set(color_image, egui::TextureOptions::LINEAR);
            frame.pixels = pixels;
            frame.size_px = spec.size_px;
        } else {
            let texture =
                ctx.load_texture("occluview-mesh", color_image, egui::TextureOptions::LINEAR);
            self.render.rendered = Some(RenderedFrame {
                texture,
                pixels,
                size_px: spec.size_px,
            });
        }
        self.render.invalidation.consume_redraw();
        tracing::info!(
            width_px = spec.size_px[0],
            height_px = spec.size_px[1],
            render_ms = render_started_at.elapsed().as_millis(),
            "viewport frame rendered"
        );
    }

    pub(in crate::app) fn render_cut_now(&mut self, ctx: &egui::Context) {
        let Some(scene) = self.document.scene.clone() else {
            self.tools.cut_view.disable();
            return;
        };
        let bbox = self.effective_scene_bbox(&scene);
        let Some(cut) = self.tools.cut_view.cut_view_spec(bbox) else {
            return;
        };
        let (focus, half_extent) = self.tools.cut_view.cut_view_focus(bbox);
        let basis = self.tools.cut_view.slice_basis();
        let Some((color_image, slice_cam)) =
            self.render_section_pixels(&scene, cut.plane, focus, half_extent, basis)
        else {
            return;
        };
        self.tools.cut_view.store_slice(ctx, color_image, slice_cam);
    }

    pub(in crate::app) fn maybe_render_bridge_split_section(&mut self, ctx: &egui::Context) {
        if !(self.tools.bridge_split_active()
            && self.tools.bridge_split_section.take_needs_render()
            && self.tools.bridge_split_section.wants_offscreen_slice())
        {
            return;
        }
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let Some(frame) = self.tools.bridge_split_section.frame() else {
            return;
        };
        let bbox = self.effective_scene_bbox(&scene);
        let plane = occluview_render::ClipPlane::new(
            frame.normal().to_array(),
            frame.normal().dot(frame.pose().center),
        );
        let (focus, half_extent) = self.tools.bridge_split_section.focus(bbox);
        let basis = self.tools.bridge_split_section.slice_basis();
        let Some((color_image, slice_cam)) =
            self.render_section_pixels(&scene, plane, focus, half_extent, basis)
        else {
            return;
        };
        self.tools
            .bridge_split_section
            .store_slice(ctx, color_image, slice_cam);
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn render_section_pixels(
        &mut self,
        scene: &Scene,
        plane: occluview_render::ClipPlane,
        focus: glam::Vec3,
        half_extent: f32,
        basis: crate::cut_ruler::SliceBasis,
    ) -> Option<(egui::ColorImage, crate::cut_ruler::SliceCam)> {
        let bbox = self.effective_scene_bbox(scene);
        let restore_deviation = self.align_overlay_is_up();
        if let Err(e) = self.ensure_offscreen() {
            tracing::error!(error = ?e, "section-view offscreen init failed");
            self.note_offscreen_failure_anyhow(&e);
            return None;
        }
        let offscreen = self.render.offscreen.as_ref()?;
        let mut scene_rebuilt = false;
        if self.render.invalidation.offscreen_scene_stale() {
            let updates = self.prepared_scene_updates(scene);
            let rebuild = self
                .render
                .prepared_scene
                .as_mut()
                .is_none_or(|prepared| !prepared.update(offscreen.renderer(), &updates));
            if rebuild {
                let sources = self.prepared_scene_sources(scene);
                self.render.prepared_scene = Some(offscreen.prepare_scene(&sources));
                scene_rebuilt = true;
            }
            self.render.invalidation.consume_offscreen_scene();
        }
        if scene_rebuilt && self.push_sculpt_shadow_offscreen() != Some(true) {
            if let Some(worker) = self.tools.sculpt.worker.as_ref() {
                worker.request_full_sync();
            }
        }
        if (scene_rebuilt && restore_deviation) || self.tools.align.deviation_push_pending {
            self.tools.align.deviation_push_pending = !self.push_deviation_colors_offscreen();
        }
        let pixels = {
            let offscreen = self.render.offscreen.as_ref()?;
            let prepared = self.render.prepared_scene.as_ref()?;
            let camera = occluview_render::cut_view_camera_focused_with_up(
                &plane,
                focus,
                half_extent,
                bbox.half_diagonal(),
                basis.up,
            );
            let spec = ThumbnailSpec {
                size_px: CutTool::preview_size_px(),
                background: self.persistence.settings.viewport_background.linear(),
            };
            match pollster::block_on(offscreen.render_prepared_scene_with_clip_with_deadline(
                PreparedSceneClipRequest {
                    scene: prepared,
                    camera: &camera,
                    clip: &plane,
                    spec,
                    deadline: RenderDeadline::after(APP_OFFSCREEN_RENDER_TIMEOUT),
                },
            )) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(error = ?e, "section-view render failed");
                    self.note_offscreen_failure(&e);
                    return None;
                }
            }
        };
        let preview_size = usize::from(CutTool::preview_size_px());
        let color_image =
            egui::ColorImage::from_rgba_unmultiplied([preview_size, preview_size], &pixels);
        let slice_cam = crate::cut_ruler::SliceCam {
            focus,
            normal: glam::Vec3::from_array(plane.normal),
            half_extent,
        };
        Some((color_image, slice_cam))
    }

    pub(in crate::app) fn active_viewport_clip_plane(
        &self,
        bbox: Aabb,
    ) -> occluview_render::ClipPlane {
        if self.tools.bridge_split_active() {
            return self.tools.bridge_split_section.frame().map_or_else(
                occluview_render::ClipPlane::disabled,
                |frame| {
                    occluview_render::ClipPlane::new(
                        frame.normal().to_array(),
                        frame.normal().dot(frame.pose().center),
                    )
                },
            );
        }
        self.tools.cut_view.viewport_clip_plane(bbox)
    }

    pub(in crate::app) fn active_section_panel_rect(
        &self,
        viewport_rect: egui::Rect,
    ) -> Option<egui::Rect> {
        let visible = if self.tools.bridge_split_active() {
            self.tools.bridge_split_section.slice_visible()
        } else {
            self.tools.cut_view.is_active() && self.tools.cut_view.slice_visible()
        };
        visible.then(|| crate::cut_ruler::section_panel_rect(viewport_rect))?
    }

    pub(in crate::app) fn axis_gizmo_is_hidden(&self) -> bool {
        self.tools.cut_view.is_active() && self.tools.cut_view.slice_visible()
    }

    pub(in crate::app) fn ensure_offscreen(&mut self) -> Result<()> {
        if !self.offscreen_available() {
            // Typed, because the caller classifies the failure by its cause. A
            // bare string would fall through to the conservative "cannot
            // classify" branch and latch the path off permanently — turning the
            // deferral into the state it exists to avoid, on the first frame
            // that arrives inside the wait.
            return Err(Error::new(RenderError::ReadbackTimeout {
                timeout: OFFSCREEN_RETRY_DELAY,
            })
            .context("offscreen rendering is waiting out a retry delay"));
        }
        if self.render.offscreen_failed {
            return Err(Error::new(RenderError::Surface(
                "offscreen rendering is disabled after a previous GPU failure".to_owned(),
            )));
        }
        if self.render.offscreen.is_none() {
            self.render.offscreen = Some(
                pollster::block_on(Offscreen::new_with_adapter_policy(
                    AdapterPolicy::HardwareThenFallback,
                    RenderDeadline::after(APP_OFFSCREEN_INITIALIZATION_TIMEOUT),
                ))
                .context("initializing offscreen")?,
            );
        }
        Ok(())
    }

    // Scene preparation, overlay restoration, and readback share one frame
    // boundary; extracting them independently risks returning a mixed frame.
    #[expect(clippy::too_many_lines)]
    pub(in crate::app) fn render_scene_pixels(&mut self) -> Result<(ViewportSpec, Vec<u8>)> {
        if self.render.camera.is_none() {
            self.reset_camera_to_home();
        }
        let scene = self.document.scene.clone().context("no scene loaded")?;
        let bbox = self.effective_scene_bbox(&scene);
        let mut cam = self.render.camera.context("camera unavailable")?;
        cam.fit_clip_planes_to_bbox(bbox);
        self.ensure_offscreen()?;

        let [width_px, height_px] = self.render.render_extent_px;
        let aspect = f32::from(width_px) / f32::from(height_px.max(1));
        let view = build_view_matrix(&cam);
        let proj = build_proj_matrix(&cam, aspect);
        let gpu_cam = GpuCamera::new(view, proj, camera_studio_light_dir(&cam), cam.eye());
        let spec = ViewportSpec {
            size_px: self.render.render_extent_px,
            background: self.persistence.settings.viewport_background.linear(),
        };

        let offscreen = self
            .render
            .offscreen
            .as_ref()
            .context("offscreen unavailable")?;
        let restore_deviation = self.align_overlay_is_up();
        let mut scene_rebuilt = false;
        if self.render.invalidation.offscreen_scene_stale() {
            let updates = self.prepared_scene_updates(&scene);
            let rebuild = self
                .render
                .prepared_scene
                .as_mut()
                .is_none_or(|prepared| !prepared.update(offscreen.renderer(), &updates));
            if rebuild {
                let prepare_started_at = Instant::now();
                let sources = self.prepared_scene_sources(&scene);
                let vertex_count: usize = sources
                    .iter()
                    .map(|source| source.mesh.vertices().len())
                    .sum();
                self.render.prepared_scene = Some(offscreen.prepare_scene(&sources));
                scene_rebuilt = true;
                tracing::info!(
                    mesh_count = sources.len(),
                    vertex_count,
                    upload_ms = prepare_started_at.elapsed().as_millis(),
                    "offscreen viewport scene prepared"
                );
            }
            self.render.invalidation.consume_offscreen_scene();
        }
        if scene_rebuilt && self.push_sculpt_shadow_offscreen() != Some(true) {
            if let Some(worker) = self.tools.sculpt.worker.as_ref() {
                worker.request_full_sync();
            }
        }
        if (scene_rebuilt && restore_deviation) || self.tools.align.deviation_push_pending {
            self.tools.align.deviation_push_pending = !self.push_deviation_colors_offscreen();
        }
        if self.render.invalidation.offscreen_overlay_stale() {
            let overlay = selection_overlay_for_scene(&scene, &self.document.edit_mode);
            self.render.prepared_selection_overlay = overlay.as_ref().map(|overlay| {
                let sources = overlay.prepared_sources();
                offscreen.prepare_scene(&sources)
            });
            self.render.invalidation.consume_offscreen_overlay();
        }
        let prepared = self
            .render
            .prepared_scene
            .as_ref()
            .context("prepared scene unavailable")?;
        let selection_overlay = self
            .render
            .prepared_selection_overlay
            .as_ref()
            .filter(|_| self.selection_overlay_visible());
        let clip_plane = self.active_viewport_clip_plane(bbox);
        let pixels = if clip_plane.enabled != 0 {
            pollster::block_on(
                offscreen.render_prepared_viewport_with_clip_and_overlay_with_deadline(
                    PreparedViewportClipRequest {
                        scene: prepared,
                        overlay: selection_overlay,
                        camera: &gpu_cam,
                        clip: &clip_plane,
                        spec,
                        show_ghost: self.persistence.settings.show_cut_ghost,
                        deadline: RenderDeadline::after(APP_OFFSCREEN_RENDER_TIMEOUT),
                    },
                ),
            )
        } else {
            pollster::block_on(
                offscreen.render_prepared_viewport_with_overlay_with_deadline(
                    PreparedViewportRequest {
                        scene: prepared,
                        overlay: selection_overlay,
                        camera: &gpu_cam,
                        spec,
                        deadline: RenderDeadline::after(APP_OFFSCREEN_RENDER_TIMEOUT),
                    },
                ),
            )
        }
        .context("rendering viewport")?;
        Ok((spec, pixels))
    }

    /// Consume the redraw that triggered a failed fallback render and decide
    /// what the path owes next.
    ///
    /// Keeping the request pending would make egui call the same failed submit
    /// forever, hiding the original cause behind a repaint storm and burning a
    /// CPU core, so a failure always consumes the redraw. What differs is what
    /// happens after: a broken graphics stack latches the path off until the
    /// operator restarts, while a missed readback deadline only defers the next
    /// attempt. Latching a deadline off for the session would leave the section
    /// panel showing a previous plane and, with no live viewport, stop the
    /// viewport repainting at all, on hardware never shown to be broken.
    pub(in crate::app) fn note_offscreen_failure_anyhow(&mut self, error: &Error) {
        if let Some(render_error) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<RenderError>())
        {
            self.note_offscreen_failure(render_error);
            return;
        }
        // An error with no typed render cause is not something this path can
        // classify; keep the conservative behaviour.
        self.render.invalidation.consume_redraw();
        self.render.offscreen_failed = true;
        self.render.offscreen_retry_after = None;
    }

    /// Whether the offscreen path is allowed to run right now.
    ///
    /// A terminal failure keeps it off; a deferred retry waits out its delay so
    /// a loaded machine cannot be asked to fail on every repaint.
    pub(in crate::app) fn offscreen_available(&self) -> bool {
        if self.render.offscreen_failed {
            return false;
        }
        match self.render.offscreen_retry_after {
            Some(deadline) => Instant::now() >= deadline,
            None => true,
        }
    }

    pub(in crate::app) fn note_offscreen_failure(&mut self, error: &RenderError) {
        self.render.invalidation.consume_redraw();
        if terminal_offscreen_render_error(error) {
            self.render.offscreen_failed = true;
            self.render.offscreen_retry_after = None;
            return;
        }
        if retryable_offscreen_render_error(error) {
            self.render.offscreen_retry_after = Some(Instant::now() + OFFSCREEN_RETRY_DELAY);
        }
    }

    /// Replay the display-only deviation colours into the prepared offscreen
    /// vertex buffer. The live viewport has an equivalent sparse/full upload
    /// path, but the fallback renderer keeps its own prepared scene and would
    /// otherwise upload the scan's original colours whenever it rebuilt.
    ///
    /// The CPU mesh remains untouched: only the cached GPU vertices are
    /// rewritten, and clearing a map writes the original vertices back once.
    pub(in crate::app) fn push_deviation_colors_offscreen(&self) -> bool {
        let (Some(scene), Some(offscreen), Some(prepared)) = (
            self.document.scene.clone(),
            self.render.offscreen.as_ref(),
            self.render.prepared_scene.as_ref(),
        ) else {
            return false;
        };
        let pending = self.tools.align.overlay_colors.clone();
        if pending.is_empty() {
            let mut wrote = true;
            for entry in scene.meshes() {
                let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
                wrote &= prepared.write_entry_vertices(
                    offscreen.renderer(),
                    &topology,
                    entry.mesh.vertices(),
                );
            }
            return wrote;
        }

        let mut wrote = true;
        for (layer, colors) in pending {
            let Some(entry) = super::super::app_align::layer_of(&scene, layer) else {
                wrote = false;
                continue;
            };
            if colors.len() != entry.mesh.vertices().len() {
                wrote = false;
                continue;
            }
            let mut vertices = entry.mesh.vertices().to_vec();
            for (vertex, color) in vertices.iter_mut().zip(colors.iter()) {
                vertex.color = *color;
            }
            let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
            wrote &= prepared.write_entry_vertices(offscreen.renderer(), &topology, &vertices);
        }
        wrote
    }
}

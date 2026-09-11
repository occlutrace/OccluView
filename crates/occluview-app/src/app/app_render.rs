//! Turning the current scene into pixels, through whichever path is available.
//!
//! Two paths draw the same scene with the same `occluview-render` pipeline: an
//! eframe/wgpu paint callback into the live surface, and an offscreen render
//! whose result is blitted as an egui texture. The live path is used when the
//! backend gave us one; the offscreen path is the fallback and is also what
//! produces the cut-view preview.
//!
//! Both consume their own [`crate::invalidation::RenderInvalidation`]
//! cursors documented in [`super::state_render`] where they rebuild. Each path caches its own
//! `PreparedScene`, so a scene change stales both or the untouched path keeps
//! drawing the previous geometry.

use super::selection_overlay::selection_overlay_for_scene;
use super::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, egui, live_viewport,
    paint_axis_gizmo, paint_scale_bar, AppErrorDialog, Arc, AxisGizmoInput, Context, CutTool,
    GpuCamera, GpuMeshUniform, Instant, Mat4, OccluViewApp, Offscreen, PreparedSceneSource,
    PreparedSceneTopology, PreparedSceneUpdate, RenderedFrame, Result, Scene, SceneMesh,
    ThumbnailSpec, ViewportSpec,
};
use occluview_core::Aabb;
use occluview_render::{
    AdapterPolicy, PreparedSceneClipRequest, PreparedViewportClipRequest, PreparedViewportRequest,
    RenderDeadline,
};
use std::time::Duration;

const APP_OFFSCREEN_RENDER_TIMEOUT: Duration = Duration::from_secs(2);
const APP_OFFSCREEN_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(8);

impl OccluViewApp {
    /// Whether a selection overlay may be drawn over the current scene.
    ///
    /// Sculpt streams a display-only worker shadow into the prepared scene
    /// while the document remains at its last committed mesh. The selection
    /// overlay has its own GPU geometry and cannot safely follow that shadow
    /// sparsely, so hiding it during Sculpt is safer than showing stale faces.
    /// The mode transition and the sculpt commit invalidate it for a rebuild.
    fn selection_overlay_visible(&self) -> bool {
        self.tools.sculpt.armed.is_none() && self.tools.sculpt.stroke.is_none()
    }

    /// Bounds for the pixels currently shown by the renderer.
    ///
    /// During an active Sculpt stroke the prepared GPU scene contains the
    /// worker shadow, while `Scene::bbox()` still describes the committed
    /// mesh. Replacing only the worker layer's local bounds keeps Cut View,
    /// clipping and camera framing from lagging behind a large displacement.
    fn effective_scene_bbox(&self, scene: &Scene) -> Aabb {
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

    pub(super) fn render_now(&mut self, ctx: &egui::Context) {
        let render_started_at = Instant::now();
        let (spec, pixels) = match self.render_scene_pixels() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!(error = ?e, "offscreen render failed");
                self.ui.app_error = Some(AppErrorDialog {
                    title: self.ui.locale.tr("render-failed-title"),
                    summary: self.ui.locale.tr("render-failed-summary"),
                    details: format!("Render failed\n\n{e:#}"),
                });
                self.ui.status_message = Some(self.ui.locale.tr("render-failed-status"));
                return;
            }
        };

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [usize::from(spec.size_px[0]), usize::from(spec.size_px[1])],
            &pixels,
        );
        // Reuse ONE persistent egui texture id: update it in place with
        // `TextureHandle::set` (a texture-`set` delta) rather than
        // `Context::load_texture` (a fresh id whose previous handle, dropped
        // here, emits a texture-`free`). egui-wgpu 0.29 runs `free_texture` —
        // which calls `wgpu::Texture::destroy` — AFTER recording this frame's
        // draws but BEFORE `queue.submit`. The second render-pending pass
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

    pub(super) fn render_cut_now(&mut self, ctx: &egui::Context) {
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

    pub(super) fn maybe_render_bridge_split_section(&mut self, ctx: &egui::Context) {
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
    fn render_section_pixels(
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
            return None;
        }
        let offscreen = self.render.offscreen.as_ref()?;
        let mut scene_rebuilt = false;
        if self.render.invalidation.offscreen_scene_stale() {
            let updates = prepared_scene_updates(scene);
            let rebuild = self
                .render
                .prepared_scene
                .as_mut()
                .is_none_or(|prepared| !prepared.update(offscreen.renderer(), &updates));
            if rebuild {
                let sources = prepared_scene_sources(scene);
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

    fn active_viewport_clip_plane(&self, bbox: Aabb) -> occluview_render::ClipPlane {
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

    pub(super) fn active_section_panel_rect(
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

    pub(super) fn axis_gizmo_is_hidden(&self) -> bool {
        self.tools.cut_view.is_active() && self.tools.cut_view.slice_visible()
    }

    pub(super) fn ensure_offscreen(&mut self) -> Result<()> {
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
    pub(super) fn render_scene_pixels(&mut self) -> Result<(ViewportSpec, Vec<u8>)> {
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
            let updates = prepared_scene_updates(&scene);
            let rebuild = self
                .render
                .prepared_scene
                .as_mut()
                .is_none_or(|prepared| !prepared.update(offscreen.renderer(), &updates));
            if rebuild {
                let prepare_started_at = Instant::now();
                let sources = prepared_scene_sources(&scene);
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

    /// Replay the display-only deviation colours into the prepared offscreen
    /// vertex buffer. The live viewport has an equivalent sparse/full upload
    /// path, but the fallback renderer keeps its own prepared scene and would
    /// otherwise upload the scan's original colours whenever it rebuilt.
    ///
    /// The CPU mesh remains untouched: only the cached GPU vertices are
    /// rewritten, and clearing a map writes the original vertices back once.
    fn push_deviation_colors_offscreen(&self) -> bool {
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
            let Some(entry) = super::app_align::layer_of(&scene, layer) else {
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

    pub(super) fn sync_live_viewport(&mut self) {
        // A rebuild uploads the scan's own colours, so a live deviation map
        // has to be pushed again or it silently vanishes on the next scene
        // change.
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
                viewport.update_view(&gpu_cam, self.render.render_extent_px, clip_plane);
                let mut rebuilt = false;
                if self.render.invalidation.live_scene_stale() {
                    let sources = prepared_scene_sources(scene);
                    let updates = prepared_scene_updates(scene);
                    // Only a real rebuild re-uploads the scan's own colours. A
                    // uniform-only reconcile leaves the map on the GPU exactly
                    // where it was, so pushing it again would move thirty-four
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
                            super::selection_overlay::SelectionOverlayScene::prepared_sources,
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

    pub(super) fn clear_live_viewport(&self) {
        let Some(live_viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        if let Ok(mut viewport) = live_viewport.lock() {
            viewport.clear();
        }
    }

    /// Poll the live viewport's GPU error latch once per frame. wgpu reports
    /// draw/submit validation faults and device-lost events through the handler
    /// we installed instead of panicking; surface any message honestly (status
    /// line always, copyable dialog only when no other error is showing, so a
    /// GPU that faults every frame cannot spam modal dialogs).
    pub(super) fn poll_gpu_errors(&mut self) {
        let Some(live_viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        let error = match live_viewport.lock() {
            Ok(viewport) => viewport.take_gpu_error(),
            Err(e) => {
                tracing::warn!(error = ?e, "live viewport lock failed while polling GPU errors");
                return;
            }
        };
        let Some(error) = error else {
            return;
        };
        tracing::error!(gpu_error = %error, "surfacing GPU error to the operator");
        self.ui.status_message = Some(self.ui.locale.tr("gpu-failed-status"));
        if self.ui.app_error.is_none() {
            self.ui.app_error = Some(AppErrorDialog {
                title: self.ui.locale.tr("gpu-failed-title"),
                summary: self.ui.locale.tr("gpu-failed-summary"),
                details: format!("wgpu uncaptured error\n\n{error}"),
            });
        }
    }

    pub(super) fn set_scene(&mut self, scene: Scene, reset_camera: bool) {
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.document.edit_mode.sync_to_scene(&scene);
        // A structural scene swap (load, delete, another mesh edit, undo/redo)
        // reverts the geometry the persistent sculpt session was prepared over,
        // WITHOUT necessarily changing topology_id (a sculpt commit preserves
        // it), so drop the session here and re-prepare on the next stroke.
        self.tools.sculpt.invalidate_session();
        self.document.scene = Some(Arc::new(scene));
        self.clear_live_viewport();
        self.render.prepared_scene = None;
        self.render.prepared_selection_overlay = None;
        if reset_camera {
            self.reset_camera_to_home();
        }
        self.render.invalidation.scene_geometry_changed();
        self.document.mesh_selection_drag = None;
        self.render.rendered = None;
        // Whatever the align tool was showing described the geometry that just
        // got replaced. Undo and redo already dropped it by hand; every other
        // structural path — repair, close holes, crop, cut, separate, a bridge
        // split commit, a cancelled mesh-edit session — did not, so a repaired
        // scan kept a map of its own former surface, lost its tint to the map
        // shading, and the panel went on reporting a percentage for a surface
        // that no longer existed. Hoisted to the one place they all pass through.
        self.forget_align_fit(&self.ui.locale.tr("align-status-scan-changed"));
        // Structural scene change: world anchors may now dangle over deleted or
        // replaced geometry, so measurements are cleared (the tool stays armed
        // while something remains to measure). Material-only updates keep them
        // (world space is unchanged).
        self.tools.measure.clear_measurements();
        if !self.has_measurable_layer() {
            self.tools.measure.disarm();
        }
        if self.can_render_cut_view() {
            // A planted disc holds a WORLD-space plane. Scanner vendors place
            // models at wildly different origins, so a plane kept across a
            // scene replace usually leaves the new case entirely on the
            // clipped-away side, drawing as a faint ghost — which reads as "the
            // file loaded wrong". Re-arm instead: the tool stays on, the stale
            // placement does not. Bridge split already does this three lines
            // above; only the cut view was left behind.
            if self.tools.cut_view.is_active() {
                self.tools.cut_view.enable();
            }
            self.tools.cut_view.mark_dirty();
        } else {
            self.tools.cut_view.disable();
        }
    }

    pub(super) fn update_scene_materials(&mut self, scene: Scene) {
        self.document.scene = Some(Arc::new(scene));
        self.mark_scene_materials_changed();
    }

    /// The bookkeeping a material change needs, for a caller that already owns
    /// the live scene and mutated it in place.
    pub(super) fn mark_scene_materials_changed(&mut self) {
        if let Some(scene) = self.document.scene.clone() {
            self.document.edit_mode.sync_to_scene(&scene);
        }
        self.render.invalidation.scene_geometry_changed();
        self.document.mesh_selection_drag = None;
        if self.can_render_cut_view() {
            self.tools.cut_view.mark_dirty();
        } else {
            self.tools.cut_view.disable();
        }
    }

    pub(super) fn clear_scene(&mut self) {
        // The last layer can disappear while Align Meshes is armed. Revoke its
        // pose, overlay, mask, and worker generation before a new scene may
        // reuse one of the old layer ids.
        self.reset_align_state_for_scene_clear();
        // A clear has no replacement scene to validate against. Revoke the
        // persistent Sculpt worker before dropping the scene so a background
        // completion cannot outlive this generation and be mistaken for the
        // next file's layer.
        self.tools.sculpt.invalidate_session();
        self.document.clear_unsaved_mesh_edits();
        self.document.hidden_layer_stack.clear();
        self.document.translucent_layer_restore.clear();
        self.document.scene = None;
        self.clear_live_viewport();
        self.render.prepared_scene = None;
        self.render.prepared_selection_overlay = None;
        self.persistence.current_paths.clear();
        self.render.camera = None;
        self.render.rendered = None;
        self.render.invalidation.reset();
        self.document.mesh_selection_drag = None;
        self.document.load_queue_camera_reset = super::LoadQueueCameraReset::Idle;
        self.document.camera_modified_during_load = false;
        self.document.edit_mode.clear();
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.tools.cut_view.disable();
        self.tools.measure.disarm();
        self.render.section_cache.clear();
    }

    pub(super) fn show_central_panel(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        // The default CentralPanel carries an 8 px inner margin. That leaves a
        // visible strip between the application chrome and the render surface;
        // this panel owns the viewport background, so it must be edge-to-edge.
        egui::CentralPanel::no_frame().show(root_ui, |ui| {
            ui.painter().rect_filled(
                ui.max_rect(),
                0.0,
                self.persistence.settings.viewport_background.srgb(),
            );
            self.sync_render_extent(ui.available_size(), ctx.pixels_per_point());
            let live_viewport = self.render.live_viewport.clone();
            if let Some(live_viewport) = live_viewport {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let response = ui.allocate_rect(viewport_rect, egui::Sense::click_and_drag());
                ui.painter()
                    .add(live_viewport::paint_callback(response.rect, live_viewport));
                self.show_viewport_overlays(ui, &response, &ctx);
            } else if let Some(texture) = self
                .render
                .rendered
                .as_ref()
                .map(|rendered| rendered.texture.clone())
            {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let response = ui.put(
                    viewport_rect,
                    egui::Image::new((texture.id(), available))
                        .sense(egui::Sense::click_and_drag()),
                );
                self.show_viewport_overlays(ui, &response, &ctx);
            } else if self.document.scene.is_none() {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let response = ui.allocate_rect(viewport_rect, egui::Sense::click());
                Self::set_drop_hover_cursor_if_hovering(&ctx);
                self.show_empty_state(ui, &response, &ctx);
                self.show_status_overlay(ui, viewport_rect);
            } else {
                ui.spinner();
            }
        });
    }

    /// Every overlay the viewport draws, and the input arbitration that
    /// follows them.
    ///
    /// One body, called by both branches of `show_central_panel_impl`. Written
    /// twice, every new tool has to be wired into both copies with nothing to
    /// say when one is missed, and the one that gets missed is the offscreen
    /// copy: it never runs on a developer machine, only for operators whose
    /// driver could not give the app a live viewport, who are the people least
    /// able to diagnose "the Align button does nothing". The branches differ
    /// only in how they obtain `response`.
    fn show_viewport_overlays(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        // While files hover anywhere over the window the viewport advertises
        // itself as the drop target, without painting a border over the model.
        Self::set_drop_hover_cursor_if_hovering(ctx);
        if self.document.scene.is_none() {
            // No scene yet: a quiet centered call to action over the clear
            // color. The overlays below are all camera/scene-gated, so the
            // right-click scene menu keeps working untouched.
            self.show_empty_state(ui, response, ctx);
        }
        let mut axis_snap = None;
        if let Some(camera) = self.render.camera.as_ref() {
            paint_scale_bar(
                ui,
                response.rect,
                camera,
                self.persistence.settings.unit_display,
                self.persistence.settings.viewport_background,
            );
        }
        if let Some(camera) = self.render.camera.as_ref() {
            let gizmo_hidden = self.axis_gizmo_is_hidden();
            if !gizmo_hidden {
                let gizmo_avoid = self.active_section_panel_rect(response.rect);
                axis_snap = paint_axis_gizmo(AxisGizmoInput {
                    ui,
                    image_rect: response.rect,
                    camera,
                    response,
                    avoid: gizmo_avoid,
                    background: self.persistence.settings.viewport_background,
                });
            }
        }
        self.show_layers_overlay(ui, response.rect, ctx);
        self.show_mesh_editor_overlay(response.rect, ctx);
        self.paint_mesh_selection_drag_overlay_impl(ui);
        self.show_status_overlay(ui, response.rect);
        let bridge_ui_consumed = self.show_bridge_split_overlay(ui, response, ctx);
        let cut_ui_consumed = self.show_cut_tool_overlay(ui, response.rect, ctx);
        // A click the axis gizmo snapped on never doubles as a measure anchor.
        let align_ui_consumed =
            self.show_align_tool_overlay(ui, response, axis_snap.is_some(), ctx);
        let measure_ui_consumed =
            self.show_measure_tool_overlay(ui, response, axis_snap.is_some(), ctx);
        if let Some(axis) = axis_snap {
            if let Some(camera) = self.render.camera.as_mut() {
                camera.snap_to_axis(axis);
                self.render.invalidation.request_redraw();
                ctx.request_repaint();
            }
        }
        if !bridge_ui_consumed && !cut_ui_consumed && !measure_ui_consumed && !align_ui_consumed {
            self.handle_viewport_input(ctx, response, response.rect, axis_snap.is_some());
        }
        // Input resolves and caches the authoritative sculpt hit first. The
        // visual cursor then reuses it for held drags and publishes its GPU
        // uniforms before the callback's render pass executes.
        self.paint_sculpt_cursor_impl(ui, response);
    }

    pub(super) fn render_pending_frame(&mut self, ctx: &egui::Context) {
        if self.render.invalidation.redraw_pending() {
            if self.render.live_viewport.is_some() {
                self.sync_live_viewport();
            } else {
                self.render_now(ctx);
            }
            ctx.request_repaint();
        }
    }
}

/// Transform a local AABB conservatively for camera and section framing.
fn transformed_bbox(local: Aabb, transform: glam::Affine3A) -> Aabb {
    if local.is_empty() {
        return Aabb::EMPTY;
    }
    let corners = [
        local.min,
        glam::Vec3::new(local.min.x, local.min.y, local.max.z),
        glam::Vec3::new(local.min.x, local.max.y, local.min.z),
        glam::Vec3::new(local.min.x, local.max.y, local.max.z),
        glam::Vec3::new(local.max.x, local.min.y, local.min.z),
        glam::Vec3::new(local.max.x, local.min.y, local.max.z),
        glam::Vec3::new(local.max.x, local.max.y, local.min.z),
        local.max,
    ];
    corners
        .into_iter()
        .map(|corner| transform.transform_point3(corner))
        .fold(Aabb::EMPTY, Aabb::enclose_point)
}

pub(super) fn scene_mesh_uniform(entry: &SceneMesh) -> GpuMeshUniform {
    // Derived from the overlay rather than stored beside it, so the two can
    // never disagree: a layer draws as a measured map exactly when it carries
    // one, and that same condition forces its colors on and its texture off.
    let deviation = entry.deviation_colors().is_some();
    GpuMeshUniform {
        model: Mat4::from(entry.transform).to_cols_array(),
        tint: entry.tint,
        opacity: entry.opacity,
        has_texture: u32::from(entry.mesh.texture().is_some()),
        show_orientation: u32::from(entry.show_orientation),
        show_vertex_colors: u32::from(entry.show_vertex_colors || deviation),
        show_texture: u32::from(entry.show_texture && !deviation),
        measured_map: u32::from(deviation),
        padding: [0; 2],
    }
}

pub(super) fn prepared_scene_sources(scene: &Scene) -> Vec<PreparedSceneSource<'_>> {
    scene
        .meshes()
        .iter()
        .map(|entry| PreparedSceneSource {
            mesh: &entry.mesh,
            uniform: scene_mesh_uniform(entry),
            visible: entry.visible,
            wireframe: entry.wireframe,
        })
        .collect()
}

pub(super) fn prepared_scene_updates(scene: &Scene) -> Vec<PreparedSceneUpdate> {
    scene
        .meshes()
        .iter()
        .map(|entry| PreparedSceneUpdate {
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            uniform: scene_mesh_uniform(entry),
            visible: entry.visible,
            wireframe: entry.wireframe,
        })
        .collect()
}

#[cfg(test)]
#[path = "app_render_tests.rs"]
mod tests;

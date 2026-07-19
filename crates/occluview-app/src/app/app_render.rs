use super::selection_overlay::selection_overlay_for_scene;
use super::{
    build_proj_matrix, build_view_matrix, camera_studio_light_dir, egui, live_viewport,
    paint_axis_gizmo, paint_scale_bar, AppErrorDialog, Arc, Context, CutTool, GpuCamera,
    GpuMeshUniform, Instant, Mat4, OccluViewApp, Offscreen, PreparedSceneSource,
    PreparedSceneTopology, PreparedSceneUpdate, RenderedFrame, Result, Scene, SceneMesh,
    SceneStats, ThumbnailSpec, ViewportSpec, VIEWPORT_BACKGROUND_LINEAR,
};

impl OccluViewApp {
    pub(super) fn render_now_impl(&mut self, ctx: &egui::Context) {
        let render_started_at = Instant::now();
        let (spec, pixels, stats) = match self.render_scene_pixels() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!(error = ?e, "offscreen render failed");
                self.app_error = Some(AppErrorDialog {
                    title: "Could not render scene".to_string(),
                    summary: "The file opened, but the viewport could not be rendered.".to_string(),
                    details: format!("Render failed\n\n{e:#}"),
                });
                self.status_message = Some("Render failed".to_string());
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
        if let Some(frame) = self.rendered.as_mut() {
            frame.texture.set(color_image, egui::TextureOptions::LINEAR);
            frame.pixels = pixels;
            frame.size_px = spec.size_px;
            frame.stats = stats;
        } else {
            let texture =
                ctx.load_texture("occluview-mesh", color_image, egui::TextureOptions::LINEAR);
            self.rendered = Some(RenderedFrame {
                texture,
                pixels,
                size_px: spec.size_px,
                stats,
            });
        }
        self.needs_render = false;
        tracing::info!(
            width_px = spec.size_px[0],
            height_px = spec.size_px[1],
            render_ms = render_started_at.elapsed().as_millis(),
            "viewport frame rendered"
        );
    }

    pub(super) fn render_cut_now_impl(&mut self, ctx: &egui::Context) {
        let Some(scene) = self.scene.clone() else {
            self.cut_view.disable();
            return;
        };
        let bbox = scene.bbox();
        let Some(cut) = self.cut_view.cut_view_spec(bbox) else {
            return;
        };
        let (focus, half_extent) = self.cut_view.cut_view_focus(bbox);
        let Some((color_image, slice_cam)) =
            self.render_section_pixels(&scene, cut.plane, focus, half_extent)
        else {
            return;
        };
        self.cut_view.store_slice(ctx, color_image, slice_cam);
    }

    pub(super) fn maybe_render_bridge_split_section(&mut self, ctx: &egui::Context) {
        if !(self.bridge_split_active()
            && self.bridge_split_section.take_needs_render()
            && self.bridge_split_section.wants_offscreen_slice())
        {
            return;
        }
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(frame) = self.bridge_split_section.frame() else {
            return;
        };
        let bbox = scene.bbox();
        let plane = occluview_render::ClipPlane::new(
            frame.normal().to_array(),
            frame.normal().dot(frame.pose().center),
        );
        let (focus, half_extent) = self.bridge_split_section.focus(bbox);
        let Some((color_image, slice_cam)) =
            self.render_section_pixels(&scene, plane, focus, half_extent)
        else {
            return;
        };
        self.bridge_split_section
            .store_slice(ctx, color_image, slice_cam);
    }

    fn render_section_pixels(
        &mut self,
        scene: &Scene,
        plane: occluview_render::ClipPlane,
        focus: glam::Vec3,
        half_extent: f32,
    ) -> Option<(egui::ColorImage, crate::cut_ruler::SliceCam)> {
        let bbox = scene.bbox();
        if let Err(e) = self.ensure_offscreen() {
            tracing::error!(error = ?e, "section-view offscreen init failed");
            return None;
        }
        let offscreen = self.offscreen.as_ref()?;
        if self.offscreen_scene_dirty {
            let updates = prepared_scene_updates(scene);
            let rebuild = self
                .prepared_scene
                .as_mut()
                .is_none_or(|prepared| !prepared.update(offscreen.renderer(), &updates));
            if rebuild {
                let sources = prepared_scene_sources(scene);
                self.prepared_scene = Some(offscreen.prepare_scene(&sources));
            }
            self.offscreen_scene_dirty = false;
        }
        let pixels = {
            let offscreen = self.offscreen.as_ref()?;
            let prepared = self.prepared_scene.as_ref()?;
            let camera = occluview_render::cut_view_camera_focused(
                &plane,
                focus,
                half_extent,
                bbox.half_diagonal(),
            );
            let spec = ThumbnailSpec {
                size_px: CutTool::preview_size_px(),
                background: VIEWPORT_BACKGROUND_LINEAR,
            };
            match pollster::block_on(
                offscreen.render_prepared_scene_with_clip(prepared, &camera, &plane, spec),
            ) {
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

    fn active_viewport_clip_plane(
        &self,
        bbox: occluview_core::Aabb,
    ) -> occluview_render::ClipPlane {
        if self.bridge_split_active() {
            return self.bridge_split_section.frame().map_or_else(
                occluview_render::ClipPlane::disabled,
                |frame| {
                    occluview_render::ClipPlane::new(
                        frame.normal().to_array(),
                        frame.normal().dot(frame.pose().center),
                    )
                },
            );
        }
        self.cut_view.viewport_clip_plane(bbox)
    }

    fn active_section_panel_rect(&self, viewport_rect: egui::Rect) -> Option<egui::Rect> {
        let visible = if self.bridge_split_active() {
            self.bridge_split_section.slice_visible()
        } else {
            self.cut_view.is_active() && self.cut_view.slice_visible()
        };
        visible.then(|| crate::cut_ruler::section_panel_rect(viewport_rect))?
    }

    pub(super) fn ensure_offscreen_impl(&mut self) -> Result<()> {
        if self.offscreen.is_none() {
            self.offscreen = Some(
                pollster::block_on(Offscreen::new_prefer_hardware())
                    .context("initializing offscreen")?,
            );
        }
        Ok(())
    }

    pub(super) fn render_scene_pixels_impl(
        &mut self,
    ) -> Result<(ViewportSpec, Vec<u8>, SceneStats)> {
        if self.camera.is_none() {
            self.reset_camera_to_home();
        }
        let scene = self.scene.clone().context("no scene loaded")?;
        let mut cam = self.camera.context("camera unavailable")?;
        cam.fit_clip_planes_to_bbox(scene.bbox());
        self.ensure_offscreen()?;

        let [width_px, height_px] = self.render_extent_px;
        let aspect = f32::from(width_px) / f32::from(height_px.max(1));
        let view = build_view_matrix(&cam);
        let proj = build_proj_matrix(&cam, aspect);
        let gpu_cam = GpuCamera::new(view, proj, camera_studio_light_dir(&cam), cam.eye());
        let spec = ViewportSpec {
            size_px: self.render_extent_px,
            background: VIEWPORT_BACKGROUND_LINEAR,
        };

        let offscreen = self.offscreen.as_ref().context("offscreen unavailable")?;
        if self.offscreen_scene_dirty {
            let updates = prepared_scene_updates(&scene);
            let rebuild = self
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
                self.prepared_scene = Some(offscreen.prepare_scene(&sources));
                tracing::info!(
                    mesh_count = sources.len(),
                    vertex_count,
                    upload_ms = prepare_started_at.elapsed().as_millis(),
                    "offscreen viewport scene prepared"
                );
            }
            self.offscreen_scene_dirty = false;
        }
        if self.selection_overlay_dirty {
            let overlay = selection_overlay_for_scene(&scene, &self.edit_mode);
            self.prepared_selection_overlay = overlay.as_ref().map(|overlay| {
                let sources = overlay.prepared_sources();
                offscreen.prepare_scene(&sources)
            });
            self.selection_overlay_dirty = false;
        }
        let prepared = self
            .prepared_scene
            .as_ref()
            .context("prepared scene unavailable")?;
        let selection_overlay = self.prepared_selection_overlay.as_ref();
        let clip_plane = self.active_viewport_clip_plane(scene.bbox());
        let pixels = if clip_plane.enabled != 0 {
            pollster::block_on(offscreen.render_prepared_viewport_with_clip_and_overlay(
                prepared,
                selection_overlay,
                &gpu_cam,
                &clip_plane,
                spec,
            ))
        } else {
            pollster::block_on(offscreen.render_prepared_viewport_with_overlay(
                prepared,
                selection_overlay,
                &gpu_cam,
                spec,
            ))
        }
        .context("rendering viewport")?;
        let stats = self.scene_stats.context("scene stats unavailable")?;
        Ok((spec, pixels, stats))
    }

    pub(super) fn sync_live_viewport_impl(&mut self) {
        let Some(live_viewport) = self.live_viewport.clone() else {
            return;
        };
        if self.camera.is_none() {
            self.reset_camera_to_home();
        }
        let Some(scene) = self.scene.as_ref() else {
            self.clear_live_viewport();
            self.needs_render = false;
            return;
        };
        let Some(mut cam) = self.camera else {
            return;
        };
        cam.fit_clip_planes_to_bbox(scene.bbox());

        let [width_px, height_px] = self.render_extent_px;
        let aspect = f32::from(width_px) / f32::from(height_px.max(1));
        let view = build_view_matrix(&cam);
        let proj = build_proj_matrix(&cam, aspect);
        let gpu_cam = GpuCamera::new(view, proj, camera_studio_light_dir(&cam), cam.eye());
        let clip_plane = self.active_viewport_clip_plane(scene.bbox());

        match live_viewport.lock() {
            Ok(mut viewport) => {
                viewport.update_view(&gpu_cam, self.render_extent_px, clip_plane);
                if self.live_viewport_scene_dirty {
                    let sources = prepared_scene_sources(scene);
                    let updates = prepared_scene_updates(scene);
                    viewport.sync_scene(&sources, &updates);
                    self.live_viewport_scene_dirty = false;
                }
                if self.selection_overlay_dirty {
                    let overlay = selection_overlay_for_scene(scene, &self.edit_mode);
                    let sources = overlay.as_ref().map_or_else(
                        Vec::new,
                        super::selection_overlay::SelectionOverlayScene::prepared_sources,
                    );
                    viewport.sync_selection_overlay(&sources);
                    self.selection_overlay_dirty = false;
                }
                self.needs_render = false;
            }
            Err(e) => {
                tracing::warn!(error = ?e, "live viewport lock failed");
            }
        };
    }

    pub(super) fn clear_live_viewport_impl(&self) {
        let Some(live_viewport) = self.live_viewport.as_ref() else {
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
    pub(super) fn poll_gpu_errors_impl(&mut self) {
        let Some(live_viewport) = self.live_viewport.as_ref() else {
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
        self.status_message = Some("Graphics driver reported a problem".to_string());
        if self.app_error.is_none() {
            self.app_error = Some(AppErrorDialog {
                title: "Graphics problem".to_string(),
                summary: "The graphics driver reported a problem while drawing. The view may \
                          be incomplete. Saving your work and restarting OccluView is \
                          recommended if it keeps happening."
                    .to_string(),
                details: format!("wgpu uncaptured error\n\n{error}"),
            });
        }
    }

    pub(super) fn set_scene_impl(&mut self, scene: Scene, reset_camera: bool) {
        self.bridge_split.cancel();
        self.bridge_split_disc.disarm();
        self.bridge_split_section.reset();
        self.edit_mode.sync_to_scene(&scene);
        // A structural scene swap (load, delete, another mesh edit, undo/redo)
        // reverts the geometry the persistent sculpt session was prepared over,
        // WITHOUT necessarily changing topology_id (a sculpt commit preserves
        // it), so drop the session here and re-prepare on the next stroke.
        self.sculpt.invalidate_session();
        let stats = scene_stats(&scene);
        self.scene = Some(Arc::new(scene));
        self.scene_stats = Some(stats);
        self.clear_live_viewport();
        self.prepared_scene = None;
        self.prepared_selection_overlay = None;
        if reset_camera {
            self.reset_camera_to_home();
        }
        self.needs_render = true;
        self.live_viewport_scene_dirty = self.live_viewport.is_some();
        self.offscreen_scene_dirty = true;
        self.selection_overlay_dirty = true;
        self.mesh_selection_drag = None;
        self.rendered = None;
        // Structural scene change: world anchors may now dangle over deleted or
        // replaced geometry, so measurements are cleared (the tool stays armed
        // while something remains to measure). Material-only updates keep them
        // (world space is unchanged).
        self.measure.clear_measurements();
        if !self.has_measurable_layer() {
            self.measure.disarm();
        }
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        } else {
            self.cut_view.disable();
        }
    }

    pub(super) fn update_scene_materials_impl(&mut self, scene: Scene) {
        self.edit_mode.sync_to_scene(&scene);
        let stats = scene_stats(&scene);
        self.scene = Some(Arc::new(scene));
        self.scene_stats = Some(stats);
        self.needs_render = true;
        self.live_viewport_scene_dirty = self.live_viewport.is_some();
        self.offscreen_scene_dirty = true;
        self.selection_overlay_dirty = true;
        self.mesh_selection_drag = None;
        if self.can_render_cut_view() {
            self.cut_view.mark_dirty();
        } else {
            self.cut_view.disable();
        }
    }

    pub(super) fn clear_scene_impl(&mut self) {
        self.clear_unsaved_mesh_edits();
        self.hidden_layer_stack.clear();
        self.translucent_layer_restore.clear();
        self.scene = None;
        self.scene_stats = None;
        self.clear_live_viewport();
        self.prepared_scene = None;
        self.prepared_selection_overlay = None;
        self.current_paths.clear();
        self.camera = None;
        self.rendered = None;
        self.needs_render = false;
        self.live_viewport_scene_dirty = false;
        self.offscreen_scene_dirty = false;
        self.selection_overlay_dirty = false;
        self.mesh_selection_drag = None;
        self.load_queue_camera_reset = super::LoadQueueCameraReset::Idle;
        self.camera_modified_during_load = false;
        self.edit_mode.clear();
        self.bridge_split.cancel();
        self.bridge_split_disc.disarm();
        self.bridge_split_section.reset();
        self.cut_view.disable();
        self.measure.disarm();
        self.section_cache.clear();
    }

    pub(super) fn show_central_panel_impl(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, egui::Color32::from_rgb(226, 230, 234));
            self.sync_render_extent(ui.available_size(), ctx.pixels_per_point());
            let live_viewport = self.live_viewport.clone();
            let live_stats = self.scene_stats;
            if let (Some(live_viewport), Some(stats)) = (live_viewport, live_stats) {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let response = ui.allocate_rect(viewport_rect, egui::Sense::click_and_drag());
                ui.painter()
                    .add(live_viewport::paint_callback(response.rect, live_viewport));
                let mut axis_snap = None;
                paint_scale_bar(ui, response.rect, stats);
                super::paint_version_stamp(ui, response.rect);
                if let Some(camera) = self.camera.as_ref() {
                    // Lift the gizmo above the docked Section panel while cutting
                    // so it never sits under the bottom-right panel.
                    let gizmo_avoid = self.active_section_panel_rect(response.rect);
                    axis_snap = paint_axis_gizmo(ui, response.rect, camera, &response, gizmo_avoid);
                }
                self.show_layers_overlay(ui, response.rect, ctx);
                self.show_mesh_editor_overlay(response.rect, ctx);
                self.paint_mesh_selection_drag_overlay_impl(ui);
                self.paint_sculpt_cursor_impl(ui, response.rect);
                self.show_status_overlay(ui, response.rect);
                let bridge_ui_consumed = self.show_bridge_split_overlay(ui, &response, ctx);
                let cut_ui_consumed = self.show_cut_tool_overlay(ui, response.rect, ctx);
                // A click the axis gizmo snapped on never doubles as a measure
                // anchor.
                let measure_ui_consumed =
                    self.show_measure_tool_overlay(ui, &response, axis_snap.is_some(), ctx);
                if let Some(axis) = axis_snap {
                    if let Some(camera) = self.camera.as_mut() {
                        camera.snap_to_axis(axis);
                        self.needs_render = true;
                        ctx.request_repaint();
                    }
                }
                if !bridge_ui_consumed && !cut_ui_consumed && !measure_ui_consumed {
                    self.handle_viewport_input(ctx, &response, response.rect);
                }
            } else if let Some((texture, stats)) = self
                .rendered
                .as_ref()
                .map(|rendered| (rendered.texture.clone(), rendered.stats))
            {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let response = ui.put(
                    viewport_rect,
                    egui::Image::new((texture.id(), available))
                        .sense(egui::Sense::click_and_drag()),
                );
                let mut axis_snap = None;
                paint_scale_bar(ui, response.rect, stats);
                super::paint_version_stamp(ui, response.rect);
                if let Some(camera) = self.camera.as_ref() {
                    // Lift the gizmo above the docked Section panel while cutting
                    // so it never sits under the bottom-right panel.
                    let gizmo_avoid = self.active_section_panel_rect(response.rect);
                    axis_snap = paint_axis_gizmo(ui, response.rect, camera, &response, gizmo_avoid);
                }
                self.show_layers_overlay(ui, response.rect, ctx);
                self.show_mesh_editor_overlay(response.rect, ctx);
                self.paint_mesh_selection_drag_overlay_impl(ui);
                self.paint_sculpt_cursor_impl(ui, response.rect);
                self.show_status_overlay(ui, response.rect);
                let bridge_ui_consumed = self.show_bridge_split_overlay(ui, &response, ctx);
                let cut_ui_consumed = self.show_cut_tool_overlay(ui, response.rect, ctx);
                // A click the axis gizmo snapped on never doubles as a measure
                // anchor.
                let measure_ui_consumed =
                    self.show_measure_tool_overlay(ui, &response, axis_snap.is_some(), ctx);
                if let Some(axis) = axis_snap {
                    if let Some(camera) = self.camera.as_mut() {
                        camera.snap_to_axis(axis);
                        self.needs_render = true;
                        ctx.request_repaint();
                    }
                }
                if !bridge_ui_consumed && !cut_ui_consumed && !measure_ui_consumed {
                    self.handle_viewport_input(ctx, &response, response.rect);
                }
            } else if self.scene.is_none() {
                let available = ui.available_size();
                let viewport_rect = egui::Rect::from_min_size(ui.cursor().min, available);
                let _response = ui.allocate_rect(viewport_rect, egui::Sense::hover());
                self.show_status_overlay(ui, viewport_rect);
                super::paint_version_stamp(ui, viewport_rect);
            } else {
                ui.spinner();
            }
        });
    }

    pub(super) fn render_pending_frame_impl(&mut self, ctx: &egui::Context) {
        if self.needs_render {
            if self.live_viewport.is_some() {
                self.sync_live_viewport();
            } else {
                self.render_now(ctx);
            }
            ctx.request_repaint();
        }
    }
}

pub(super) fn scene_mesh_uniform(entry: &SceneMesh) -> GpuMeshUniform {
    GpuMeshUniform {
        model: Mat4::from(entry.transform).to_cols_array(),
        tint: entry.tint,
        opacity: entry.opacity,
        has_texture: u32::from(entry.mesh.texture().is_some()),
        show_orientation: u32::from(entry.show_orientation),
        show_vertex_colors: u32::from(entry.show_vertex_colors),
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

pub(super) fn scene_stats(scene: &Scene) -> SceneStats {
    let bbox = scene.bbox();
    let [w, h, d] = bbox.dimensions_mm();
    SceneStats {
        bbox_mm: [w.as_mm(), h.as_mm(), d.as_mm()],
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

    /// Source contract for the destroyed-texture submit crash. The per-frame
    /// render paths must update ONE persistent egui texture id in place
    /// (`TextureHandle::set` / `CutTool::store_slice`), never allocate a fresh id
    /// per render — a dropped predecessor emits a texture-`free` that egui-wgpu
    /// 0.29 turns into `wgpu::Texture::destroy` *before* `queue.submit`, killing a
    /// texture the same frame painted. The viewport render's `load_texture` is
    /// the first-time-only fallback in the `None` arm.
    #[test]
    fn per_frame_render_paths_reuse_persistent_texture_ids() {
        let source = include_str!("app_render.rs").replace("\r\n", "\n");
        assert!(
            source.contains("frame.texture.set(color_image, egui::TextureOptions::LINEAR)"),
            "render_now must update the viewport texture in place, not reallocate it"
        );
        assert!(
            source.contains("self.cut_view.store_slice(ctx, color_image, slice_cam)"),
            "render_cut_now must route the slice through CutTool::store_slice"
        );
        assert!(
            !source.contains("load_texture(\"occluview-cut\""),
            "the cut slice must not allocate a fresh egui texture id per render"
        );
    }
}

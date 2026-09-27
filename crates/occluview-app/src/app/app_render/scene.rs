use super::super::{
    egui, live_viewport, paint_axis_gizmo, paint_scale_bar, Arc, AxisGizmoInput, GpuMeshUniform,
    Instant, Mat4, OccluViewApp, Scene, SceneMesh,
};
use occluview_core::Aabb;

impl OccluViewApp {
    pub(in crate::app) fn set_scene(&mut self, mut scene: Scene, reset_camera: bool) {
        self.document.content_revision = self.document.content_revision.wrapping_add(1);
        // The drag is ended here, and which form is decided by what happens to
        // the layer, not by where the code sits. `set_scene` is reached by two
        // different transitions:
        //
        // - Replace (and the scene-destroying paths): the layer and the pose
        //   both go. Recording would push a history step describing the
        //   outgoing scene, and the guard would not refuse it (it matches layer
        //   ids), so the first Ctrl+Z would show the edit undone and the second
        //   would put it back and rewind the pose. `forget_replaced_scene_state`
        //   already drops the gesture for this path before the scene is
        //   installed.
        // - Append: the layer survives into the combined scene, so the pose
        //   sitting on it is just as real there. Dropping it would leave a scan
        //   in a pose that no history step describes and no save prompt names.
        //   It must be committed.
        //
        // `abandon_align_drag` is the commit form and is the right default here:
        // on the Replace path the drag is already `None` (dropped by
        // `forget_replaced_scene_state`), so it is a no-op, and on the Append
        // path it records the move. Calling `discard` here instead would let an
        // append carry a moved pose forward with nothing recording it.
        self.abandon_align_drag();
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.document.edit_mode.sync_to_scene(&scene);
        // A structural scene swap (load, delete, another mesh edit, undo/redo)
        // reverts the geometry the persistent sculpt session was prepared over,
        // without necessarily changing topology_id (a sculpt commit preserves
        // it), so drop the session here and re-prepare on the next stroke.
        self.tools.sculpt.invalidate_session();
        self.document.unsaved_sculpt_stroke = false;
        // A measured map describes one pose. Scene installation clears the
        // alignment result, so remove its display state from restored history.
        for entry in scene.meshes_mut() {
            if entry.overlay_kind() == Some(occluview_core::OverlayKind::Measured) {
                entry.clear_overlay();
            }
        }
        self.document.scene = Some(Arc::new(scene));
        self.enrol_align_arrivals();
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
        // got replaced. Every structural path (undo, redo, repair, close holes,
        // crop, cut, separate, a bridge split commit, a cancelled mesh-edit
        // session) passes through here. Left up, the map would keep describing
        // the former surface, take the scan's tint, and the panel would report
        // a percentage for a surface that no longer exists.
        self.forget_align_fit(
            &self
                .ui
                .locale
                .tr(crate::i18n::message_id!("align-status-scan-changed")),
        );
        // Structural scene change: world anchors may now dangle over deleted or
        // replaced geometry, so measurements are cleared (the tool stays armed
        // while something remains to measure). Material-only updates keep them
        // (world space is unchanged).
        self.tools.measure.clear_measurements();
        if !self.has_measurable_layer() {
            self.tools.measure.disarm();
        }
        if self.can_render_cut_view() {
            // A planted disc holds a world-space plane. Scanner vendors place
            // models at wildly different origins, so a plane kept across a
            // scene replace usually leaves the new case entirely on the
            // clipped-away side, drawing as a faint ghost — which reads as "the
            // file loaded wrong". Re-arm instead: the tool stays on, the stale
            // placement does not. Bridge split does the same just above.
            if self.tools.cut_view.is_active() {
                self.tools.cut_view.enable();
            }
            self.tools.cut_view.mark_dirty();
        } else {
            self.tools.cut_view.disable();
        }
    }

    pub(in crate::app) fn update_scene_materials(&mut self, scene: Scene) {
        self.document.scene = Some(Arc::new(scene));
        self.mark_scene_materials_changed();
    }

    /// The bookkeeping a material change needs, for a caller that already owns
    /// the live scene and mutated it in place.
    pub(in crate::app) fn mark_scene_materials_changed(&mut self) {
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

    pub(in crate::app) fn clear_scene(&mut self) {
        self.document.content_revision = self.document.content_revision.wrapping_add(1);
        // Overlay cleanup may edit the scene; detach this handle first.
        let scene = self.document.scene.take();
        // The last layer can disappear while Align Meshes is armed. Revoke its
        // pose, overlay, mask, and worker generation before a new scene may
        // reuse one of the old layer ids.
        self.reset_align_state_for_scene_clear();
        drop(scene);
        // A clear has no replacement scene to validate against. Revoke the
        // persistent Sculpt worker before dropping the scene so a background
        // completion cannot outlive this generation and be mistaken for the
        // next file's layer.
        self.tools.sculpt.invalidate_session();
        self.document.unsaved_sculpt_stroke = false;
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
        self.document.load_queue_camera_reset = super::super::LoadQueueCameraReset::Idle;
        self.document.camera_modified_during_load = false;
        self.document.edit_mode.clear();
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.tools.cut_view.disable();
        self.tools.measure.disarm();
        self.render.section_cache.clear();
    }

    pub(in crate::app) fn show_central_panel(&mut self, root_ui: &mut egui::Ui) {
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
                // The callback paints into egui's render pass at this rect, so
                // it is the real viewport; `render_extent_px` is clamped for the
                // offscreen target and the invalidation threshold. The splat
                // radius is measured in pixels of the former.
                let ppp = ctx.pixels_per_point();
                let live_px = response.rect.size() * ppp;
                self.render.live_viewport_px = Some([
                    // Not clamped to the render-extent bounds: this is the
                    // viewport the callback actually paints, and the splat
                    // radius is measured against it. A non-finite or negative
                    // size cannot reach here (egui rects are finite and
                    // non-negative), so the cast is a plain round with a floor
                    // of one pixel.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        live_px.x.round().max(1.0) as u32
                    },
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        live_px.y.round().max(1.0) as u32
                    },
                ]);
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
    /// One body, called by both branches of `show_central_panel_impl`, so a new
    /// tool reaches the offscreen branch too. That branch runs only for
    /// operators whose driver could not give the app a live viewport, where a
    /// tool missing from it would be hardest to diagnose. The branches differ
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
        // The contact reading runs whether or not the Align tool is armed, and
        // its readout is painted after the panels so the chip sits above them.
        self.drain_contacts_worker(ctx);
        self.sync_contacts_with_scene(ctx);
        self.handle_contact_escape(ctx);
        let contact_ui_consumed = self.show_contact_bar(ui, response.rect, ctx);
        self.show_contact_hover(ui, response, ctx);
        let contact_ui_consumed = contact_ui_consumed && !align_ui_consumed;
        let measure_ui_consumed =
            self.show_measure_tool_overlay(ui, response, axis_snap.is_some(), ctx);
        if let Some(axis) = axis_snap {
            if let Some(camera) = self.render.camera.as_mut() {
                camera.snap_to_axis(axis);
                self.render.invalidation.request_redraw();
                ctx.request_repaint();
            }
        }
        if !bridge_ui_consumed
            && !cut_ui_consumed
            && !measure_ui_consumed
            && !align_ui_consumed
            && !contact_ui_consumed
        {
            self.handle_viewport_input(ctx, response, response.rect, axis_snap.is_some());
        }
        // Input resolves and caches the authoritative sculpt hit first. The
        // visual cursor then reuses it for held drags and publishes its GPU
        // uniforms before the callback's render pass executes.
        self.paint_sculpt_cursor_impl(ui, response);
    }

    pub(in crate::app) fn render_pending_frame(&mut self, ctx: &egui::Context) {
        if self.render.invalidation.redraw_pending() {
            if self.render.live_viewport.is_some() {
                self.sync_live_viewport();
            } else if !self.offscreen_available() {
                self.render.invalidation.consume_redraw();
                // Wake up when the wait is over. Without this the retry waits
                // for the operator's next input, and on a machine with no live
                // viewport, which is the machine this path serves, a still
                // window would never try again.
                if let Some(deadline) = self.render.offscreen_retry_after {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    ctx.request_repaint_after(remaining);
                }
            } else {
                self.render_now(ctx);
            }
            if self.render.live_viewport.is_some() || self.offscreen_available() {
                ctx.request_repaint();
            }
        }
    }
}

/// Transform a local AABB conservatively for camera and section framing.
pub(in crate::app) fn transformed_bbox(local: Aabb, transform: glam::Affine3A) -> Aabb {
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

pub(in crate::app) fn scene_mesh_uniform(entry: &SceneMesh) -> GpuMeshUniform {
    // Derived from the overlay rather than stored beside it, so the two can
    // never disagree about which kind is up. A measured map replaces the
    // scan's colours and ignores its tint — the ramp is the reading. Paint
    // does neither: it is mixed over the surface's own material, so the scan
    // keeps its tint, its texture and its normal lighting and only the marked
    // region turns blue.
    let measured = entry.overlay_kind() == Some(occluview_core::OverlayKind::Measured);
    let paint = entry.overlay_kind() == Some(occluview_core::OverlayKind::Paint);
    let overlay = measured || paint;
    GpuMeshUniform {
        model: Mat4::from(entry.transform).to_cols_array(),
        tint: entry.tint,
        opacity: entry.opacity,
        has_texture: u32::from(entry.mesh.texture().is_some()),
        show_orientation: u32::from(entry.show_orientation),
        show_vertex_colors: u32::from(entry.show_vertex_colors || overlay),
        // A measured map is drawn instead of the texture; paint is drawn over
        // it, so the texture (the scan's real colour) stays.
        show_texture: u32::from(entry.show_texture && !measured),
        measured_map: u32::from(measured),
        overlay_paint: u32::from(paint),
        ..GpuMeshUniform::identity()
    }
}

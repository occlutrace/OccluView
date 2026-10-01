use super::super::{
    egui, live_viewport, paint_axis_gizmo, paint_scale_bar, Arc, AxisGizmoInput, GpuMeshUniform,
    Instant, Mat4, Scene, SceneContext, SceneMesh,
};
use occluview_core::Aabb;

impl SceneContext<'_> {
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
        self.document.current_paths.clear();
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

    /// Draw one scene into its assigned canvas. The workspace root owns the
    /// shared `CentralPanel`, so scene rendering never changes global layout.
    pub(in crate::app) fn show_pane(
        &mut self,
        root_ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        workspace_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        self.workspace_rect = Some(workspace_rect);
        self.sync_render_extent(viewport_rect.size(), ctx.pixels_per_point());
        let input_allowed = self.is_active && self.input_allowed;
        if input_allowed {
            self.handle_edit_shortcuts(ctx);
        }
        let sense = if input_allowed {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        };
        let live_viewport = self.render.live_viewport.clone();
        if let Some(live_viewport) = live_viewport {
            let response = root_ui.allocate_rect(viewport_rect, sense);
            // The callback paints into the egui render pass at this rect. Its
            // camera and GPU peer belong only to this SceneContext.
            let live_px = response.rect.size() * ctx.pixels_per_point();
            self.render.live_viewport_px = Some([
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    live_px.x.round().max(1.0) as u32
                },
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    live_px.y.round().max(1.0) as u32
                },
            ]);
            root_ui
                .painter()
                .add(live_viewport::paint_callback(response.rect, live_viewport));
            self.show_viewport_overlays(root_ui, &response, ctx, input_allowed);
        } else if let Some(texture) = self
            .render
            .rendered
            .as_ref()
            .map(|rendered| rendered.texture.clone())
        {
            let available = viewport_rect.size();
            let response = root_ui.put(
                viewport_rect,
                egui::Image::new((texture.id(), available)).sense(sense),
            );
            self.show_viewport_overlays(root_ui, &response, ctx, input_allowed);
        } else if self
            .document
            .scene
            .as_ref()
            .is_none_or(|scene| scene.meshes().is_empty())
        {
            let response = root_ui.allocate_rect(viewport_rect, sense);
            Self::set_drop_hover_cursor_if_hovering(ctx);
            self.show_empty_state(root_ui, &response, ctx);
            if self.is_active {
                self.show_status_overlay(root_ui, viewport_rect);
            }
        } else {
            root_ui.scope_builder(egui::UiBuilder::new().max_rect(viewport_rect), |ui| {
                ui.centered_and_justified(egui::Ui::spinner);
            });
        }
    }

    /// Every overlay the viewport draws, and the input arbitration that
    /// follows them.
    ///
    /// Viewport-specific overlays for one pane. The common Layers panel is
    /// drawn once by the workspace root after all scene surfaces.
    fn show_viewport_overlays(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
        input_allowed: bool,
    ) {
        // While files hover anywhere over the window the viewport advertises
        // itself as the drop target, without painting a border over the model.
        Self::set_drop_hover_cursor_if_hovering(ctx);
        if self
            .document
            .scene
            .as_ref()
            .is_none_or(|scene| scene.meshes().is_empty())
        {
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
        if self.is_active {
            self.show_status_overlay(ui, response.rect);
        }
        let tool_ui_allowed =
            self.is_active && ctx.input(|input| input.focused) && !self.ui.modal_dialog_open();
        if !tool_ui_allowed {
            self.publish_sculpt_cursor(None);
            self.paint_passive_tool_overlays(ui, response.rect, ctx);
            return;
        }
        self.show_mesh_editor_overlay(response.rect, ctx);
        self.paint_mesh_selection_drag_overlay_impl(ui);
        // Resolve the contact strip and its dynamically sized details panel
        // before scene tools inspect raw press positions on this same layer.
        self.drain_contacts_worker(ctx);
        self.sync_contacts_with_scene(ctx);
        let contact_ui_consumed = self.show_contact_bar(ui, response.rect, ctx);
        self.show_contact_hover(ui, response, ctx);
        let bridge_ui_consumed = self.show_bridge_split_overlay(ui, response, ctx);
        let cut_ui_consumed = self.show_cut_tool_overlay(ui, response.rect, ctx);
        // A click the axis gizmo snapped on never doubles as a measure anchor.
        let align_ui_consumed =
            self.show_align_tool_overlay(ui, response, axis_snap.is_some(), ctx);
        let contact_ui_consumed = contact_ui_consumed && !align_ui_consumed;
        let measure_ui_consumed =
            self.show_measure_tool_overlay(ui, response, axis_snap.is_some(), ctx);
        self.handle_contact_escape(ctx);
        if let Some(axis) = axis_snap {
            if let Some(camera) = self.render.camera.as_mut().filter(|_| input_allowed) {
                camera.snap_to_axis(axis);
                self.request_camera_repaint(ctx);
            }
        }
        if input_allowed
            && !bridge_ui_consumed
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

    /// Keep scene annotations visible while controls belong to another pane.
    /// Painting the saved result must never advance the inactive tool's input.
    fn paint_passive_tool_overlays(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        ctx: &egui::Context,
    ) {
        self.paint_mesh_selection_drag_overlay_impl(ui);
        if let Some(camera) = self.render.camera {
            crate::measure_overlay::paint_measurements(
                ui.painter(),
                &camera,
                viewport,
                &self.tools.measure,
                self.persistence.settings.unit_display,
                None,
                self.persistence.settings.ruler_line_angle,
            );
            if self.tools.align.tool.is_armed() {
                if let Some(scene) = self.document.scene.as_deref() {
                    crate::align_overlay::paint_pairs(
                        ui.painter(),
                        &crate::align_overlay::PairPaint {
                            camera: &camera,
                            viewport_rect: viewport,
                            scene,
                            tool: &self.tools.align.tool,
                            rejected: &self.tools.align.rejected,
                            hover: None,
                        },
                    );
                }
            }
        }
        ui.scope(|ui| {
            ui.disable();
            self.show_cut_tool_overlay(ui, viewport, ctx);
            self.show_contact_bar(ui, viewport, ctx);
        });
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

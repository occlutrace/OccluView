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
        // Preserving a view requires an existing camera. The first mesh in a
        // new pane must be framed even when automatic framing is disabled.
        if reset_camera || self.render.camera.is_none() {
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
        self.document.departed_layer_paths.clear();
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
            let live_viewport_px = [
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    live_px.x.round().max(1.0) as u32
                },
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    live_px.y.round().max(1.0) as u32
                },
            ];
            if self.render.live_viewport_px != Some(live_viewport_px) {
                self.render.live_viewport_px = Some(live_viewport_px);
                self.render.invalidation.request_redraw();
            }
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
            self.show_viewport_overlays(root_ui, &response, ctx, input_allowed);
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
                &self.ui.locale,
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
        self.log_tool_gate_for_viewport(ctx, response.rect);
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

    fn log_tool_gate_for_viewport(&mut self, ctx: &egui::Context, viewport: egui::Rect) {
        let pressed = ctx.input(|input| {
            input.raw.events.iter().any(|event| {
                matches!(event, egui::Event::PointerButton { pos, pressed: true, .. }
                if viewport.contains(*pos))
            })
        });
        self.scene_ui.log_skipped_tool_input(
            super::super::state_ui::ToolGateState {
                scene_key: self.scene_key,
                active_scene_key: self.active_scene_key,
                is_active: self.is_active,
                window_focused: ctx.input(|input| input.focused),
                modal_dialog_open: self.ui.modal_dialog_open(),
                input_route: self.input_route,
                capture_owner: self.input_capture,
            },
            pressed,
        );
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
            crate::measure::measure_overlay::paint_measurements(
                ui.painter(),
                &camera,
                viewport,
                &self.tools.measure,
                self.persistence.settings.unit_display,
                None,
                self.persistence.settings.ruler_line_angle,
                &self.ui.locale,
            );
            if self.tools.align.tool.is_armed() {
                if let Some(scene) = self.document.scene.as_deref() {
                    crate::align::align_overlay::paint_pairs(
                        ui.painter(),
                        &crate::align::align_overlay::PairPaint {
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

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::app::OccluViewApp;
    use eframe::App;

    fn input_app(ctx: &egui::Context) -> OccluViewApp {
        let mut app = crate::app::app_test_support::test_app("scene-input");
        app.ui.repaint_ctx = ctx.clone();
        app
    }

    fn frame(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        focused_frame(app, ctx, events, true)
    }

    fn focused_frame(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        focused: bool,
    ) -> egui::FullOutput {
        sized_frame(app, ctx, events, focused, egui::vec2(1000.0, 800.0))
    }

    fn sized_frame(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        focused: bool,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            focused,
            ..Default::default()
        };
        let mut native_frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_logic(&input, |ctx| app.logic(ctx, &mut native_frame));
        // Input tests supply a cached image instead of creating a GPU device.
        for scene in &mut app.workspace.scenes {
            if scene.document.scene.is_some() {
                scene.render.offscreen_retry_after =
                    Some(Instant::now() + std::time::Duration::from_secs(600));
                if scene.render.rendered.is_none() {
                    scene.render.rendered = Some(crate::app::RenderedFrame {
                        texture: ctx.load_texture(
                            "input-test-viewport",
                            egui::ColorImage::filled([1, 1], egui::Color32::GRAY),
                            egui::TextureOptions::LINEAR,
                        ),
                        pixels: vec![128, 128, 128, 255],
                        size_px: [1, 1],
                    });
                }
            }
        }
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut native_frame));
        output.textures_delta.clear();
        output
    }

    fn secondary_button(point: egui::Pos2, pressed: bool) -> egui::Event {
        pointer_button(point, egui::PointerButton::Secondary, pressed)
    }

    fn secondary_menu(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        point: egui::Pos2,
    ) -> egui::FullOutput {
        frame(app, ctx, vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            frame(app, ctx, vec![secondary_button(point, pressed)]);
        }
        assert!(
            egui::Popup::is_any_open(ctx),
            "secondary click opens its menu"
        );
        frame(app, ctx, vec![])
    }

    fn pointer_button(
        point: egui::Pos2,
        button: egui::PointerButton,
        pressed: bool,
    ) -> egui::Event {
        egui::Event::PointerButton {
            pos: point,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn has_control(output: &egui::FullOutput, label: &str) -> bool {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .is_some_and(|update| {
                update.nodes.iter().any(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
                })
            })
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "AccessKit exposes f64 bounds for the bounded f32 test viewport."
    )]
    fn control_point(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        label: &str,
    ) -> anyhow::Result<egui::Pos2> {
        let output = frame(app, ctx, vec![]);
        let bounds = output
            .platform_output
            .accesskit_update
            .and_then(|update| {
                update.nodes.into_iter().find_map(|(_, node)| {
                    (node.role() == egui::accesskit::Role::Button && node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
            })
            .ok_or_else(|| anyhow::anyhow!("missing control {label}"))?;
        Ok(egui::pos2(
            bounds.x0.midpoint(bounds.x1) as f32,
            bounds.y0.midpoint(bounds.y1) as f32,
        ))
    }

    fn click_control(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        label: &str,
    ) -> anyhow::Result<()> {
        let point = control_point(app, ctx, label)?;
        frame(app, ctx, vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![pointer_button(point, egui::PointerButton::Primary, pressed)],
            );
        }
        Ok(())
    }

    /// Pump frames until the mesh editor's `label` control can take a click.
    ///
    /// Entering from a layer row also prepares Sculpt in the background. The
    /// editor's controls wait for that, and its window moves when the pending
    /// notice goes, so a click is safe only once both are at rest.
    fn rest_editor_control(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        label: &str,
    ) -> anyhow::Result<()> {
        let waiting = Instant::now();
        let mut at = control_point(app, ctx, label)?;
        loop {
            let pending = app.workspace.scenes[0].tools.sculpt.is_busy();
            let next = control_point(app, ctx, label)?;
            if !pending && next == at {
                return Ok(());
            }
            at = next;
            anyhow::ensure!(
                waiting.elapsed() < std::time::Duration::from_secs(10),
                "the editor never came to rest for {label}"
            );
            if pending {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn empty_pane_right_click_opens_scene_menu_before_any_render() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = input_app(&ctx);
        let point = egui::pos2(650.0, 350.0);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            frame(&mut app, &ctx, vec![secondary_button(point, pressed)]);
        }
        assert!(
            egui::Popup::is_any_open(&ctx),
            "an empty pane must route secondary clicks before it has a rendered texture"
        );
        let output = frame(&mut app, &ctx, vec![]);
        assert!(output
            .platform_output
            .accesskit_update
            .is_some_and(|update| {
                update.nodes.iter().any(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button
                        && node.label().is_some_and(|label| label.contains("Save"))
                })
            }));
        assert!(!app.workspace.scenes[0].presentation.open_dialog_requested);
        assert!(app.workspace.input.capture().is_none());
    }

    #[test]
    fn decoded_open_routes_restore_active_input_and_panel_buttons() -> anyhow::Result<()> {
        use crate::scene_loading::{PendingSceneLoad, SceneLoadMode};

        for (source, mode) in [
            ("startup", SceneLoadMode::Replace),
            ("open", SceneLoadMode::Replace),
            ("recent", SceneLoadMode::Replace),
            ("single-instance", SceneLoadMode::Replace),
            ("add", SceneLoadMode::Append),
            ("drop", SceneLoadMode::Append),
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = input_app(&ctx);
            let original_key = app.workspace.scenes[0].key;
            let (sender, receiver) = std::sync::mpsc::channel();
            sender.send(Ok(crate::app::app_test_support::named_scene(
                "surface", 0.0,
            )))?;
            app.loader.install_active(PendingSceneLoad {
                scene_key: original_key,
                paths: vec![std::path::PathBuf::from("input-test.stl")],
                source,
                mode,
                started_at: Instant::now(),
                receiver,
                superseded: false,
                content_revision_at_request: 0,
                dirty_at_request: false,
                requested_at: Instant::now(),
            });
            frame(&mut app, &ctx, vec![]);
            let target = app.workspace.scenes[0].target();
            assert_eq!(app.workspace.input.active(), target, "{source}: live epoch");
            assert!(app
                .active_context()
                .is_some_and(|scene| scene.is_active && scene.input_allowed));
            assert!(!app.ui.modal_dialog_open(), "{source}: no stale modal");
            frame(&mut app, &ctx, vec![key(egui::Key::E)]);
            assert!(app.workspace.scenes[0]
                .document
                .edit_mode
                .has_active_session());
            frame(&mut app, &ctx, vec![]);
            click_control(&mut app, &ctx, "All")?;
            assert!(
                app.active_context().is_some_and(|scene| {
                    scene.document.scene.as_deref().is_some_and(|model| {
                        scene.document.edit_mode.visible_selected_face_count(model) == 1
                    })
                }),
                "{source}: All changes selection through the actual panel"
            );
            click_control(&mut app, &ctx, "None")?;
            assert!(app.active_context().is_some_and(|scene| {
                scene.document.scene.as_deref().is_some_and(|model| {
                    scene.document.edit_mode.visible_selected_face_count(model) == 0
                })
            }));
            click_control(&mut app, &ctx, "Invert")?;
            assert!(app.active_context().is_some_and(|scene| {
                scene.document.scene.as_deref().is_some_and(|model| {
                    scene.document.edit_mode.visible_selected_face_count(model) == 1
                })
            }));
            let output = secondary_menu(&mut app, &ctx, egui::pos2(900.0, 700.0));
            assert!(
                has_control(&output, "Save scene as…"),
                "{source}: empty-space menu"
            );
            let scene = &app.workspace.scenes[0];
            let viewport = scene
                .presentation
                .viewport_context_menu
                .as_ref()
                .map(|(response, _)| response.rect)
                .ok_or_else(|| anyhow::anyhow!("{source}: viewport menu response"))?;
            let camera = scene
                .render
                .camera
                .ok_or_else(|| anyhow::anyhow!("{source}: loaded camera"))?;
            let (on_model, _) = crate::viewer::project_world_to_viewport(
                &camera,
                viewport,
                glam::vec3(0.25, 0.25, 0.0),
            )
            .ok_or_else(|| anyhow::anyhow!("{source}: projected mesh point"))?;
            frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
            frame(&mut app, &ctx, vec![]);
            let output = secondary_menu(&mut app, &ctx, on_model);
            assert!(
                has_control(&output, "Wireframe overlay"),
                "{source}: picked-layer menu"
            );
            frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        }
        Ok(())
    }

    #[test]
    fn window_focus_and_information_dialog_release_the_edit_panel() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = input_app(&ctx);
        let mut scene = app
            .active_context()
            .ok_or_else(|| anyhow::anyhow!("initial scene"))?;
        scene.set_scene(
            crate::app::app_test_support::named_scene("surface", 0.0),
            true,
        );
        frame(&mut app, &ctx, vec![key(egui::Key::E)]);
        frame(&mut app, &ctx, vec![]);
        assert!(has_control(&frame(&mut app, &ctx, vec![]), "All"));
        let unfocused = focused_frame(
            &mut app,
            &ctx,
            vec![egui::Event::WindowFocused(false)],
            false,
        );
        assert!(!has_control(&unfocused, "All"));
        let focused = focused_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true)], true);
        assert!(has_control(&focused, "All"));
        app.ui.information_dialog =
            crate::app::information_dialog::InformationDialog::KeyboardMouse;
        assert!(!has_control(&frame(&mut app, &ctx, vec![]), "All"));
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
        assert!(!app.ui.information_dialog.is_open());
        assert!(has_control(&frame(&mut app, &ctx, vec![]), "All"));
        click_control(&mut app, &ctx, "All")?;
        Ok(())
    }

    #[test]
    fn scene_creation_switching_and_close_keep_the_active_input_target() -> anyhow::Result<()> {
        use crate::app::workspace::commands::{SplitSide, WorkspaceCommand};

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = input_app(&ctx);
        let first_key = app.workspace.scenes[0].key;
        let first_name = app.workspace.scenes[0].name.clone();
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("initial scene"))?
            .set_scene(
                crate::app::app_test_support::named_scene("first", 0.0),
                true,
            );
        frame(&mut app, &ctx, vec![key(egui::Key::E)]);
        click_control(&mut app, &ctx, "All")?;
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("first active scene"))?
            .queue_new_scene(SplitSide::Right);
        frame(&mut app, &ctx, vec![]);
        let second_key = app.workspace.input.active().scene;
        assert_ne!(second_key, first_key);
        assert_eq!(app.workspace.scenes.len(), 2);
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("new active scene"))?
            .set_scene(
                crate::app::app_test_support::named_scene("second", 0.0),
                true,
            );
        frame(&mut app, &ctx, vec![key(egui::Key::E)]);
        click_control(&mut app, &ctx, "All")?;
        frame(&mut app, &ctx, vec![key(egui::Key::F6)]);
        assert_eq!(app.workspace.input.active().scene, first_key);
        assert!(app
            .active_context()
            .is_some_and(|scene| scene.is_active && scene.input_allowed));
        click_control(&mut app, &ctx, "None")?;
        frame(&mut app, &ctx, vec![key(egui::Key::F6)]);
        assert_eq!(app.workspace.input.active().scene, second_key);
        click_control(&mut app, &ctx, &first_name)?;
        assert_eq!(app.workspace.input.active().scene, first_key);
        app.workspace
            .commands
            .push_back(WorkspaceCommand::CloseScene { scene: second_key });
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.workspace.scenes.len(), 1);
        assert_eq!(app.workspace.input.active().scene, first_key);
        click_control(&mut app, &ctx, "All")?;
        app.workspace
            .commands
            .push_back(WorkspaceCommand::CloseScene { scene: first_key });
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.workspace.scenes.len(), 1);
        assert_ne!(app.workspace.input.active().scene, first_key);
        assert!(app
            .active_context()
            .is_some_and(|scene| scene.is_active && scene.input_allowed));
        let output = secondary_menu(&mut app, &ctx, egui::pos2(850.0, 500.0));
        assert!(has_control(&output, "Save scene as…"));
        Ok(())
    }

    #[test]
    fn loading_without_auto_framing_initializes_only_a_missing_camera() {
        use crate::scene_loading::SceneLoadMode;

        for (mode, existing_camera) in [
            (SceneLoadMode::Append, false),
            (SceneLoadMode::Replace, false),
            (SceneLoadMode::Append, true),
            (SceneLoadMode::Replace, true),
        ] {
            let ctx = egui::Context::default();
            let mut app = input_app(&ctx);
            app.persistence.settings.frame_scene_on_open = false;
            if existing_camera {
                app.workspace.scenes[0].render.camera = Some(crate::app::Camera {
                    target: glam::vec3(8.0, 9.0, 10.0),
                    orthographic_height: 77.0,
                    ..Default::default()
                });
            }
            let previous = app.workspace.scenes[0].render.camera;
            let model = crate::app::app_test_support::named_scene("surface", 0.0);
            let pending =
                crate::app::app_test_support::delivered_load(&app, model, mode, "input-test.stl");
            app.loader.install_active(pending);
            frame(&mut app, &ctx, vec![]);
            let camera = app.workspace.scenes[0].render.camera;
            assert!(
                camera.is_some(),
                "{mode:?}: a first load always needs a camera"
            );
            if let Some(previous) = previous {
                assert_eq!(camera.map(|camera| camera.target), Some(previous.target));
                assert!(
                    camera.is_some_and(|camera| {
                        (camera.orthographic_height - previous.orthographic_height).abs()
                            < f32::EPSILON
                    }),
                    "{mode:?}: preserve the existing view when auto framing is off"
                );
            }
        }
    }

    #[test]
    fn edit_entry_releases_competing_tools_and_accepts_a_face_click() -> anyhow::Result<()> {
        for route in ["toolbar", "key", "layer"] {
            for previous in ["align", "cut", "measure", "bridge"] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let mut app = input_app(&ctx);
                let model = crate::app::app_test_support::named_scene("surface", 0.0);
                app.active_context()
                    .ok_or_else(|| anyhow::anyhow!("active scene"))?
                    .set_scene(model.clone(), true);
                secondary_menu(&mut app, &ctx, egui::pos2(900.0, 700.0));
                let viewport = app.workspace.scenes[0]
                    .presentation
                    .viewport_context_menu
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("viewport response"))?
                    .0
                    .rect;
                frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
                frame(&mut app, &ctx, vec![]);
                arm_previous_tool(&mut app, &ctx, previous, &model)?;
                // A retained Sculpt tab must not determine the new session's owner.
                app.workspace.scenes[0].tools.editor_tab =
                    crate::mesh_editor::mesh_editor_overlay::EditorTab::Sculpt;
                match route {
                    "toolbar" => click_control(&mut app, &ctx, "Edit")?,
                    "key" => {
                        frame(&mut app, &ctx, vec![key(egui::Key::E)]);
                    }
                    _ => {
                        let row = control_point(&mut app, &ctx, "surface")?;
                        secondary_menu(&mut app, &ctx, row);
                        click_control(&mut app, &ctx, "Mesh Editing")?;
                    }
                }
                let scene = &app.workspace.scenes[0];
                assert!(
                    scene.document.edit_mode.has_active_session(),
                    "{previous} -> {route}"
                );
                assert!(
                    !scene.tools.align.tool.is_armed(),
                    "{previous} -> {route}: Align owns clicks"
                );
                assert!(
                    !scene.tools.cut_view.is_active(),
                    "{previous} -> {route}: Cut owns clicks"
                );
                assert!(
                    !scene.tools.measure.is_active(),
                    "{previous} -> {route}: Measure owns clicks"
                );
                assert_eq!(
                    scene.tools.bridge_split.session().mode(),
                    crate::bridge_split::BridgeSplitMode::Off
                );
                assert_eq!(
                    scene.tools.editor_tab,
                    crate::mesh_editor::mesh_editor_overlay::EditorTab::EditMesh
                );
                rest_editor_control(&mut app, &ctx, "Lasso")?;
                click_control(&mut app, &ctx, "Lasso")?;
                let camera = app.workspace.scenes[0]
                    .render
                    .camera
                    .ok_or_else(|| anyhow::anyhow!("camera"))?;
                let (point, _) = crate::viewer::project_world_to_viewport(
                    &camera,
                    viewport,
                    glam::vec3(0.25, 0.25, 0.0),
                )
                .ok_or_else(|| anyhow::anyhow!("mesh projection"))?;
                frame(&mut app, &ctx, vec![egui::Event::PointerMoved(point)]);
                for pressed in [true, false] {
                    frame(
                        &mut app,
                        &ctx,
                        vec![pointer_button(point, egui::PointerButton::Primary, pressed)],
                    );
                }
                assert_eq!(
                    app.workspace.scenes[0]
                        .document
                        .edit_mode
                        .visible_selected_face_count(&model),
                    1,
                    "{previous} -> {route}: the primary gesture must select"
                );
            }
        }
        Ok(())
    }

    fn arm_previous_tool(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        tool: &str,
        model: &Scene,
    ) -> anyhow::Result<()> {
        let mut scene = app
            .active_context()
            .ok_or_else(|| anyhow::anyhow!("active scene"))?;
        match tool {
            "align" => scene.arm_align_tool(ctx),
            "cut" => scene.tools.cut_view.enable(),
            "measure" => scene
                .tools
                .measure
                .arm(crate::measure::measure_tool::MeasureMode::Ruler),
            _ => scene.begin_bridge_split_from_layer(model, model.meshes()[0].id()),
        }
        Ok(())
    }

    #[test]
    fn entering_a_viewport_tool_finishes_edit_selection() -> anyhow::Result<()> {
        for ((tool, shortcut), route) in [
            ("align", egui::Key::A),
            ("cut", egui::Key::C),
            ("measure", egui::Key::M),
        ]
        .into_iter()
        .flat_map(|tool| [(tool, "key"), (tool, "toolbar")])
        {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = input_app(&ctx);
            let model = crate::app::app_test_support::named_scene("surface", 0.0);
            app.active_context()
                .ok_or_else(|| anyhow::anyhow!("active scene"))?
                .set_scene(model.clone(), true);
            frame(&mut app, &ctx, vec![key(egui::Key::E)]);
            click_control(&mut app, &ctx, "All")?;
            match route {
                "key" => {
                    frame(&mut app, &ctx, vec![key(shortcut)]);
                }
                _ => click_control(
                    &mut app,
                    &ctx,
                    match tool {
                        "align" => "Align",
                        "cut" => "Cut View",
                        _ => "Ruler",
                    },
                )?,
            }
            let scene = &app.workspace.scenes[0];
            assert!(
                !scene.document.edit_mode.has_active_session(),
                "Edit -> {tool}: checkpoint remains open"
            );
            assert_eq!(
                scene.document.edit_mode.visible_selected_face_count(&model),
                0
            );
            assert!(scene.document.mesh_selection_drag.is_none());
            assert!(scene.tools.sculpt.armed.is_none());
            assert!(
                match tool {
                    "align" => scene.tools.align.tool.is_armed(),
                    "cut" => scene.tools.cut_view.is_active(),
                    _ => scene.tools.measure.is_active(),
                },
                "Edit -> {tool}: requested tool never entered"
            );
        }
        Ok(())
    }

    #[test]
    fn editing_keeps_alignment_poses_without_restoring_its_display_on_cancel() -> anyhow::Result<()>
    {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = input_app(&ctx);
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("active scene"))?
            .set_scene(
                crate::app::app_test_support::named_scene("surface", 0.0),
                true,
            );
        {
            let mut scene = app
                .active_context()
                .ok_or_else(|| anyhow::anyhow!("active scene"))?;
            scene.arm_align_tool(&ctx);
            let layer = scene
                .document
                .scene
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("mesh"))?
                .meshes()[0]
                .id();
            assert!(scene.attach_overlay_colors(
                layer,
                vec![[255, 0, 0, 255]; 3],
                crate::app::align::display::AlignOverlay::Map
            ));
            scene
                .document
                .live_scene_mut()
                .ok_or_else(|| anyhow::anyhow!("live mesh"))?
                .meshes_mut()[0]
                .transform = glam::Affine3A::from_translation(glam::vec3(4.0, 5.0, 6.0));
        }
        frame(&mut app, &ctx, vec![key(egui::Key::E)]);
        click_control(&mut app, &ctx, "Cancel")?;
        let scene = &app.workspace.scenes[0];
        let model = scene
            .document
            .scene
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("mesh"))?;
        assert_eq!(
            model.meshes()[0].transform.translation,
            glam::vec3a(4.0, 5.0, 6.0)
        );
        assert!(model.meshes()[0].overlay_kind().is_none());
        assert!(!scene.tools.align.tool.is_armed());
        assert!(scene.tools.align.session_poses.is_empty());
        Ok(())
    }

    #[test]
    fn blocked_viewport_presses_log_once_until_the_gate_changes() -> anyhow::Result<()> {
        let buffer = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let log_buffer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || DiagnosticWriter(log_buffer.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || -> anyhow::Result<()> {
            let ctx = egui::Context::default();
            let mut app = input_app(&ctx);
            let point = egui::pos2(900.0, 700.0);
            focused_frame(&mut app, &ctx, vec![], false);
            for _ in 0..2 {
                for pressed in [true, false] {
                    focused_frame(
                        &mut app,
                        &ctx,
                        vec![pointer_button(point, egui::PointerButton::Primary, pressed)],
                        false,
                    );
                }
            }
            let text = diagnostic_text(&buffer)?;
            assert_eq!(text.matches("viewport tool input skipped").count(), 1);
            assert!(text.contains("DEBUG") && text.contains("window_focused=false"));
            assert!(text.contains("active_scene_id=1") && text.contains("scene_id=1"));
            assert!(
                text.contains("modal_dialog_open=false") && text.contains("input_route=Suppressed")
            );
            assert!(text.contains("capture_owner=None"));
            app.ui.information_dialog = crate::app::information_dialog::InformationDialog::About;
            for pressed in [true, false] {
                focused_frame(
                    &mut app,
                    &ctx,
                    vec![secondary_button(point, pressed)],
                    false,
                );
            }
            let text = diagnostic_text(&buffer)?;
            assert_eq!(text.matches("viewport tool input skipped").count(), 2);
            assert!(text.contains("modal_dialog_open=true"));
            app.ui.information_dialog = crate::app::information_dialog::InformationDialog::None;
            focused_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true)], true);
            for pressed in [true, false] {
                focused_frame(
                    &mut app,
                    &ctx,
                    vec![secondary_button(point, pressed)],
                    false,
                );
            }
            assert_eq!(
                diagnostic_text(&buffer)?
                    .matches("viewport tool input skipped")
                    .count(),
                3
            );
            for pressed in [true, false] {
                focused_frame(
                    &mut app,
                    &ctx,
                    vec![secondary_button(egui::pos2(900.0, 5.0), pressed)],
                    false,
                );
            }
            assert_eq!(
                diagnostic_text(&buffer)?
                    .matches("viewport tool input skipped")
                    .count(),
                3
            );
            focused_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true)], true);
            app.active_context()
                .ok_or_else(|| anyhow::anyhow!("active scene"))?
                .queue_new_scene(crate::app::workspace::commands::SplitSide::Right);
            frame(&mut app, &ctx, vec![]);
            frame(&mut app, &ctx, vec![secondary_button(point, true)]);
            frame(
                &mut app,
                &ctx,
                vec![pointer_button(
                    egui::pos2(400.0, 700.0),
                    egui::PointerButton::Primary,
                    true,
                )],
            );
            let text = diagnostic_text(&buffer)?;
            assert!(
                text.lines().any(|line| line.contains("scene_id=1")
                    && line.contains("active_scene_id=2")
                    && line.contains("is_active=false")
                    && line.contains("input_route=Captured")
                    && line.contains("capture_owner=Some")),
                "a blocked peer press must identify the captured scene: {text}"
            );
            Ok(())
        })
    }

    fn diagnostic_text(buffer: &std::sync::Mutex<Vec<u8>>) -> anyhow::Result<String> {
        let bytes = buffer
            .lock()
            .map_err(|_| anyhow::anyhow!("log buffer lock"))?
            .clone();
        Ok(String::from_utf8(bytes)?)
    }

    struct DiagnosticWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for DiagnosticWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("log buffer lock"))?
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn loads_scene_switches_resize_and_layout_changes_keep_the_camera_in_its_pane(
    ) -> anyhow::Result<()> {
        use crate::app::workspace::commands::{SplitSide, WorkspaceCommand};
        use crate::app::workspace::layout::WorkspaceLayout;
        use crate::scene_loading::SceneLoadMode;

        let ctx = egui::Context::default();
        let mut app = input_app(&ctx);
        app.persistence.settings.frame_scene_on_open = false;
        let size = egui::vec2(1000.0, 800.0);
        for offset in [0.0, 10.0] {
            let model = crate::app::app_test_support::named_scene("surface", offset);
            app.loader
                .install_active(crate::app::app_test_support::delivered_load(
                    &app,
                    model,
                    SceneLoadMode::Append,
                    "surface.stl",
                ));
            frame(&mut app, &ctx, vec![]);
            let camera = app.workspace.scenes[0]
                .render
                .camera
                .ok_or_else(|| anyhow::anyhow!("first camera"))?;
            assert_eq!(camera.target, glam::vec3(0.5, 0.5, 0.0));
            assert_camera_in_pane(&mut app, &ctx, egui::pos2(900.0, 700.0), size)?;
        }
        let first = app.workspace.scenes[0].target();
        let first_camera = app.workspace.scenes[0].render.camera;
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("active scene"))?
            .queue_new_scene(SplitSide::Right);
        frame(&mut app, &ctx, vec![]);
        app.active_context()
            .ok_or_else(|| anyhow::anyhow!("second scene"))?
            .set_scene(
                crate::app::app_test_support::named_scene("peer", 20.0),
                false,
            );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.workspace.scenes[1]
                .render
                .camera
                .map(|camera| camera.target),
            Some(glam::vec3(20.5, 0.5, 0.0))
        );
        assert_camera_in_pane(&mut app, &ctx, egui::pos2(900.0, 700.0), size)?;
        frame(&mut app, &ctx, vec![key(egui::Key::F6)]);
        assert_eq!(app.workspace.input.active(), first);
        assert_camera_in_pane(&mut app, &ctx, egui::pos2(400.0, 700.0), size)?;
        let split = app.workspace.layout;
        app.workspace
            .commands
            .push_back(WorkspaceCommand::SetLayout(WorkspaceLayout::single(
                first.pane,
            )));
        frame(&mut app, &ctx, vec![]);
        assert_camera_in_pane(&mut app, &ctx, egui::pos2(900.0, 700.0), size)?;
        let resized = egui::vec2(1010.0, 810.0);
        sized_frame(&mut app, &ctx, vec![], true, resized);
        assert_camera_in_pane(&mut app, &ctx, egui::pos2(900.0, 700.0), resized)?;
        app.workspace
            .commands
            .push_back(WorkspaceCommand::SetLayout(split));
        sized_frame(&mut app, &ctx, vec![], true, resized);
        assert_camera_in_pane(&mut app, &ctx, egui::pos2(400.0, 700.0), resized)?;
        assert_eq!(
            app.workspace.scenes[0]
                .render
                .camera
                .map(|camera| camera.target),
            first_camera.map(|camera| camera.target)
        );
        Ok(())
    }

    fn assert_camera_in_pane(
        app: &mut OccluViewApp,
        ctx: &egui::Context,
        point: egui::Pos2,
        size: egui::Vec2,
    ) -> anyhow::Result<()> {
        sized_frame(app, ctx, vec![egui::Event::PointerMoved(point)], true, size);
        for pressed in [true, false] {
            sized_frame(app, ctx, vec![secondary_button(point, pressed)], true, size);
        }
        let scene = app
            .workspace
            .scene(app.workspace.active_id())
            .ok_or_else(|| anyhow::anyhow!("active scene"))?;
        let viewport = scene
            .presentation
            .viewport_context_menu
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("viewport response"))?
            .0
            .rect;
        let camera = scene
            .render
            .camera
            .ok_or_else(|| anyhow::anyhow!("camera"))?;
        let (center, _) =
            crate::viewer::project_world_to_viewport(&camera, viewport, camera.target)
                .ok_or_else(|| anyhow::anyhow!("target projection"))?;
        assert!(center.distance(viewport.center()) < 0.01);
        assert!((scene.render.viewport_aspect - viewport.aspect_ratio()).abs() < 1e-6);
        sized_frame(app, ctx, vec![key(egui::Key::Escape)], true, size);
        sized_frame(app, ctx, vec![], true, size);
        Ok(())
    }

    #[test]
    fn decision_dialogs_block_layer_controls_until_dismissed() -> anyhow::Result<()> {
        for dialog in ["close", "replace", "error"] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = input_app(&ctx);
            app.active_context()
                .ok_or_else(|| anyhow::anyhow!("initial scene"))?
                .set_scene(
                    crate::app::app_test_support::named_scene("surface", 0.0),
                    true,
                );
            frame(&mut app, &ctx, vec![]);
            frame(&mut app, &ctx, vec![]);
            let scene_key = app.workspace.scenes[0].key;
            match dialog {
                "close" => {
                    app.workspace.close_request =
                        Some(crate::app::workspace::state::CloseRequest::Scene(scene_key));
                    app.ui.close_guard_open = true;
                }
                "replace" => {
                    app.ui.pending_replace_open = Some(crate::app::state_ui::PendingReplaceOpen {
                        scene_key,
                        paths: vec![std::path::PathBuf::from("replacement.stl")],
                        source: "open",
                        requested_at: Instant::now(),
                    });
                }
                _ => {
                    app.ui.app_error = Some(crate::app::state_ui::AppErrorDialog {
                        title: "Load error".to_owned(),
                        summary: "The file could not be loaded.".to_owned(),
                        details: "Invalid mesh".to_owned(),
                        action: crate::app::state_ui::AppErrorAction::None,
                    });
                }
            }
            frame(&mut app, &ctx, vec![]);
            click_control(&mut app, &ctx, "Hide layer: surface")?;
            assert!(
                app.workspace.scenes[0]
                    .document
                    .scene
                    .as_deref()
                    .is_some_and(|scene| scene.meshes()[0].visible),
                "{dialog}: the decision must block layer changes behind it"
            );
            click_control(
                &mut app,
                &ctx,
                if dialog == "error" { "Close" } else { "Cancel" },
            )?;
            assert!(
                !app.ui.command_dialog_open(),
                "{dialog}: dismissal clears its gate"
            );
            assert!(app.workspace.close_request.is_none());
            click_control(&mut app, &ctx, "Hide layer: surface")?;
            assert!(
                app.workspace.scenes[0]
                    .document
                    .scene
                    .as_deref()
                    .is_some_and(|scene| !scene.meshes()[0].visible),
                "{dialog}: dismissing the dialog must restore layer input"
            );
        }
        Ok(())
    }
}

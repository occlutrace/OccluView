use super::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
    apply_visible_selected_face_mesh_edit_action_with_limit, egui, mesh_editor_overlay,
    pick_scene_hit, AppErrorDialog, LayerContextAction, MeshEditorAction, MeshSelectionDrag,
    OccluViewApp, Scene, ScreenPolygonSelectionRequest,
};
use crate::viewer::lasso_capture::{self, LassoEvent};

impl OccluViewApp {
    pub(super) fn handle_edit_shortcuts_impl(&mut self, ctx: &egui::Context) {
        // Sculpt tool hotkeys (1 = Add/Remove, 2 = Smooth) — Edit-Mesh only,
        // handled before the other shortcuts so they claim the digit keys first.
        if self.handle_sculpt_hotkeys(ctx) {
            ctx.request_repaint();
            return;
        }

        // Consume a shortcut only when the editor can actually act on it, so
        // other contexts keep their Cmd+A/Z/Y when nothing is editable.
        let select_all_pressed = self.edit_mode.has_active_session()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
        let selected_all = select_all_pressed
            && self
                .scene
                .as_ref()
                .is_some_and(|scene| self.edit_mode.select_all_visible_selections(scene));
        if selected_all {
            self.selection_overlay_dirty = true;
            self.needs_render = true;
            self.status_message = self.scene.as_ref().map(|scene| {
                format!(
                    "Selected {} faces",
                    self.edit_mode.visible_selected_face_count(scene)
                )
            });
            ctx.request_repaint();
            return;
        }

        // Delete/Backspace removes the selected faces during an edit session
        // (exocad convention). Consumed only when it can actually act.
        let delete_pressed = self.edit_mode.has_active_session()
            && self
                .scene
                .as_ref()
                .is_some_and(|scene| self.edit_mode.visible_selected_face_count(scene) > 0)
            && !self.edit_mode.is_busy()
            && ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            });
        if delete_pressed {
            self.request_edit_session_action(LayerContextAction::DeleteSelectedFaces, ctx);
            return;
        }

        // Redo before undo: Ctrl+Shift+Z must not fall through to plain Ctrl+Z.
        let redo_pressed = self.edit_mode.redo_layer_id().is_some()
            && ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || input.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    )
            });
        let undo_pressed = !redo_pressed
            && self.edit_mode.undo_layer_id().is_some()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
        if !redo_pressed && !undo_pressed {
            return;
        }
        self.apply_history_navigation(redo_pressed, ctx);
    }

    pub(super) fn show_mesh_editor_overlay(
        &mut self,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        if !self.edit_mode.has_active_session() {
            return;
        }
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let state = mesh_editor_overlay::MeshEditorPanelState {
            selected_face_count: self.edit_mode.visible_selected_face_count(scene),
            can_undo: self.edit_mode.undo_layer_id().is_some(),
            can_redo: self.edit_mode.redo_layer_id().is_some(),
            lasso_armed: self.edit_mode.lasso_armed(),
            object_mode: self.edit_mode.object_mode(),
            through_mesh: self.edit_mode.through_mesh(),
            sculpt_armed: self.sculpt.armed,
            dirty: self.edit_mode.is_dirty(),
            busy: self.edit_mode.is_busy(),
            active_tab: self.editor_tab,
        };
        let Some(action) = mesh_editor_overlay::show(ctx, viewport_rect, state) else {
            return;
        };

        if self.handle_mesh_editor_ui_action(action, ctx) {
            return;
        }

        let layer_action = match action {
            MeshEditorAction::Delete => LayerContextAction::DeleteSelectedFaces,
            MeshEditorAction::Crop => LayerContextAction::CropToSelectedFaces,
            MeshEditorAction::Cut => LayerContextAction::CutSelectionToNewLayer,
            MeshEditorAction::Separate => LayerContextAction::SeparateSelectedComponents,
            MeshEditorAction::CloseHoles => LayerContextAction::CloseHoles,
            MeshEditorAction::SwitchTab(_)
            | MeshEditorAction::SelectAll
            | MeshEditorAction::InvertSelection
            | MeshEditorAction::ClearSelection
            | MeshEditorAction::Undo
            | MeshEditorAction::Redo
            | MeshEditorAction::ToggleLasso
            | MeshEditorAction::ToggleObject
            | MeshEditorAction::ToggleThroughMesh
            | MeshEditorAction::ToggleSculpt(_)
            | MeshEditorAction::Done
            | MeshEditorAction::Cancel => return,
        };
        self.request_edit_session_action(layer_action, ctx);
    }

    fn request_edit_session_action(
        &mut self,
        layer_action: LayerContextAction,
        ctx: &egui::Context,
    ) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let selected_layers = self
            .edit_mode
            .visible_selection_plan(&scene)
            .into_iter()
            .map(|selection| selection.layer_id)
            .collect::<Vec<_>>();
        let target_layers = selected_layers;
        if target_layers.is_empty() {
            self.status_message = Some("Select mesh faces first".to_string());
            return;
        }
        let ids_before = scene
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect::<Vec<_>>();
        let mut draft = scene.as_ref().clone();
        let close_holes_limit_mm = (layer_action == LayerContextAction::CloseHoles)
            .then(|| mesh_editor_overlay::close_holes_limit_mm(ctx))
            .flatten();
        match apply_visible_selected_face_mesh_edit_action_with_limit(
            &mut draft,
            &mut self.edit_mode,
            layer_action,
            close_holes_limit_mm,
        ) {
            Ok(apply) if apply.scene_changed => {
                let spawned = draft
                    .meshes()
                    .iter()
                    .map(occluview_core::SceneMesh::id)
                    .filter(|id| !ids_before.contains(id))
                    .collect::<Vec<_>>();
                self.commit_scene_draft(Some(scene.as_ref()), draft, ctx);
                for layer_id in target_layers.iter().chain(&spawned) {
                    self.mark_mesh_edits_unsaved(*layer_id);
                }
                self.status_message = Some(format!(
                    "{} on {} visible layer{}",
                    batch_action_label(layer_action),
                    target_layers.len(),
                    if target_layers.len() == 1 { "" } else { "s" }
                ));
            }
            Ok(_) => {
                self.status_message = Some(
                    "No changes: refine the selection; hidden layers stay untouched".to_string(),
                );
                ctx.request_repaint();
            }
            Err(error) => {
                let summary = format!("Could not edit selection: {error}");
                self.status_message = Some(summary.clone());
                self.app_error = Some(AppErrorDialog {
                    title: "Could not edit selection".to_string(),
                    summary,
                    details: format!("Multi-layer selection edit failed\n\nError:\n{error:#}"),
                });
                ctx.request_repaint();
            }
        }
    }

    /// Toggle the lasso on/off. Arming it takes over from the sculpt brush
    /// and drops any half-drawn outline/marquee.
    fn toggle_lasso_mode(&mut self, ctx: &egui::Context) {
        if !self
            .edit_mode
            .set_lasso_armed(!self.edit_mode.lasso_armed())
        {
            return;
        }
        self.abort_sculpt_stroke();
        self.sculpt.disarm();
        self.mesh_selection_drag = None;
        self.needs_render = true;
        self.status_message = Some(if self.edit_mode.lasso_armed() {
            "Lasso armed: click or drag to outline; Enter, double-click, \
             or click the start closes"
                .to_string()
        } else {
            "Lasso disarmed".to_string()
        });
        ctx.request_repaint();
    }

    /// Toggle Object pick on/off. Arming it disarms the lasso (mutually
    /// exclusive gestures) and drops any half-drawn lasso/marquee so nothing
    /// stale lingers under the new gesture.
    fn toggle_object_select_mode(&mut self, ctx: &egui::Context) {
        if !self
            .edit_mode
            .set_object_mode(!self.edit_mode.object_mode())
        {
            return;
        }
        self.abort_sculpt_stroke();
        self.sculpt.disarm();
        self.mesh_selection_drag = None;
        self.needs_render = true;
        self.status_message = Some(if self.edit_mode.object_mode() {
            "Object select: click an object to select it whole".to_string()
        } else {
            "Object select off".to_string()
        });
        ctx.request_repaint();
    }

    fn handle_mesh_editor_ui_action(
        &mut self,
        action: MeshEditorAction,
        ctx: &egui::Context,
    ) -> bool {
        match action {
            MeshEditorAction::SwitchTab(tab) => {
                self.switch_editor_tab(tab, ctx);
                true
            }
            MeshEditorAction::SelectAll => {
                if self
                    .scene
                    .as_ref()
                    .is_some_and(|scene| self.edit_mode.select_all_visible_selections(scene))
                {
                    self.selection_overlay_dirty = true;
                    self.needs_render = true;
                    self.update_visible_selection_status();
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::InvertSelection => {
                if self
                    .scene
                    .as_ref()
                    .is_some_and(|scene| self.edit_mode.invert_visible_selections(scene))
                {
                    self.selection_overlay_dirty = true;
                    self.needs_render = true;
                    self.update_visible_selection_status();
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::ClearSelection => {
                if self
                    .scene
                    .as_ref()
                    .is_some_and(|scene| self.edit_mode.clear_visible_selections(scene))
                {
                    self.selection_overlay_dirty = true;
                    self.needs_render = true;
                    self.status_message = Some("Selection cleared".to_string());
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::ToggleLasso => {
                self.toggle_lasso_mode(ctx);
                true
            }
            MeshEditorAction::ToggleObject => {
                self.toggle_object_select_mode(ctx);
                true
            }
            MeshEditorAction::ToggleSculpt(kind) => {
                self.toggle_sculpt_tool(kind, ctx);
                true
            }
            MeshEditorAction::ToggleThroughMesh => {
                if self
                    .edit_mode
                    .set_through_mesh(!self.edit_mode.through_mesh())
                {
                    self.needs_render = true;
                    self.status_message = Some(if self.edit_mode.through_mesh() {
                        "Through-mesh selection".to_string()
                    } else {
                        "Surface selection".to_string()
                    });
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::Undo => {
                self.undo_mesh_editor_action(ctx);
                true
            }
            MeshEditorAction::Redo => {
                self.apply_history_navigation(true, ctx);
                true
            }
            MeshEditorAction::Done => {
                self.finish_mesh_edit_session(ctx);
                true
            }
            MeshEditorAction::Cancel => {
                self.cancel_mesh_edit_session(ctx);
                true
            }
            MeshEditorAction::Delete
            | MeshEditorAction::Crop
            | MeshEditorAction::Cut
            | MeshEditorAction::Separate
            | MeshEditorAction::CloseHoles => false,
        }
    }

    fn update_visible_selection_status(&mut self) {
        self.status_message = self.scene.as_ref().map(|scene| {
            let faces = self.edit_mode.visible_selected_face_count(scene);
            let layers = self.edit_mode.visible_selected_layer_count(scene);
            if layers > 1 {
                format!("Selected {faces} faces across {layers} layers")
            } else {
                format!("Selected {faces} faces")
            }
        });
    }

    /// Confirm the edit session: edits stay on the live scene, the panel and
    /// selection overlay are dismissed, and the undo stack is kept so Ctrl-Z
    /// still reverts individual mesh ops afterwards.
    fn finish_mesh_edit_session(&mut self, ctx: &egui::Context) {
        self.commit_sculpt_stroke(ctx);
        self.sculpt.disarm();
        self.edit_mode.finish_edit_session();
        self.mesh_selection_drag = None;
        self.selection_overlay_dirty = true;
        self.needs_render = true;
        self.status_message = Some("Edit mesh session applied".to_string());
        ctx.request_repaint();
    }

    /// Revert the whole edit session to the captured baseline scene.
    fn cancel_mesh_edit_session(&mut self, ctx: &egui::Context) {
        self.abort_sculpt_stroke();
        self.sculpt.disarm();
        let current_scene = self.scene.clone();
        let baseline = self.edit_mode.cancel_edit_session();
        self.mesh_selection_drag = None;
        let Some(baseline) = baseline else {
            self.selection_overlay_dirty = true;
            self.needs_render = true;
            ctx.request_repaint();
            return;
        };
        self.commit_scene_draft(current_scene.as_deref(), baseline, ctx);
        self.status_message = Some("Edit mesh session reverted".to_string());
    }

    fn undo_mesh_editor_action(&mut self, ctx: &egui::Context) {
        self.apply_history_navigation(false, ctx);
    }

    /// Undo (`redo == false`) or redo (`redo == true`) the last mesh edit and
    /// commit the resulting draft scene. Shared by the panel Undo button and
    /// the Ctrl+Z / Ctrl+Y viewport shortcuts.
    fn apply_history_navigation(&mut self, redo: bool, ctx: &egui::Context) {
        // Finalize any in-flight sculpt drag first (as Done/Cancel do), so the
        // undo acts on a settled scene and the stroke's dabs are not silently
        // dropped when the coming scene swap invalidates the sculpt session.
        self.commit_sculpt_stroke(ctx);
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let paths = self.current_paths.clone();
        let mut draft = scene.as_ref().clone();
        let apply = if redo {
            apply_last_mesh_edit_redo_with_status(self, &mut draft, &paths)
        } else {
            apply_last_mesh_edit_undo_with_status(self, &mut draft, &paths)
        };
        if !apply.scene_changed {
            return;
        }
        self.commit_scene_draft(Some(scene.as_ref()), draft, ctx);
    }

    /// Swap the draft scene in as the live scene (or clear it, if the draft
    /// ended up empty) and request a repaint. The shared tail of every
    /// mesh-edit commit path (shortcuts, undo/redo, cancel).
    fn commit_scene_draft(
        &mut self,
        previous_scene: Option<&Scene>,
        draft: Scene,
        ctx: &egui::Context,
    ) {
        self.commit_structural_scene(previous_scene, draft, ctx);
    }

    pub(super) fn paint_mesh_selection_drag_overlay_impl(&self, ui: &egui::Ui) {
        let Some(drag) = self.mesh_selection_drag.as_ref() else {
            return;
        };
        match drag {
            MeshSelectionDrag::Rect { .. } => {
                let rect = drag.rect();
                ui.painter()
                    .rect_filled(rect, 3.0, crate::ui_theme::ACCENT.gamma_multiply(0.08));
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    egui::Stroke::new(1.0, crate::ui_theme::ACCENT.gamma_multiply(0.85)),
                );
            }
            MeshSelectionDrag::Lasso { points } => {
                // exocad look: a dashed ribbon with no interior fill. The live
                // cursor gets a rubber-band segment, plus a fainter hint back
                // to the first point (where a click closes the outline).
                let Some(&first) = points.first() else {
                    return;
                };
                let stroke = egui::Stroke::new(1.5, crate::ui_theme::ACCENT);
                let (dash, gap) = (6.0, 4.0);
                if points.len() >= 2 {
                    ui.painter()
                        .extend(egui::Shape::dashed_line(points, stroke, dash, gap));
                }
                if let Some(hover) = ui.ctx().pointer_hover_pos() {
                    if let Some(&last) = points.last() {
                        ui.painter().extend(egui::Shape::dashed_line(
                            &[last, hover],
                            stroke,
                            dash,
                            gap,
                        ));
                    }
                    if points.len() >= 2 {
                        let hint =
                            egui::Stroke::new(1.0, crate::ui_theme::ACCENT.gamma_multiply(0.48));
                        ui.painter().extend(egui::Shape::dashed_line(
                            &[hover, first],
                            hint,
                            dash,
                            gap,
                        ));
                    }
                }
                // Visible close target: clicking back inside this handle (or
                // double-clicking anywhere) closes the outline.
                ui.painter().circle_stroke(first, 4.0, stroke);
            }
        }
    }

    pub(super) fn track_mesh_selection_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        viewport_rect: egui::Rect,
        pan_drag_active: bool,
    ) -> bool {
        // The armed lasso owns primary clicks (exocad outline placement); the
        // default mode is the marquee rectangle drag below.
        if self.edit_mode.lasso_armed() && self.edit_mode.has_active_session() {
            return self.track_polygon_lasso(ctx, response, viewport_rect, pan_drag_active);
        }

        // Object pick owns no drag: a stationary primary click (handled in
        // `handle_primary_face_selection_click`) selects the whole component;
        // a drag falls through so the camera keeps orbit/pan/zoom.
        if self.edit_mode.object_mode() {
            return false;
        }

        self.begin_mesh_selection_drag(ctx, response, pan_drag_active);

        let Some(drag) = self.mesh_selection_drag.as_mut() else {
            return false;
        };
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(current) = response.interact_pointer_pos() {
                if let MeshSelectionDrag::Rect {
                    current: drag_current,
                    ..
                } = drag
                {
                    *drag_current = current;
                }
                ctx.request_repaint();
            }
            return false;
        }
        if !response.drag_stopped_by(egui::PointerButton::Primary) {
            return false;
        }

        let finalized = self.mesh_selection_drag.take();
        let changed = match finalized {
            // The marquee is the same region select as the lasso, expressed as
            // a 4-point polygon: one inclusion rule, Surface/Through honored.
            Some(MeshSelectionDrag::Rect { origin, current }) => {
                let rect = egui::Rect::from_two_pos(origin, current);
                let corners = [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                ];
                self.commit_screen_polygon_selection(ctx, viewport_rect, &corners)
            }
            _ => false,
        };
        ctx.request_repaint();
        changed
    }

    /// exocad lasso: outline points are placed on the primary PRESS edge (never
    /// on click-release — egui reclassifies a moved click as a drag and drops
    /// it, so press-based capture is the only way input is never lost). Holding
    /// and dragging samples freehand points; discrete presses make straight
    /// segments; the two mix freely. Enter, a double-click, or a press back on
    /// the first-point handle closes and applies the selection; Esc abandons the
    /// outline and keeps the lasso armed. The dashed ribbon is drawn with no
    /// fill by `paint_mesh_selection_drag_overlay_impl`.
    ///
    /// The decision itself lives in the pure `lasso_capture` state machine; this
    /// method is a thin adapter that feeds real egui input in and applies the
    /// returned event.
    fn track_polygon_lasso(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        viewport_rect: egui::Rect,
        pan_drag_active: bool,
    ) -> bool {
        // LMB+RMB pan takes the primary drag away from the lasso.
        if pan_drag_active {
            return false;
        }

        let (outline_active, point_count, first_point, last_point) = match &self.mesh_selection_drag
        {
            Some(MeshSelectionDrag::Lasso { points }) => (
                true,
                points.len(),
                points.first().copied(),
                points.last().copied(),
            ),
            _ => (false, 0, None, None),
        };

        // Enter/Esc are consumed only while an outline is in progress, so they
        // keep their normal meaning everywhere else.
        let (enter, escape) = if outline_active {
            ctx.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                )
            })
        } else {
            (false, false)
        };
        let (pressed, down, pointer_pos) = ctx.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.interact_pos(),
            )
        });
        let double_clicked = response.double_clicked();

        let frame = lasso_capture::LassoFrameInput {
            pressed,
            down,
            double_clicked,
            enter,
            escape,
            // `contains_pointer` stays true through a drag but is false when an
            // egui window covers the cursor, so a press on the mesh-editor window
            // never adds a lasso point.
            over_viewport: response.contains_pointer(),
            pointer_pos,
            first_point,
            last_point,
            point_count,
        };

        // Whether the lasso owns this frame's primary gesture. Returning `true`
        // stops a primary press/double-click from leaking into the face pick or
        // the camera double-click focus, while RMB orbit / MMB retarget / wheel
        // zoom (none of which set these) still fall through.
        let owns_primary = (pressed && frame.over_viewport) || double_clicked;

        match lasso_capture::decide(&frame) {
            LassoEvent::AddPoint(pos) => {
                match &mut self.mesh_selection_drag {
                    Some(MeshSelectionDrag::Lasso { points }) => points.push(pos),
                    _ => {
                        self.mesh_selection_drag =
                            Some(MeshSelectionDrag::Lasso { points: vec![pos] });
                    }
                }
                ctx.request_repaint();
                true
            }
            LassoEvent::Sample(pos) => {
                if let Some(MeshSelectionDrag::Lasso { points }) = &mut self.mesh_selection_drag {
                    points.push(pos);
                    ctx.request_repaint();
                }
                true
            }
            LassoEvent::Close => {
                if let Some(MeshSelectionDrag::Lasso { points }) = self.mesh_selection_drag.take() {
                    self.commit_screen_polygon_selection(ctx, viewport_rect, &points);
                }
                ctx.request_repaint();
                true
            }
            LassoEvent::Drop => {
                self.mesh_selection_drag = None;
                self.status_message = Some("Lasso outline dropped".to_string());
                ctx.request_repaint();
                true
            }
            LassoEvent::None => {
                // A close gesture with too few points: tell the operator and keep
                // the outline so they can add more.
                if outline_active
                    && (enter || double_clicked)
                    && point_count < lasso_capture::MIN_LASSO_POINTS
                {
                    self.status_message = Some("Lasso needs at least 3 points".to_string());
                }
                if outline_active {
                    // Keep the rubber-band segment tracking the live cursor.
                    ctx.request_repaint();
                }
                owns_primary
            }
        }
    }

    /// Run one closed screen-space outline through the shared selection API.
    /// exocad convention: outlines accumulate; holding SHIFT un-marks.
    fn commit_screen_polygon_selection(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        polygon_px: &[egui::Pos2],
    ) -> bool {
        let unmark = ctx.input(|input| input.modifiers.shift);
        let camera = self.camera;
        let scene = self.scene.clone();
        let Some((camera, scene)) = camera.zip(scene) else {
            return false;
        };
        let changed = self.edit_mode.select_faces_in_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect,
                polygon_px,
                unmark,
                through_mesh: self.edit_mode.through_mesh(),
            },
        );
        if changed {
            self.selection_overlay_dirty = true;
            self.needs_render = true;
            self.update_visible_selection_status();
        }
        changed
    }

    fn begin_mesh_selection_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
    ) {
        let drag_allowed = self.edit_mode.has_active_session()
            && self.editor_tab == mesh_editor_overlay::EditorTab::EditMesh
            && !pan_drag_active
            && !ctx.input(|input| {
                input.pointer.button_down(egui::PointerButton::Secondary)
                    || input.pointer.button_down(egui::PointerButton::Middle)
            });
        if drag_allowed && response.drag_started_by(egui::PointerButton::Primary) {
            let origin = ctx.input(|input| input.pointer.press_origin());
            let current = response.interact_pointer_pos();
            if let (Some(origin), Some(current)) = (origin, current) {
                self.mesh_selection_drag = Some(MeshSelectionDrag::Rect { origin, current });
                ctx.request_repaint();
            }
        } else if !response.dragged_by(egui::PointerButton::Primary) && !response.drag_stopped() {
            self.mesh_selection_drag = None;
        }
    }

    pub(super) fn handle_primary_face_selection_click(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) -> bool {
        if !self.edit_mode.has_active_session()
            || self.editor_tab != mesh_editor_overlay::EditorTab::EditMesh
        {
            return false;
        }
        let camera = self.camera;
        let scene = self.scene.clone();
        let pointer = response.interact_pointer_pos();
        // exocad convention: a click marks the face; SHIFT-click un-marks it.
        let unmark = ctx.input(|input| input.modifiers.shift);
        let Some(((camera, scene), pointer)) = camera.zip(scene).zip(pointer) else {
            return false;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return false;
        };
        // Object pick selects the whole component under the cursor; the default
        // single-face click marks just the picked facet. SHIFT un-marks in both.
        let acted = if self.edit_mode.object_mode() {
            self.edit_mode.select_component_hit(&scene, hit, unmark)
        } else {
            self.edit_mode
                .select_face_hit_with_mode(&scene, hit, unmark)
        };
        if !acted {
            return false;
        }

        self.selection_overlay_dirty = true;
        self.needs_render = true;
        self.update_visible_selection_status();
        ctx.request_repaint();
        true
    }
}

fn batch_action_label(action: LayerContextAction) -> &'static str {
    match action {
        LayerContextAction::CloseHoles => "Closed safe interior holes",
        LayerContextAction::DeleteSelectedFaces => "Deleted selection",
        LayerContextAction::CropToSelectedFaces => "Cropped selection",
        LayerContextAction::CutSelectionToNewLayer => "Cut selection",
        LayerContextAction::SeparateSelectedComponents => "Separated selection",
        _ => "Edited selection",
    }
}

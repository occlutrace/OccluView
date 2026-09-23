use super::mesh_editor_overlay as editor;
use super::{
    apply_last_mesh_edit_redo_with_status, apply_last_mesh_edit_undo_with_status,
    apply_visible_selected_face_mesh_edit_action_with_limit, egui, pick_scene_hit, AppErrorAction,
    AppErrorDialog, LayerContextAction, MeshEditorAction, MeshSelectionDrag, OccluViewApp, Scene,
    ScreenPolygonSelectionRequest,
};
use crate::viewer::lasso_capture::{self, LassoEvent};

impl OccluViewApp {
    /// `_unguarded`, not `_impl`: calling it skips the dialog check.
    pub(super) fn handle_edit_shortcuts_unguarded(&mut self, ctx: &egui::Context) {
        // Sculpt tool hotkeys (1 = Add/Remove, 2 = Smooth) — Mesh Editor only,
        // handled before the other shortcuts so they claim the digit keys first.
        if self.handle_sculpt_hotkeys(ctx) {
            ctx.request_repaint();
            return;
        }

        // Consume a shortcut only when the editor can actually act on it, so
        // other contexts keep their Cmd+A/Z/Y when nothing is editable.
        let select_all_pressed = self.document.edit_mode.has_active_session()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
        let selected_all =
            select_all_pressed
                && self.document.scene.as_ref().is_some_and(|scene| {
                    self.document.edit_mode.select_all_visible_selections(scene)
                });
        if selected_all {
            self.render.invalidation.selection_changed();
            self.ui.status_message = self.document.scene.as_ref().map(|scene| {
                self.ui.locale.tr_plural(
                    "edit-selected-faces",
                    &[],
                    &[(
                        "faces",
                        self.document.edit_mode.visible_selected_face_count(scene),
                    )],
                )
            });
            ctx.request_repaint();
            return;
        }

        // Delete/Backspace removes the selected faces during an edit session
        // (the dental CAD convention). Consumed only when it can actually act.
        let delete_pressed = self.document.edit_mode.has_active_session()
            && self.document.scene.as_ref().is_some_and(|scene| {
                self.document.edit_mode.visible_selected_face_count(scene) > 0
            })
            && !self.document.edit_mode.is_busy()
            && !self.tools.sculpt.is_busy()
            && ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            });
        if delete_pressed {
            self.request_edit_session_action(LayerContextAction::DeleteSelectedFaces, ctx);
            return;
        }

        // Redo before undo: Ctrl+Shift+Z must not fall through to plain Ctrl+Z.
        let redo_pressed = self.document.edit_mode.redo_layer_id().is_some()
            && ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || input.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    )
            });
        let undo_pressed = !redo_pressed
            && self.document.edit_mode.undo_layer_id().is_some()
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
        if !self.document.edit_mode.has_active_session() {
            return;
        }
        let Some(scene) = self.document.scene.as_ref() else {
            return;
        };
        let state = editor::MeshEditorPanelState {
            selected_face_count: self.document.edit_mode.visible_selected_face_count(scene),
            can_undo: self.document.edit_mode.undo_layer_id().is_some(),
            can_redo: self.document.edit_mode.redo_layer_id().is_some(),
            lasso_armed: self.document.edit_mode.lasso_armed(),
            object_mode: self.document.edit_mode.object_mode(),
            through_mesh: self.document.edit_mode.through_mesh(),
            sculpt_armed: self.tools.sculpt.armed,
            dirty: self.document.edit_mode.is_dirty(),
            busy: self.document.edit_mode.is_busy(),
            sculpt_pending: self.tools.sculpt.is_busy(),
            active_tab: self.tools.editor_tab,
        };
        let Some(action) = editor::show(ctx, viewport_rect, state, &self.ui.locale) else {
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
        if self.tools.sculpt.is_busy() {
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-finishing"));
            ctx.request_repaint();
            return;
        }
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let selected_layers = self
            .document
            .edit_mode
            .visible_selection_plan(&scene)
            .into_iter()
            .map(|selection| selection.layer_id)
            .collect::<Vec<_>>();
        let target_layers = selected_layers;
        if target_layers.is_empty() {
            self.ui.status_message = Some(self.ui.locale.tr("edit-select-faces-first"));
            return;
        }
        let ids_before = scene
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect::<Vec<_>>();
        let mut draft = scene.as_ref().clone();
        let close_holes_limit_mm = (layer_action == LayerContextAction::CloseHoles)
            .then(|| editor::close_holes_limit_mm(ctx))
            .flatten();
        match apply_visible_selected_face_mesh_edit_action_with_limit(
            &mut draft,
            &mut self.document.edit_mode,
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
                    self.document.mark_mesh_edits_unsaved(*layer_id);
                }
                self.ui.status_message = Some(self.ui.locale.tr_plural(
                    "batchedit-status",
                    &[(
                        "label",
                        &super::app_layer_edits::whole_mesh::batch_action_label(
                            layer_action,
                            &self.ui.locale,
                        ),
                    )],
                    &[("n", target_layers.len())],
                ));
            }
            Ok(_) => {
                self.ui.status_message = Some(self.ui.locale.tr("edit-no-changes-hidden"));
                ctx.request_repaint();
            }
            Err(error) => {
                let summary = self.ui.locale.tr_with(
                    "edit-apply-failed-summary",
                    &[("detail", &error.to_string())],
                );
                self.ui.status_message = Some(summary.clone());
                self.ui.app_error = Some(AppErrorDialog {
                    title: self.ui.locale.tr("edit-apply-failed-title"),
                    summary,
                    details: format!("Multi-layer selection edit failed\n\nError:\n{error:#}"),
                    action: AppErrorAction::None,
                });
                ctx.request_repaint();
            }
        }
    }

    /// Toggle the lasso on/off. Arming it takes over from the sculpt brush
    /// and drops any half-drawn outline/marquee.
    fn toggle_lasso_mode(&mut self, ctx: &egui::Context) {
        if !self
            .document
            .edit_mode
            .set_lasso_armed(!self.document.edit_mode.lasso_armed())
        {
            return;
        }
        self.abort_sculpt_stroke();
        self.tools.sculpt.disarm();
        self.document.mesh_selection_drag = None;
        self.render.invalidation.overlay_tools_changed();
        self.ui.status_message = Some(if self.document.edit_mode.lasso_armed() {
            self.ui.locale.tr("sculpt-lasso-armed")
        } else {
            self.ui.locale.tr("sculpt-lasso-off")
        });
        ctx.request_repaint();
    }

    /// Toggle Object pick on/off. Arming it disarms the lasso (mutually
    /// exclusive gestures) and drops any half-drawn lasso/marquee so nothing
    /// stale lingers under the new gesture.
    fn toggle_object_select_mode(&mut self, ctx: &egui::Context) {
        if !self
            .document
            .edit_mode
            .set_object_mode(!self.document.edit_mode.object_mode())
        {
            return;
        }
        self.abort_sculpt_stroke();
        self.tools.sculpt.disarm();
        self.document.mesh_selection_drag = None;
        self.render.invalidation.overlay_tools_changed();
        self.ui.status_message = Some(if self.document.edit_mode.object_mode() {
            self.ui.locale.tr("sculpt-object-on")
        } else {
            self.ui.locale.tr("sculpt-object-off")
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
                if self.document.scene.as_ref().is_some_and(|scene| {
                    self.document.edit_mode.select_all_visible_selections(scene)
                }) {
                    self.render.invalidation.selection_changed();
                    self.update_visible_selection_status();
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::InvertSelection => {
                if self
                    .document
                    .scene
                    .as_ref()
                    .is_some_and(|scene| self.document.edit_mode.invert_visible_selections(scene))
                {
                    self.render.invalidation.selection_changed();
                    self.update_visible_selection_status();
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::ClearSelection => {
                if self
                    .document
                    .scene
                    .as_ref()
                    .is_some_and(|scene| self.document.edit_mode.clear_visible_selections(scene))
                {
                    self.render.invalidation.selection_changed();
                    self.ui.status_message = Some(self.ui.locale.tr("sculpt-selection-cleared"));
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
                    .document
                    .edit_mode
                    .set_through_mesh(!self.document.edit_mode.through_mesh())
                {
                    self.render.invalidation.overlay_tools_changed();
                    self.ui.status_message = Some(if self.document.edit_mode.through_mesh() {
                        self.ui.locale.tr("sculpt-through-on")
                    } else {
                        self.ui.locale.tr("sculpt-through-off")
                    });
                    ctx.request_repaint();
                }
                true
            }
            MeshEditorAction::Undo => {
                self.apply_history_navigation(false, ctx);
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
        self.ui.status_message = self.document.scene.as_ref().map(|scene| {
            let faces = self.document.edit_mode.visible_selected_face_count(scene);
            let layers = self.document.edit_mode.visible_selected_layer_count(scene);
            if layers > 1 {
                self.ui.locale.tr_plural(
                    "edit-selected-faces-across",
                    &[],
                    &[("faces", faces), ("layers", layers)],
                )
            } else {
                self.ui
                    .locale
                    .tr_plural("edit-selected-faces", &[], &[("faces", faces)])
            }
        });
    }

    /// Confirm the edit session: edits stay on the live scene, the panel and
    /// selection overlay are dismissed, and the undo stack is kept so Ctrl-Z
    /// still reverts individual mesh ops afterwards.
    fn finish_mesh_edit_session(&mut self, ctx: &egui::Context) {
        if !self.commit_sculpt_stroke(ctx) {
            if self.tools.sculpt.stroke.is_some() {
                self.tools.sculpt.finish_requested = true;
            }
            return;
        }
        if self.tools.sculpt.worker_has_pending_work() {
            self.tools.sculpt.finish_requested = true;
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-finishing"));
            ctx.request_repaint();
            return;
        }
        self.finish_mesh_edit_session_now(ctx);
    }

    pub(super) fn finish_mesh_edit_session_now(&mut self, ctx: &egui::Context) {
        self.tools.sculpt.disarm();
        self.document.edit_mode.finish_edit_session();
        self.document.mesh_selection_drag = None;
        self.render.invalidation.selection_changed();
        self.ui.status_message = Some(self.ui.locale.tr("session-applied"));
        ctx.request_repaint();
    }

    /// Revert the whole edit session to the captured baseline scene.
    fn cancel_mesh_edit_session(&mut self, ctx: &egui::Context) {
        self.abort_sculpt_stroke();
        self.tools.sculpt.disarm();
        let current_scene = self.document.scene.clone();
        let baseline = self.document.edit_mode.cancel_edit_session();
        self.document.mesh_selection_drag = None;
        let Some(baseline) = baseline else {
            self.render.invalidation.selection_changed();
            ctx.request_repaint();
            return;
        };
        self.commit_scene_draft(current_scene.as_deref(), baseline, ctx);
        self.ui.status_message = Some(self.ui.locale.tr("session-reverted"));
    }

    /// Undo (`redo == false`) or redo (`redo == true`) the last mesh edit and
    /// commit the resulting draft scene. Shared by the panel Undo button and
    /// the Ctrl+Z / Ctrl+Y viewport shortcuts.
    fn apply_history_navigation(&mut self, redo: bool, ctx: &egui::Context) {
        // Finalize any in-flight sculpt drag first (as Done/Cancel do), so the
        // undo acts on a settled scene and the stroke's dabs are not silently
        // dropped when the coming scene swap invalidates the sculpt session.
        if !self.commit_sculpt_stroke(ctx) {
            if self.tools.sculpt.stroke.is_some() {
                self.tools.sculpt.pending_history = Some(redo);
            }
            return;
        }
        if self.tools.sculpt.worker_has_pending_work() {
            self.tools.sculpt.pending_history = Some(redo);
            self.ui.status_message = Some(self.ui.locale.tr("sculpt-finishing-history"));
            ctx.request_repaint();
            return;
        }
        self.apply_history_navigation_now(redo, ctx);
    }

    pub(super) fn apply_history_navigation_now(&mut self, redo: bool, ctx: &egui::Context) {
        // Close any open hand-drag BEFORE the draft is cloned from the live
        // scene. `set_scene` used to do this on the way in, which was too late:
        // `finish_align_drag` builds its `before` snapshot from whatever scene is
        // installed, so it recorded the drag against the OUTGOING scene (the one
        // this undo is about to replace) and pushed that as the newest history
        // entry. The live scene then became the draft with the edit undone while
        // the newest entry described the edit as applied — and because the guard
        // compares layer ids only, nothing refused it: the first Ctrl+Z showed
        // the edit undone and the second put it back and rewound the pose. The
        // drag is also truncated here either way (it takes `self.tools.align.drag`),
        // so closing it at this point loses the operator nothing.
        self.finish_align_drag();
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let paths = self.persistence.current_paths.clone();
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
        // Here rather than at the call sites, so Ctrl+Z gets it too: history
        // can revert the pose an align overlay describes, and the shortcut is
        // live the whole time Align Scans is open.
        self.forget_align_fit(&self.ui.locale.tr("align-status-stepped"));
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
        let Some(drag) = self.document.mesh_selection_drag.as_ref() else {
            return;
        };
        match drag {
            MeshSelectionDrag::Rect { .. } => {
                let rect = drag.rect();
                ui.painter()
                    .rect_filled(rect, 3.0, crate::ui_theme::accent().gamma_multiply(0.08));
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    egui::Stroke::new(1.0_f32, crate::ui_theme::accent().gamma_multiply(0.85)),
                    egui::StrokeKind::Middle,
                );
            }
            MeshSelectionDrag::Lasso { points } => {
                // The dental-CAD look: a dashed ribbon with no interior fill. The
                // live cursor gets a rubber-band segment, plus a fainter hint back
                // to the first point (where a click closes the outline).
                let Some(&first) = points.first() else {
                    return;
                };
                let stroke = egui::Stroke::new(1.5_f32, crate::ui_theme::accent());
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
                        let hint_color = crate::ui_theme::accent().gamma_multiply(0.48);
                        let hint = egui::Stroke::new(1.0_f32, hint_color);
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
        // The armed lasso owns primary clicks (the dental CAD outline-placement
        // convention); the default mode is the marquee rectangle drag below.
        if self.document.edit_mode.lasso_armed() && self.document.edit_mode.has_active_session() {
            return self.track_polygon_lasso(ctx, response, viewport_rect, pan_drag_active);
        }

        // Object pick owns no drag: a stationary primary click (handled in
        // `handle_primary_face_selection_click`) selects the whole component;
        // a drag falls through so the camera keeps orbit/pan/zoom.
        if self.document.edit_mode.object_mode() {
            return false;
        }

        self.begin_mesh_selection_drag(ctx, response, pan_drag_active);

        let Some(drag) = self.document.mesh_selection_drag.as_mut() else {
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

        let finalized = self.document.mesh_selection_drag.take();
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

    /// Dental CAD lasso: outline points are placed on the primary PRESS edge (never on
    /// click-release — egui reclassifies a moved click as a drag and drops it, so press-based
    /// capture is the only way input is never lost). Holding
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

        let (outline_active, point_count, first_point, last_point) =
            match &self.document.mesh_selection_drag {
                Some(MeshSelectionDrag::Lasso { points }) => (
                    true,
                    points.len(),
                    points.first().copied(),
                    points.last().copied(),
                ),
                _ => (false, 0, None, None),
            };

        // Enter/Esc are consumed only while an outline is in progress, so they
        // keep their normal meaning everywhere else — and never while a modal
        // is in front, where the key belongs to the dialog. Without this the
        // outline ate the Escape and the modal (which consumes it later in the
        // same frame) stayed open, so one Escape appeared to do nothing while
        // silently dropping the outline.
        let (enter, escape) = if outline_active && !self.ui.modal_dialog_open() {
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
                match &mut self.document.mesh_selection_drag {
                    Some(MeshSelectionDrag::Lasso { points }) => points.push(pos),
                    _ => {
                        self.document.mesh_selection_drag =
                            Some(MeshSelectionDrag::Lasso { points: vec![pos] });
                    }
                }
                ctx.request_repaint();
                true
            }
            LassoEvent::Sample(pos) => {
                if let Some(MeshSelectionDrag::Lasso { points }) =
                    &mut self.document.mesh_selection_drag
                {
                    points.push(pos);
                    ctx.request_repaint();
                }
                true
            }
            LassoEvent::Close => {
                if let Some(MeshSelectionDrag::Lasso { points }) =
                    self.document.mesh_selection_drag.take()
                {
                    self.commit_screen_polygon_selection(ctx, viewport_rect, &points);
                }
                ctx.request_repaint();
                true
            }
            LassoEvent::Drop => {
                self.document.mesh_selection_drag = None;
                self.ui.status_message = Some(self.ui.locale.tr("lasso-dropped"));
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
                    self.ui.status_message = Some(self.ui.locale.tr("lasso-needs-points"));
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
    /// Dental CAD convention: outlines accumulate; holding SHIFT un-marks.
    fn commit_screen_polygon_selection(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        polygon_px: &[egui::Pos2],
    ) -> bool {
        let unmark = ctx.input(|input| input.modifiers.shift);
        let camera = self.render.camera;
        let scene = self.document.scene.clone();
        let Some((camera, scene)) = camera.zip(scene) else {
            return false;
        };
        let changed = self.document.edit_mode.select_faces_in_screen_polygon(
            &scene,
            &camera,
            ScreenPolygonSelectionRequest {
                viewport_rect,
                polygon_px,
                unmark,
                through_mesh: self.document.edit_mode.through_mesh(),
            },
        );
        if changed {
            self.render.invalidation.selection_changed();
            self.update_visible_selection_status();
        }
        changed
    }

    pub(super) fn handle_primary_face_selection_click(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) -> bool {
        if !self.document.edit_mode.has_active_session()
            || self.tools.editor_tab != editor::EditorTab::EditMesh
        {
            return false;
        }
        let camera = self.render.camera;
        let scene = self.document.scene.clone();
        let pointer = response.interact_pointer_pos();
        // Dental CAD convention: a click marks the face; SHIFT-click un-marks it.
        let unmark = ctx.input(|input| input.modifiers.shift);
        let Some(((camera, scene), pointer)) = camera.zip(scene).zip(pointer) else {
            return false;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return false;
        };
        // Object pick selects the whole component under the cursor; the default
        // single-face click marks just the picked facet. SHIFT un-marks in both.
        let acted = if self.document.edit_mode.object_mode() {
            self.document
                .edit_mode
                .select_component_hit(&scene, hit, unmark)
        } else {
            self.document
                .edit_mode
                .select_face_hit_with_mode(&scene, hit, unmark)
        };
        if !acted {
            return false;
        }
        self.render.invalidation.selection_changed();
        self.update_visible_selection_status();
        ctx.request_repaint();
        true
    }
}

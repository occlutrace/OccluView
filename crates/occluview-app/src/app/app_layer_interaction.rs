use super::{
    egui, layers_overlay, pick_scene_hit, Arc, LayerOverlayChanges, MeshSelectionDrag,
    OccluViewApp, PathBuf, Scene,
};
use crate::layers_overlay::LayerRowChange;

fn discard_lasso_outline(drag: &mut Option<MeshSelectionDrag>) -> bool {
    if matches!(drag, Some(MeshSelectionDrag::Lasso { .. })) {
        *drag = None;
        true
    } else {
        false
    }
}

const TRANSLUCENT_OPACITY: f32 = 0.35;

/// Ceiling for the automatic window growth, so a pathological layer count on
/// a small monitor does not ask the platform for an ever-taller window. The
/// compositor clamps to the monitor anyway; this keeps the request sane.
const LAYER_WINDOW_MAX_HEIGHT_PX: f32 = 1200.0;

impl OccluViewApp {
    /// Grow the OS window so the Layers panel fits every layer without
    /// scrolling.
    ///
    /// Fires only when the layer count changes and only ever grows: the panel
    /// stretches with the viewport for free, so a deficit means the operator
    /// added more layers than the current window shows. A manual shrink is
    /// never fought again until the count changes, and the in-panel scrollbar
    /// stays as the fallback for tiny screens (and Wayland compositors, which
    /// may ignore the programmatic resize).
    fn grow_window_for_layers(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        layer_count: usize,
    ) {
        if self.ui.layers_window_layer_count == Some(layer_count) {
            return;
        }
        let wanted = layers_overlay::layer_overlay_desired_height(layer_count)
            + layers_overlay::LAYER_OVERLAY_TOP_OFFSET_PX
            + layers_overlay::LAYER_OVERLAY_BOTTOM_RESERVE_PX;
        let deficit = wanted - viewport_rect.height();
        if deficit <= 0.0 {
            self.ui.layers_window_layer_count = Some(layer_count);
            return;
        }
        // A maximized/fullscreen window shrinks back out of its state if a
        // programmatic size arrives; the operator's window state wins, and the
        // in-panel scrollbar covers the difference.
        let viewport_info = ctx.input(|input| input.viewport().clone());
        if viewport_info.maximized == Some(true) || viewport_info.fullscreen == Some(true) {
            self.ui.layers_window_layer_count = Some(layer_count);
            return;
        }
        // screen_rect unavailable (first frame): leave the count unrecorded so
        // the next frame retries instead of silently dropping the request.
        let Some(screen) = ctx.input(|input| input.raw.screen_rect) else {
            return;
        };
        self.ui.layers_window_layer_count = Some(layer_count);
        let target_height = (screen.height() + deficit).min(LAYER_WINDOW_MAX_HEIGHT_PX);
        if target_height > screen.height() + 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                screen.width(),
                target_height,
            )));
        }
    }

    pub(super) fn show_layers_overlay(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        self.grow_window_for_layers(ctx, viewport_rect, scene.meshes().len());

        let paths = self.persistence.current_paths.clone();
        let active_layer_id = self.document.edit_mode.selected_layer_id();
        let (marked, readable) = self.contact_rows(scene.as_ref());
        let changes = layers_overlay::show(
            ui,
            viewport_rect,
            scene.as_ref(),
            &paths,
            active_layer_id,
            layers_overlay::LayerContactRows {
                marked: &marked,
                readable: &readable,
            },
            &self.ui.locale,
        );
        // Hand over the scene handle before material edits run.
        self.apply_layer_overlay_changes(scene, &paths, changes, ctx);
    }

    pub(super) fn apply_layer_overlay_changes(
        &mut self,
        scene: Arc<Scene>,
        paths: &[PathBuf],
        changes: LayerOverlayChanges,
        ctx: &egui::Context,
    ) {
        if changes.context_request.is_none() && changes.layer_edits.is_empty() {
            return;
        }
        // Release the cloned scene before an in-place material edit.
        if changes.context_request.is_none() {
            drop(scene);
            self.apply_layer_material_edits(&changes.layer_edits, ctx);
            return;
        }

        // Structural edits remain synchronous and may be expensive on large meshes.
        let mut draft = scene.as_ref().clone();
        let mut scene_changed = false;
        let mut structural_scene_change = false;
        let mut visibility_changed = Vec::new();
        if let Some(request) = changes.context_request {
            let apply =
                super::apply_layer_context_action_with_status(self, &mut draft, paths, request);
            scene_changed |= apply.scene_changed;
            structural_scene_change |= apply.structural_scene_change;
        }
        if !structural_scene_change {
            for edit in changes.layer_edits {
                if let Some(entry) = draft.meshes_mut().get_mut(edit.index) {
                    if entry.visible != edit.visible {
                        visibility_changed.push(entry.id());
                    }
                    entry.visible = edit.visible;
                    entry.opacity = edit.opacity;
                    crate::layer_actions::apply_picked_tint(entry, edit.tint, edit.tint_clicked);
                    scene_changed = true;
                }
            }
        }

        if scene_changed {
            self.remember_visibility_changes(scene.as_ref(), &draft);
            if structural_scene_change {
                self.commit_structural_scene(Some(scene.as_ref()), draft, ctx);
            } else {
                if draft.meshes().is_empty() {
                    self.clear_scene();
                } else {
                    self.update_scene_materials(draft);
                    self.invalidate_alignment_for_visibility_changes(&visibility_changed);
                }
                ctx.request_repaint();
            }
        }
    }

    /// Apply material-only edits while the document holds the sole scene handle.
    fn apply_layer_material_edits(&mut self, edits: &[LayerRowChange], ctx: &egui::Context) {
        let Some(live) = self.document.live_scene_mut() else {
            return;
        };
        let mut hidden: Vec<occluview_core::SceneMeshId> = Vec::new();
        let mut visibility_changed = Vec::new();
        let mut changed = false;
        for edit in edits {
            let Some(entry) = live.meshes_mut().get_mut(edit.index) else {
                continue;
            };
            if entry.visible != edit.visible {
                visibility_changed.push(entry.id());
            }
            if entry.visible && !edit.visible {
                hidden.push(entry.id());
            }
            entry.visible = edit.visible;
            entry.opacity = edit.opacity;
            crate::layer_actions::apply_picked_tint(entry, edit.tint, edit.tint_clicked);
            changed = true;
        }
        if !changed {
            return;
        }
        for layer in hidden {
            self.document.hidden_layer_stack.retain(|id| *id != layer);
            self.document.hidden_layer_stack.push(layer);
        }
        self.mark_scene_materials_changed();
        self.invalidate_alignment_for_visibility_changes(&visibility_changed);
        ctx.request_repaint();
    }

    /// Keep one restore history for every visibility owner, not only the
    /// Ctrl+Middle shortcut. Layer-row toggles and any future context action
    /// therefore participate in the same Shift+Ctrl restore stack.
    fn remember_visibility_changes(&mut self, before: &Scene, after: &Scene) {
        for entry in after.meshes() {
            let previous = before
                .meshes()
                .iter()
                .find(|candidate| candidate.id() == entry.id())
                .map(|candidate| candidate.visible);
            match (previous, entry.visible) {
                (Some(true), false) if !self.document.hidden_layer_stack.contains(&entry.id()) => {
                    self.document.hidden_layer_stack.push(entry.id());
                }
                (Some(false), true) => {
                    self.document
                        .hidden_layer_stack
                        .retain(|id| *id != entry.id());
                }
                _ => {}
            }
        }
        self.document
            .hidden_layer_stack
            .retain(|id| after.meshes().iter().any(|entry| entry.id() == *id));
    }

    /// Egui id under which the last right-clicked viewport layer target is
    /// stashed so the context menu can outlive the single click frame.
    fn viewport_menu_target_id() -> egui::Id {
        egui::Id::new("occluview_viewport_layer_menu_target")
    }

    /// Layer under the pointer for a viewport right-click, with the state the
    /// shared context menu needs. Returns `None` over empty space.
    fn pick_viewport_menu_target(
        &self,
        response: &egui::Response,
    ) -> Option<layers_overlay::LayerContextMenuTarget> {
        let camera = self.render.camera?;
        let scene = self.document.scene.as_ref()?;
        let pointer = response.interact_pointer_pos()?;
        let hit = pick_scene_hit(&camera, response.rect, pointer, scene)?;
        let entry = scene.meshes().get(hit.layer_index)?;
        if entry.id() != hit.layer_id {
            return None;
        }
        Some(layers_overlay::LayerContextMenuTarget {
            label: layers_overlay::layer_label(
                &self.persistence.current_paths,
                entry,
                hit.layer_index,
                &self.ui.locale,
            ),
            index: hit.layer_index,
            layer_id: hit.layer_id,
            visible: entry.visible,
            wireframe: entry.wireframe,
            face_editable: !entry.mesh.is_point_cloud(),
            can_export: !entry.mesh.vertices().is_empty(),
            show_vertex_colors: entry.show_vertex_colors,
            show_texture: entry.show_texture && entry.show_vertex_colors,
            has_color_data: entry.mesh.carries_color_data(),
            has_texture: entry.mesh.texture().is_some(),
            // Both participants wear the marks, so both are where the reading
            // is: the layer row already offers to close it on either, and the
            // viewport menu must not disagree with the row beside it.
            contacts: self.tools.contacts.pair().is_some_and(|pair| {
                pair.subject == hit.layer_id || pair.antagonist == hit.layer_id
            }),
            can_read_contacts: crate::contact::can_read_contacts(scene, hit.layer_id),
        })
    }

    /// Which layer wears contact marks, and which layers a reading can be opened
    /// on — the two facts the layer rows and the viewport menu need.
    ///
    /// Computed once per frame from the live scene rather than stored beside the
    /// reading: readability depends on the OTHER layers (a reading needs a
    /// second visible surface), so a stored copy would go stale the moment a
    /// scan was hidden or removed.
    pub(super) fn contact_rows(&self, scene: &Scene) -> (Vec<bool>, Vec<bool>) {
        // Which layers the menu offers to CLOSE on. A reading paints BOTH arches,
        // so either participant can take the marks down — `HideContacts` closes
        // the pair whichever row raised it.
        let pair = self.tools.contacts.pair();
        let marked = scene
            .meshes()
            .iter()
            .map(|entry| {
                pair.is_some_and(|pair| entry.id() == pair.subject || entry.id() == pair.antagonist)
            })
            .collect();
        let readable = scene
            .meshes()
            .iter()
            .map(|entry| crate::contact::can_read_contacts(scene, entry.id()))
            .collect();
        (marked, readable)
    }

    /// Native right-click on a mesh (a stationary secondary click, so RMB-drag
    /// still orbits) opens the same shared layer context menu as the layer row.
    pub(super) fn handle_viewport_context_menu(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) {
        if self.tools.bridge_split_active() {
            return;
        }
        // A stationary RMB first abandons an in-progress outline, then opens
        // the same layer menu. One click therefore never leaves stale lasso
        // state behind or forces the operator to right-click twice to switch
        // the editable mesh. RMB-drag orbit remains untouched.
        if response.secondary_clicked()
            && discard_lasso_outline(&mut self.document.mesh_selection_drag)
        {
            self.ui.status_message = Some(self.ui.locale.tr("lasso-dropped"));
            ctx.request_repaint();
        }
        let menu_id = Self::viewport_menu_target_id();
        // A target removed or reordered since the click must not keep serving
        // stale menu actions: validate the stashed index/id pair against the
        // live scene every frame the menu (or its next open) is served.
        let stored =
            ctx.data(|data| data.get_temp::<layers_overlay::LayerContextMenuTarget>(menu_id));
        if let Some(target) = &stored {
            let still_valid = self
                .document
                .scene
                .as_ref()
                .and_then(|scene| scene.meshes().get(target.index))
                .is_some_and(|entry| entry.id() == target.layer_id);
            if !still_valid {
                ctx.data_mut(|data| data.remove::<layers_overlay::LayerContextMenuTarget>(menu_id));
            }
        }
        if response.secondary_clicked() {
            let picked = self.pick_viewport_menu_target(response);
            ctx.data_mut(|data| match picked {
                Some(target) => {
                    data.insert_temp(menu_id, target);
                }
                None => {
                    data.remove::<layers_overlay::LayerContextMenuTarget>(menu_id);
                }
            });
        }

        let target =
            ctx.data(|data| data.get_temp::<layers_overlay::LayerContextMenuTarget>(menu_id));
        let mut request = None;
        let mut scene_request = None;
        // Empty space is not "no menu": the scene itself has actions, and one
        // of them — saving — is the only way a moved scan survives the session,
        // because the viewer has no project file.
        let (has_layers, any_moved) = self.scene_menu_state();
        response.context_menu(|ui| match target {
            Some(target) => {
                layers_overlay::show_layer_context_menu(ui, &target, &mut request, &self.ui.locale);
            }
            None => layers_overlay::show_scene_context_menu(
                ui,
                has_layers,
                any_moved,
                &mut scene_request,
                &self.ui.locale,
            ),
        });

        if let Some(action) = scene_request {
            self.apply_scene_context_action(action, ctx);
            return;
        }

        let Some(request) = request else {
            return;
        };
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let paths = self.persistence.current_paths.clone();
        self.apply_layer_overlay_changes(
            scene,
            &paths,
            LayerOverlayChanges {
                context_request: Some(request),
                layer_edits: Vec::new(),
            },
            ctx,
        );
    }

    /// Ctrl+MiddleClick: hide the layer under the cursor. The same visibility
    /// history is also fed by the Layers panel, so Shift+Ctrl restore is not
    /// tied to one input path.
    pub(super) fn hide_layer_under_cursor(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        if self.tools.bridge_split_active() {
            return;
        }
        let camera = self.render.camera;
        let scene = self.document.scene.clone();
        let pointer = response.interact_pointer_pos();
        let Some(((camera, scene), pointer)) = camera.zip(scene).zip(pointer) else {
            return;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return;
        };
        let mut draft = scene.as_ref().clone();
        let Some(entry) = draft
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == hit.layer_id)
        else {
            return;
        };
        if !entry.visible {
            return;
        }
        entry.visible = false;
        let label = layers_overlay::layer_label(
            &self.persistence.current_paths,
            entry,
            hit.layer_index,
            &self.ui.locale,
        );
        self.ui.status_message = Some(self.ui.locale.tr_with("layer-hidden", &[("label", &label)]));
        self.remember_visibility_changes(&scene, &draft);
        self.update_scene_materials(draft);
        self.invalidate_alignment_for_visibility_changes(&[hit.layer_id]);
        ctx.request_repaint();
    }

    /// Shift+MiddleClick: toggle the layer under the cursor between opaque and
    /// a translucent inspection state, remembering its previous opacity so a
    /// second toggle restores exactly what the operator had.
    pub(super) fn toggle_layer_translucency_under_cursor(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        if self.tools.bridge_split_active() {
            return;
        }
        let camera = self.render.camera;
        let scene = self.document.scene.clone();
        let pointer = response.interact_pointer_pos();
        let Some(((camera, scene), pointer)) = camera.zip(scene).zip(pointer) else {
            return;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return;
        };
        let mut draft = scene.as_ref().clone();
        let Some(entry) = draft
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == hit.layer_id)
        else {
            return;
        };
        let label = layers_overlay::layer_label(
            &self.persistence.current_paths,
            entry,
            hit.layer_index,
            &self.ui.locale,
        );
        if let Some(previous) = self
            .document
            .translucent_layer_restore
            .remove(&hit.layer_id)
        {
            entry.opacity = previous;
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr_with("layer-opaque-again", &[("label", &label)]),
            );
        } else {
            self.document
                .translucent_layer_restore
                .insert(hit.layer_id, entry.opacity);
            entry.opacity = TRANSLUCENT_OPACITY;
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr_with("layer-translucent", &[("label", &label)]),
            );
        }
        self.update_scene_materials(draft);
        ctx.request_repaint();
    }

    /// Shift+Ctrl+MiddleClick: unhide the most recently hidden layer from any
    /// visibility control that is still in the scene and still hidden.
    pub(super) fn restore_last_hidden_layer(&mut self, ctx: &egui::Context) {
        if self.tools.bridge_split_active() {
            return;
        }
        let Some(scene) = self.document.scene.clone() else {
            self.document.hidden_layer_stack.clear();
            return;
        };
        while let Some(layer_id) = self.document.hidden_layer_stack.pop() {
            let mut draft = scene.as_ref().clone();
            let Some(entry) = draft
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer_id)
            else {
                // Removed from the scene since it was hidden: try the next.
                continue;
            };
            if entry.visible {
                // Already brought back through the layers panel: skip it.
                continue;
            }
            entry.visible = true;
            let position = scene
                .as_ref()
                .meshes()
                .iter()
                .position(|probe| probe.id() == layer_id)
                .map_or(1, |index| index + 1);
            let label = layers_overlay::layer_label(
                &self.persistence.current_paths,
                entry,
                position - 1,
                &self.ui.locale,
            );
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr_with("layer-restored", &[("label", &label)]),
            );
            self.update_scene_materials(draft);
            self.invalidate_alignment_for_visibility_changes(&[layer_id]);
            ctx.request_repaint();
            return;
        }
        self.ui.status_message = Some(self.ui.locale.tr("layers-none-hidden"));
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{discard_lasso_outline, egui, MeshSelectionDrag};

    /// Both arches of a reading must offer to close it.
    ///
    /// A reading paints both participants, so a row that wears marks must say
    /// so or its menu offers to open a *second* reading on the same two scans
    /// while the first is still up. The rows are built from the pair, not from
    /// its subject: one "marked index" could only ever name one of the two.
    #[test]
    fn a_reading_marks_both_of_its_arches() {
        use crate::app::app_test_support::{push_named_layer, test_app};
        use crate::contact::ContactPair;

        let mut app = test_app("reading-marks-both-arches");
        let mut scene = super::Scene::new();
        let subject = push_named_layer(&mut scene, "lower", 0.0);
        let antagonist = push_named_layer(&mut scene, "upper", 0.1);
        let _bystander = push_named_layer(&mut scene, "wax", 5.0);
        let scene = super::Arc::new(scene);
        app.document.scene = Some(super::Arc::clone(&scene));
        app.tools.contacts.open(ContactPair {
            subject,
            antagonist,
        });

        let (marked, readable) = app.contact_rows(scene.as_ref());

        assert_eq!(marked.len(), 3, "one row per layer");
        assert!(
            marked[0] && marked[1],
            "both scans of the reading wear the marks: {marked:?}"
        );
        assert!(
            !marked[2],
            "a layer outside the pair is not a participant: {marked:?}"
        );
        assert_eq!(
            readable,
            vec![true, true, true],
            "every layer here has a visible antagonist to read against"
        );
    }

    /// Both arches of a reading must offer to close it.
    ///
    /// A reading paints both participants, so a row that wears marks must say
    /// so or its menu offers to open a *second* reading on the same two scans
    /// while the first is still up. The rows are built from the pair, not from
    /// its subject: this pins the shape that makes that possible — a flag per
    /// layer, since one "marked index" can only ever name one of the two.
    #[test]
    fn context_menu_drops_only_an_in_progress_lasso() {
        let mut lasso = Some(MeshSelectionDrag::Lasso {
            points: vec![egui::pos2(10.0, 20.0)],
        });
        assert!(discard_lasso_outline(&mut lasso));
        assert!(lasso.is_none());

        let mut marquee = Some(MeshSelectionDrag::Rect {
            origin: egui::pos2(1.0, 1.0),
            current: egui::pos2(2.0, 2.0),
        });
        assert!(!discard_lasso_outline(&mut marquee));
        assert!(matches!(marquee, Some(MeshSelectionDrag::Rect { .. })));
    }
}

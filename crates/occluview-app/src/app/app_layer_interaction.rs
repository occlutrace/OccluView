use super::{
    egui, layers_overlay, pick_scene_hit, LayerOverlayChanges, MeshSelectionDrag, OccluViewApp,
    PathBuf, Scene,
};

fn discard_lasso_outline(drag: &mut Option<MeshSelectionDrag>) -> bool {
    if matches!(drag, Some(MeshSelectionDrag::Lasso { .. })) {
        *drag = None;
        true
    } else {
        false
    }
}

const TRANSLUCENT_OPACITY: f32 = 0.35;

impl OccluViewApp {
    pub(super) fn show_layers_overlay_impl(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        let Some(scene) = self.scene.clone() else {
            return;
        };

        let paths = self.current_paths.clone();
        let active_layer_id = self.edit_mode.selected_layer_id();
        let changes =
            layers_overlay::show(ui, viewport_rect, scene.as_ref(), &paths, active_layer_id);
        self.apply_layer_overlay_changes(scene.as_ref(), &paths, changes, ctx);
    }

    pub(super) fn apply_layer_overlay_changes_impl(
        &mut self,
        scene: &Scene,
        paths: &[PathBuf],
        changes: LayerOverlayChanges,
        ctx: &egui::Context,
    ) {
        if changes.context_request.is_none() && changes.layer_edits.is_empty() {
            return;
        }

        let mut draft = scene.clone();
        let mut scene_changed = false;
        let mut structural_scene_change = false;
        if let Some(request) = changes.context_request {
            let apply =
                super::apply_layer_context_action_with_status(self, &mut draft, paths, request);
            scene_changed |= apply.scene_changed;
            structural_scene_change |= apply.structural_scene_change;
        }
        if !structural_scene_change {
            for edit in changes.layer_edits {
                if let Some(entry) = draft.meshes_mut().get_mut(edit.index) {
                    entry.visible = edit.visible;
                    entry.opacity = edit.opacity;
                    entry.tint = edit.tint;
                    scene_changed = true;
                }
            }
        }

        if scene_changed {
            if structural_scene_change {
                self.commit_structural_scene(Some(scene), draft, ctx);
            } else {
                if draft.meshes().is_empty() {
                    self.clear_scene();
                } else {
                    self.update_scene_materials(draft);
                }
                ctx.request_repaint();
            }
        }
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
        let camera = self.camera?;
        let scene = self.scene.as_ref()?;
        let pointer = response.interact_pointer_pos()?;
        let hit = pick_scene_hit(&camera, response.rect, pointer, scene)?;
        let entry = scene.meshes().get(hit.layer_index)?;
        if entry.id() != hit.layer_id {
            return None;
        }
        Some(layers_overlay::LayerContextMenuTarget {
            label: entry
                .mesh
                .name()
                .map_or_else(|| format!("layer {}", hit.layer_index + 1), String::from),
            index: hit.layer_index,
            layer_id: hit.layer_id,
            visible: entry.visible,
            wireframe: entry.wireframe,
            face_editable: !entry.mesh.is_point_cloud(),
            show_vertex_colors: entry.show_vertex_colors,
            has_color_data: entry.mesh.texture().is_some() || entry.mesh.has_vertex_colors(),
        })
    }

    /// Native right-click on a mesh (a stationary secondary click, so RMB-drag
    /// still orbits) opens the same shared layer context menu as the layer row.
    pub(super) fn handle_viewport_context_menu(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) {
        if self.bridge_split_active() {
            return;
        }
        // A stationary RMB first abandons an in-progress outline, then opens
        // the same layer menu. One click therefore never leaves stale lasso
        // state behind or forces the operator to right-click twice to switch
        // the editable mesh. RMB-drag orbit remains untouched.
        if response.secondary_clicked() && discard_lasso_outline(&mut self.mesh_selection_drag) {
            self.status_message = Some("Lasso outline dropped".to_string());
            ctx.request_repaint();
        }
        let menu_id = Self::viewport_menu_target_id();
        if response.secondary_clicked() {
            let picked = self.pick_viewport_menu_target(response);
            ctx.data_mut(|data| match picked {
                Some(target) => data.insert_temp(menu_id, target),
                None => data.remove::<layers_overlay::LayerContextMenuTarget>(menu_id),
            });
        }

        let target =
            ctx.data(|data| data.get_temp::<layers_overlay::LayerContextMenuTarget>(menu_id));
        let mut request = None;
        response.context_menu(|ui| match target {
            Some(target) => layers_overlay::show_layer_context_menu(ui, &target, &mut request),
            None => ui.close_menu(),
        });

        let Some(request) = request else {
            return;
        };
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let paths = self.current_paths.clone();
        self.apply_layer_overlay_changes(
            scene.as_ref(),
            &paths,
            LayerOverlayChanges {
                context_request: Some(request),
                layer_edits: Vec::new(),
            },
            ctx,
        );
    }

    /// Ctrl+MiddleClick: hide the layer under the cursor and remember it so
    /// Shift+Ctrl+MiddleClick can bring layers back most-recent-first.
    pub(super) fn hide_layer_under_cursor(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        if self.bridge_split_active() {
            return;
        }
        let camera = self.camera;
        let scene = self.scene.clone();
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
        let label = entry
            .mesh
            .name()
            .map_or_else(|| format!("layer {}", hit.layer_index + 1), String::from);
        self.hidden_layer_stack.push(hit.layer_id);
        self.status_message = Some(format!(
            "Hidden: {label} (Shift+Ctrl+Middle click restores)"
        ));
        self.update_scene_materials(draft);
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
        if self.bridge_split_active() {
            return;
        }
        let camera = self.camera;
        let scene = self.scene.clone();
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
        let label = entry
            .mesh
            .name()
            .map_or_else(|| format!("layer {}", hit.layer_index + 1), String::from);
        if let Some(previous) = self.translucent_layer_restore.remove(&hit.layer_id) {
            entry.opacity = previous;
            self.status_message = Some(format!("Opaque again: {label}"));
        } else {
            self.translucent_layer_restore
                .insert(hit.layer_id, entry.opacity);
            entry.opacity = TRANSLUCENT_OPACITY;
            self.status_message = Some(format!(
                "Translucent: {label} (Shift+Middle click restores)"
            ));
        }
        self.update_scene_materials(draft);
        ctx.request_repaint();
    }

    /// Shift+Ctrl+MiddleClick: unhide the most recently Ctrl+Middle-hidden
    /// layer that is still in the scene and still hidden.
    pub(super) fn restore_last_hidden_layer(&mut self, ctx: &egui::Context) {
        if self.bridge_split_active() {
            return;
        }
        let Some(scene) = self.scene.clone() else {
            self.hidden_layer_stack.clear();
            return;
        };
        while let Some(layer_id) = self.hidden_layer_stack.pop() {
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
            let label = entry
                .mesh
                .name()
                .map_or_else(|| "layer".to_string(), String::from);
            self.status_message = Some(format!("Restored: {label}"));
            self.update_scene_materials(draft);
            ctx.request_repaint();
            return;
        }
        self.status_message = Some("No middle-click-hidden layers to restore".to_string());
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{discard_lasso_outline, egui, MeshSelectionDrag};

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

use super::{
    desired_render_extent_px, egui, orbit_delta_from_drag, pick_scene_point,
    render_extent_change_requires_rerender, viewport_orbit_drag_active, viewport_pan_drag_active,
    zoom_factor_from_scroll, OccluViewApp,
};
use glam::Vec2;

#[derive(Clone, Copy)]
struct SecondaryPointerSample {
    pressed: bool,
    released: bool,
    down: bool,
    motion: egui::Vec2,
}

fn secondary_pointer_sample(ctx: &egui::Context) -> SecondaryPointerSample {
    ctx.input(|input| SecondaryPointerSample {
        pressed: input.pointer.button_pressed(egui::PointerButton::Secondary),
        released: input
            .pointer
            .button_released(egui::PointerButton::Secondary),
        down: input.pointer.button_down(egui::PointerButton::Secondary),
        motion: input.pointer.motion().unwrap_or(input.pointer.delta()),
    })
}

impl OccluViewApp {
    pub(super) fn grab_viewport_orbit_cursor_impl(&mut self, ctx: &egui::Context) {
        if self.viewport_orbit_cursor_grabbed {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
        self.viewport_orbit_cursor_grabbed = true;
    }

    pub(super) fn release_viewport_orbit_cursor_impl(&mut self, ctx: &egui::Context) {
        if !self.viewport_orbit_cursor_grabbed {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        self.viewport_orbit_cursor_grabbed = false;
    }

    pub(super) fn release_viewport_orbit_cursor_if_inactive_impl(&mut self, ctx: &egui::Context) {
        let secondary_drag_possible =
            ctx.input(|i| i.focused && i.pointer.button_down(egui::PointerButton::Secondary));
        if !secondary_drag_possible {
            self.release_viewport_orbit_cursor(ctx);
        }
    }

    pub(super) fn maybe_render_cut_view_impl(&mut self, ctx: &egui::Context) {
        // `take_needs_render` always clears the flag; the GPU slice render runs
        // only in Mesh mode (Lines draws the cached contour, no offscreen work).
        if self.cut_view.take_needs_render()
            && self.cut_view.is_active()
            && self.cut_view.wants_offscreen_slice()
            && self.can_render_cut_view()
        {
            self.render_cut_now(ctx);
            ctx.request_repaint();
        }
    }

    pub(super) fn sync_render_extent_impl(
        &mut self,
        viewport_points: egui::Vec2,
        pixels_per_point: f32,
    ) {
        if self.scene.is_none() {
            return;
        }
        let Some(desired) = desired_render_extent_px(viewport_points, pixels_per_point) else {
            return;
        };
        if render_extent_change_requires_rerender(self.render_extent_px, desired) {
            self.render_extent_px = desired;
            self.needs_render = true;
        }
    }

    fn handle_viewport_secondary_context_menu(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        sample: SecondaryPointerSample,
    ) {
        if sample.pressed {
            self.viewport_secondary_gesture_moved_since_press = false;
        }

        // Any camera motion owns the gesture, including movement below egui's
        // normal click/drag threshold. A truly stationary RMB opens the menu.
        let suppress_context_menu =
            response.secondary_clicked() && self.viewport_secondary_gesture_moved_since_press;
        if !suppress_context_menu {
            self.handle_viewport_context_menu(ctx, response);
        }
        if sample.released {
            self.viewport_secondary_gesture_moved_since_press = false;
        }
    }

    fn update_viewport_orbit_gesture(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
        sample: SecondaryPointerSample,
    ) -> bool {
        let secondary_press_owned = sample.down && response.is_pointer_button_down_on();
        let orbit_drag_active = viewport_orbit_drag_active(
            pan_drag_active,
            sample.down,
            self.viewport_orbit_cursor_grabbed,
            secondary_press_owned.then_some(sample.motion),
        );
        if (pan_drag_active || orbit_drag_active) && sample.motion.length_sq() > f32::EPSILON {
            self.viewport_secondary_gesture_moved_since_press = true;
        }
        if orbit_drag_active {
            self.grab_viewport_orbit_cursor(ctx);
        } else {
            self.release_viewport_orbit_cursor(ctx);
        }
        orbit_drag_active
    }

    pub(super) fn handle_viewport_input_impl(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        viewport_rect: egui::Rect,
    ) {
        let secondary_pointer = secondary_pointer_sample(ctx);
        self.handle_viewport_secondary_context_menu(ctx, response, secondary_pointer);

        // Modified middle clicks manage per-layer visibility and never fall
        // through to the plain middle-click camera retarget below.
        if response.clicked_by(egui::PointerButton::Middle) {
            let modifiers = ctx.input(|input| input.modifiers);
            if modifiers.command && modifiers.shift {
                self.restore_last_hidden_layer(ctx);
                return;
            }
            if modifiers.command {
                self.hide_layer_under_cursor(response, ctx);
                return;
            }
            if modifiers.shift {
                self.toggle_layer_translucency_under_cursor(response, ctx);
                return;
            }
        }

        let scene_pick =
            if response.double_clicked() || response.clicked_by(egui::PointerButton::Middle) {
                let camera = self.camera;
                let scene = self.scene.as_ref();
                response
                    .interact_pointer_pos()
                    .zip(camera)
                    .zip(scene)
                    .and_then(|((pointer, camera), scene)| {
                        pick_scene_point(&camera, response.rect, pointer, scene)
                    })
            } else {
                None
            };

        let pan_drag_active = viewport_pan_drag_active(ctx, response);
        let orbit_drag_active =
            self.update_viewport_orbit_gesture(ctx, response, pan_drag_active, secondary_pointer);

        if self.track_mesh_selection_drag(ctx, response, viewport_rect, pan_drag_active) {
            return;
        }

        // The single-click face pick belongs to the un-armed tool only; while the
        // lasso is armed, every primary gesture is the outline's (the armed
        // branch of `track_mesh_selection_drag` already handled/owned it).
        if !self.edit_mode.lasso_armed()
            && response.clicked_by(egui::PointerButton::Primary)
            && !response.dragged()
            && self.handle_primary_face_selection_click(ctx, response)
        {
            return;
        }

        let Some(camera) = self.camera.as_mut() else {
            return;
        };

        if response.double_clicked() {
            if let Some(target) = scene_pick {
                camera.target = target;
                self.request_camera_repaint(ctx);
            }
            return;
        }

        if let Some(target) = scene_pick {
            camera.target = target;
            self.request_camera_repaint(ctx);
            return;
        }

        let mut changed = false;

        if pan_drag_active {
            let pan_delta = secondary_pointer.motion;
            let viewport_size = viewport_rect.size();
            camera.pan_screen(
                Vec2::new(pan_delta.x, pan_delta.y),
                Vec2::new(viewport_size.x.max(1.0), viewport_size.y.max(1.0)),
            );
            changed = true;
        }

        if orbit_drag_active {
            if let Some(orbit_delta) =
                orbit_delta_from_drag(secondary_pointer.motion, viewport_rect.size())
            {
                camera.orbit_view_by(orbit_delta.x, orbit_delta.y);
                changed = true;
            }
        }

        if response.hovered() {
            let zoom = zoom_factor_from_scroll(ctx.input(|i| i.raw_scroll_delta.y));
            if (zoom - 1.0).abs() > f32::EPSILON {
                camera.zoom_by(zoom);
                changed = true;
            }
        }

        if changed {
            self.request_camera_repaint(ctx);
        }
    }
}

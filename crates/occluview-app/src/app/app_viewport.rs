use super::{
    desired_render_extent_px, egui, mesh_editor_overlay, orbit_delta_from_drag, pick_scene_point,
    render_extent_change_requires_rerender, viewport_orbit_drag_active, viewport_pan_drag_active,
    zoom_factor_from_scroll, MeshSelectionDrag, OccluViewApp,
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

fn pan_camera_from_point_scroll(
    camera: &mut occluview_core::Camera,
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
) -> bool {
    let scroll = super::app_input::take_raw_point_wheel_delta(ctx);
    if scroll == egui::Vec2::ZERO {
        return false;
    }
    let viewport_size = viewport_rect.size();
    camera.pan_screen(
        Vec2::new(-scroll.x, -scroll.y),
        Vec2::new(viewport_size.x.max(1.0), viewport_size.y.max(1.0)),
    );
    true
}

pub(super) fn zoom_camera_from_wheel(
    camera: &mut occluview_core::Camera,
    ctx: &egui::Context,
    zoom_sensitivity: f32,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
) -> bool {
    let wheel_zoom = zoom_factor_from_scroll(super::app_input::raw_wheel_delta(ctx).y);
    // egui-winit maps macOS trackpad magnification to Event::Zoom, whose
    // direction is opposite Camera's orthographic-height scale: spreading
    // fingers emits >1, while Camera needs <1 to zoom in.
    let pinch_zoom = ctx.input(|input| {
        input.raw.events.iter().fold(1.0, |factor, event| {
            let egui::Event::Zoom(gesture_factor) = event else {
                return factor;
            };
            if gesture_factor.is_finite() && *gesture_factor > 0.0 {
                let candidate = factor / *gesture_factor;
                if candidate.is_finite() && candidate > 0.0 {
                    candidate
                } else {
                    factor
                }
            } else {
                factor
            }
        })
    });
    let zoom = wheel_zoom * pinch_zoom;
    if (zoom - 1.0).abs() > f32::EPSILON {
        // Sensitivity is an exponent on the zoom factor: 1.0 keeps the fixed
        // gain, values below soften it, values above sharpen it, and any
        // factor stays a pure multiplicative zoom.
        return camera.zoom_at_screen_point(
            zoom.powf(zoom_sensitivity),
            Vec2::new(
                pointer.x - viewport_rect.left(),
                pointer.y - viewport_rect.top(),
            ),
            Vec2::new(viewport_rect.width(), viewport_rect.height()),
        );
    }
    false
}

/// The viewport commands one orbit-cursor lock state owes.
///
/// Locking has to lock *and* hide together: hiding the cursor without grabbing
/// it strands the pointer outside the window, and grabbing without hiding it
/// leaves the pointer jumping at the window edge during an orbit.
#[must_use]
fn orbit_cursor_commands(locked: bool) -> (egui::CursorGrab, bool) {
    let grab = if locked {
        egui::CursorGrab::Locked
    } else {
        egui::CursorGrab::None
    };
    (grab, !locked)
}

impl OccluViewApp {
    pub(super) fn grab_viewport_orbit_cursor(&mut self, ctx: &egui::Context) {
        if self.ui.viewport_orbit_cursor_grabbed {
            return;
        }
        self.set_viewport_orbit_cursor(ctx, true);
    }

    pub(super) fn release_viewport_orbit_cursor(&mut self, ctx: &egui::Context) {
        if !self.ui.viewport_orbit_cursor_grabbed {
            return;
        }
        self.set_viewport_orbit_cursor(ctx, false);
    }

    /// Apply one lock state: the pointer command, the visibility command, and
    /// the remembered state bit.
    ///
    /// The three belong to one decision and are written in one place, so no
    /// branch can lock the pointer without the state bit that releases it, or
    /// release one without showing the cursor again.
    fn set_viewport_orbit_cursor(&mut self, ctx: &egui::Context, locked: bool) {
        let (grab, visible) = orbit_cursor_commands(locked);
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(grab));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(visible));
        self.ui.viewport_orbit_cursor_grabbed = locked;
    }

    pub(super) fn release_viewport_orbit_cursor_if_inactive(&mut self, ctx: &egui::Context) {
        let secondary_drag_possible =
            ctx.input(|i| i.focused && i.pointer.button_down(egui::PointerButton::Secondary));
        if !secondary_drag_possible {
            self.release_viewport_orbit_cursor(ctx);
        }
    }

    pub(super) fn maybe_render_cut_view(&mut self, ctx: &egui::Context) {
        // `take_needs_render` always clears the flag; the GPU slice render runs
        // only in Mesh mode (Lines draws the cached contour, no offscreen work).
        if self.tools.cut_view.take_needs_render()
            && self.tools.cut_view.is_active()
            && self.tools.cut_view.wants_offscreen_slice()
            && self.can_render_cut_view()
        {
            self.render_cut_now(ctx);
            ctx.request_repaint();
        }
    }

    pub(super) fn sync_render_extent(
        &mut self,
        viewport_points: egui::Vec2,
        pixels_per_point: f32,
    ) {
        if self.document.scene.is_none() {
            return;
        }
        let Some(desired) = desired_render_extent_px(viewport_points, pixels_per_point) else {
            return;
        };
        if render_extent_change_requires_rerender(self.render.render_extent_px, desired) {
            self.render.render_extent_px = desired;
            self.render.invalidation.request_redraw();
        }
    }

    fn handle_viewport_secondary_context_menu(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        sample: SecondaryPointerSample,
    ) {
        if sample.pressed {
            self.ui.viewport_secondary_gesture_moved_since_press = false;
        }

        // Any camera motion owns the gesture, including movement below egui's
        // normal click/drag threshold. A truly stationary RMB opens the menu.
        // A press frame may also carry the cursor movement that brought the
        // pointer to the click location; it is not motion of the RMB gesture.
        // A release frame with motion is the opposite edge case: the complete
        // short drag can arrive in one egui frame and still must suppress the
        // context menu.
        if sample.released && !sample.pressed && sample.motion.length_sq() > f32::EPSILON {
            self.ui.viewport_secondary_gesture_moved_since_press = true;
        }
        let suppress_context_menu =
            response.secondary_clicked() && self.ui.viewport_secondary_gesture_moved_since_press;
        if !suppress_context_menu {
            self.handle_viewport_context_menu(ctx, response);
        }
        if sample.released {
            self.ui.viewport_secondary_gesture_moved_since_press = false;
        }
    }

    /// Modified middle clicks manage per-layer visibility. Returns true when
    /// the click was consumed and must not reach camera retarget below.
    fn handle_viewport_middle_click_modifiers(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
    ) -> bool {
        if !response.clicked_by(egui::PointerButton::Middle) {
            return false;
        }
        let modifiers = ctx.input(|input| input.modifiers);
        if modifiers.command && modifiers.shift {
            self.restore_last_hidden_layer(ctx);
        } else if modifiers.command {
            self.hide_layer_under_cursor(response, ctx);
        } else if modifiers.shift {
            self.toggle_layer_translucency_under_cursor(response, ctx);
        } else {
            return false;
        }
        true
    }

    fn update_viewport_orbit_gesture(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
        sample: SecondaryPointerSample,
    ) -> bool {
        let secondary_press_owned =
            sample.down && !sample.pressed && response.is_pointer_button_down_on();
        let orbit_drag_active = viewport_orbit_drag_active(
            pan_drag_active,
            sample.down,
            self.ui.viewport_orbit_cursor_grabbed,
            secondary_press_owned.then_some(sample.motion),
        );
        if (pan_drag_active || orbit_drag_active) && sample.motion.length_sq() > f32::EPSILON {
            self.ui.viewport_secondary_gesture_moved_since_press = true;
        }
        if orbit_drag_active {
            self.grab_viewport_orbit_cursor(ctx);
        } else {
            self.release_viewport_orbit_cursor(ctx);
        }
        orbit_drag_active
    }

    pub(super) fn handle_viewport_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        viewport_rect: egui::Rect,
        gizmo_click: bool,
    ) {
        let secondary_pointer = secondary_pointer_sample(ctx);
        let pan_drag_active = viewport_pan_drag_active(ctx, response);
        let orbit_drag_active =
            self.update_viewport_orbit_gesture(ctx, response, pan_drag_active, secondary_pointer);
        // Update the movement latch before handing the same release to egui's
        // context-menu recognizer. Otherwise the first frame of a short RMB
        // drag can still look stationary and open a menu while orbiting.
        self.handle_viewport_secondary_context_menu(ctx, response, secondary_pointer);

        // Modified middle clicks manage per-layer visibility and never fall
        // through to the plain middle-click camera retarget below.
        if self.handle_viewport_middle_click_modifiers(ctx, response) {
            return;
        }

        let scene_pick = if (self.persistence.settings.double_click_resets_camera
            && response.double_clicked())
            || response.clicked_by(egui::PointerButton::Middle)
        {
            let camera = self.render.camera;
            let scene = self.document.scene.as_ref();
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

        // An armed sculpt brush owns the primary drag ahead of every selection
        // gesture; RMB orbit / MMB retarget / wheel zoom fall through below.
        if self.handle_sculpt_drag(ctx, response, pan_drag_active) {
            return;
        }

        if self.track_mesh_selection_drag(ctx, response, viewport_rect, pan_drag_active) {
            return;
        }

        // The single-click face pick belongs to the Edit Mesh tab's un-armed
        // tool only: never on the Sculpt tab (the click is a dab) nor while the
        // lasso is armed (the outline owns the gesture).
        // A click the axis gizmo answered is a view change, not a pick. The
        // gizmo markers sit over the model, so without this the same click
        // snapped the camera AND marked the facet behind the marker.
        if self.tools.editor_tab == mesh_editor_overlay::EditorTab::EditMesh
            && !gizmo_click
            && !self.document.edit_mode.lasso_armed()
            && response.clicked_by(egui::PointerButton::Primary)
            && !response.dragged()
            && self.handle_primary_face_selection_click(ctx, response)
        {
            return;
        }

        // Shift/Ctrl + wheel resizes / re-intensifies an armed sculpt brush
        // instead of zooming the camera; consume the wheel so the zoom below
        // skips it this frame. Gated to the viewport (like the zoom) so a
        // modified scroll over a panel keeps its own meaning.
        let sculpt_wheel_used = self.adjust_sculpt_brush_from_wheel(ctx, response.hovered());

        let Some(camera) = self.render.camera.as_mut() else {
            return;
        };

        if response.double_clicked() {
            if let Some(target) = scene_pick {
                camera.focus_on(target);
                self.request_camera_repaint(ctx);
            }
            return;
        }

        if let Some(target) = scene_pick {
            camera.focus_on(target);
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
            if let Some(mut orbit_delta) =
                orbit_delta_from_drag(secondary_pointer.motion, viewport_rect.size())
            {
                let sensitivity = self.persistence.settings.orbit_sensitivity();
                orbit_delta.x *= sensitivity;
                orbit_delta.y *= sensitivity;
                camera.orbit_view_by(orbit_delta.x, orbit_delta.y);
                changed = true;
            }
        }

        // On macOS, pixel-unit scroll (a trackpad's two fingers) pans like a
        // dragged canvas and the pinch below zooms, the platform's convention.
        // Elsewhere pixel-unit scroll keeps zooming: winit reports a Wayland
        // touchpad in pixels and has no pinch there, so panning would leave
        // that touchpad with no way to zoom. A wheel the sculpt brush took
        // this frame does not also move the camera.
        if response.hovered() && cfg!(target_os = "macos") && !sculpt_wheel_used {
            changed |= pan_camera_from_point_scroll(camera, ctx, viewport_rect);
        }

        if response.hovered() && !sculpt_wheel_used {
            if let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) {
                changed |= zoom_camera_from_wheel(
                    camera,
                    ctx,
                    self.persistence.settings.zoom_sensitivity(),
                    viewport_rect,
                    pointer,
                );
            }
        }

        if changed {
            self.request_camera_repaint(ctx);
        }
    }

    /// Start a mesh-selection marquee on an explicit primary drag. Lives with
    /// the viewport input that gates it; the drag state itself is document
    /// state consumed by the mesh editor.
    pub(super) fn begin_mesh_selection_drag(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        pan_drag_active: bool,
    ) {
        let drag_allowed = self.document.edit_mode.has_active_session()
            && self.tools.editor_tab == mesh_editor_overlay::EditorTab::EditMesh
            && !pan_drag_active
            && !ctx.input(|input| {
                input.pointer.button_down(egui::PointerButton::Secondary)
                    || input.pointer.button_down(egui::PointerButton::Middle)
            });
        if drag_allowed && response.drag_started_by(egui::PointerButton::Primary) {
            let origin = ctx.input(|input| input.pointer.press_origin());
            let current = response.interact_pointer_pos();
            if let (Some(origin), Some(current)) = (origin, current) {
                self.document.mesh_selection_drag =
                    Some(MeshSelectionDrag::Rect { origin, current });
                ctx.request_repaint();
            }
        } else if !response.dragged_by(egui::PointerButton::Primary) && !response.drag_stopped() {
            self.document.mesh_selection_drag = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::zoom_camera_from_wheel;
    use eframe::egui;
    use occluview_core::Camera;

    /// Locking the orbit has to grab the pointer and hide it as one decision,
    /// and releasing has to give both back. Separate branches for the two
    /// commands could be swapped without any test noticing, which strands the
    /// pointer outside the window after an orbit.
    #[test]
    fn the_orbit_cursor_lock_and_release_are_one_decision() {
        assert_eq!(
            super::orbit_cursor_commands(true),
            (egui::CursorGrab::Locked, false),
            "a locked orbit hides the cursor while it owns it"
        );
        assert_eq!(
            super::orbit_cursor_commands(false),
            (egui::CursorGrab::None, true),
            "releasing the orbit shows the cursor again"
        );
    }

    fn camera_after_wheel(delta_y: f32, pointer: egui::Pos2) -> (bool, f32) {
        let ctx = egui::Context::default();
        let viewport_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(800.0, 600.0));
        let input = egui::RawInput {
            screen_rect: Some(viewport_rect),
            events: vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta_y),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut camera = Camera::default();
        let mut changed = false;
        ctx.run_ui(input, |ui| {
            changed = zoom_camera_from_wheel(&mut camera, ui.ctx(), 1.0, viewport_rect, pointer);
        })
        .drop_without_applying_deltas();
        (changed, camera.orthographic_height)
    }

    fn camera_after_pinch(gesture_factor: f32) -> (bool, f32) {
        let ctx = egui::Context::default();
        let viewport_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(800.0, 600.0));
        let input = egui::RawInput {
            screen_rect: Some(viewport_rect),
            events: vec![egui::Event::Zoom(gesture_factor)],
            ..Default::default()
        };
        let mut camera = Camera::default();
        let mut changed = false;
        ctx.run_ui(input, |ui| {
            changed = zoom_camera_from_wheel(
                &mut camera,
                ui.ctx(),
                1.0,
                viewport_rect,
                egui::pos2(500.0, 380.0),
            );
        })
        .drop_without_applying_deltas();
        (changed, camera.orthographic_height)
    }

    #[test]
    fn trackpad_spread_pinch_zooms_in() {
        let (changed, height) = camera_after_pinch(1.25);
        assert!(changed && height < 100.0);
    }

    #[test]
    fn trackpad_together_pinch_zooms_out() {
        let (changed, height) = camera_after_pinch(0.8);
        assert!(changed && height > 100.0);
    }

    #[test]
    fn consumer_wheel_camera_uses_vertical_delta_and_preserves_direction() {
        let (inward_changed, inward_height) = camera_after_wheel(120.0, egui::pos2(500.0, 380.0));
        let (outward_changed, outward_height) =
            camera_after_wheel(-120.0, egui::pos2(500.0, 380.0));

        assert!(inward_changed && inward_height < 100.0);
        assert!(outward_changed && outward_height > 100.0);
    }

    #[test]
    fn consumer_wheel_camera_nan_is_ignored_without_change() {
        let initial_height = Camera::default().orthographic_height;

        let (changed, height) = camera_after_wheel(f32::NAN, egui::pos2(500.0, 380.0));

        assert!(!changed);
        assert_eq!(height, initial_height);
    }
}

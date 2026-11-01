use super::PreviewSceneState;
use glam::{Vec2, Vec3};
// Zoom shares the CAD-tuned core mapping with the app viewport so the preview
// pane and the main window feel identical: no braked camera.
use occluview_core::{orbit_delta_from_pointer_motion, zoom_factor_from_scroll, Camera};

use occluview_core::{BBOX_FRAME_FILL, MIN_ORTHOGRAPHIC_HEIGHT_MM};

impl PreviewSceneState {
    pub(crate) fn orbit_drag_delta(&mut self, drag_delta_px: Vec2, viewport_px: [u16; 2]) -> bool {
        let viewport = Vec2::new(f32::from(viewport_px[0]), f32::from(viewport_px[1]));
        let Some(orbit_delta) = orbit_delta_from_pointer_motion(drag_delta_px, viewport) else {
            return false;
        };
        self.camera.orbit_view_by(orbit_delta.x, orbit_delta.y);
        true
    }

    pub(crate) fn pan_drag(&mut self, drag_delta_px: Vec2, viewport_px: [u16; 2]) -> bool {
        let viewport = Vec2::new(f32::from(viewport_px[0]), f32::from(viewport_px[1]));
        if drag_delta_px.length_squared() <= f32::EPSILON || !viewport.is_finite() {
            return false;
        }
        self.camera.pan_screen(drag_delta_px, viewport);
        true
    }

    pub(crate) fn zoom_scroll(&mut self, scroll_y: f32) -> bool {
        let scale = zoom_factor_from_scroll(scroll_y);
        if (scale - 1.0).abs() <= f32::EPSILON {
            return false;
        }
        self.camera.zoom_by(scale);
        true
    }

    /// Recenter and refit the current view onto the scene bounds without
    /// changing the viewing direction — the "I orbited/zoomed away, bring it
    /// back" action. Returns `false` for an empty scene.
    pub(crate) fn fit_view(&mut self) -> bool {
        let bbox = self.scene.bbox();
        if bbox.is_empty() {
            return false;
        }
        let radius = (0.5 * bbox.size().length()).max(1.0);
        let half_fov = 0.5 * self.camera.fovy;
        self.camera.focus_on(bbox.center());
        self.camera.orthographic_height =
            (radius * 2.0 / BBOX_FRAME_FILL).max(MIN_ORTHOGRAPHIC_HEIGHT_MM);
        self.camera.distance = if half_fov > 1e-5 {
            radius / half_fov.tan() / BBOX_FRAME_FILL
        } else {
            radius * 2.0
        };
        self.camera.fit_clip_planes_to_bbox(bbox);
        true
    }

    pub(crate) fn focus_pointer(&mut self, pointer_px: Vec2, viewport_px: [u16; 2]) -> bool {
        let viewport = Vec2::new(f32::from(viewport_px[0]), f32::from(viewport_px[1]));
        let Some((origin, direction)) = viewport_ray(&self.camera, viewport, pointer_px) else {
            return false;
        };
        let bbox = self.scene.bbox();
        let Some(target) = self
            .scene
            .pick_ray(origin, direction)
            .or_else(|| bbox.ray_entry(origin, direction))
        else {
            return false;
        };
        self.camera.focus_on(target);
        true
    }
}

fn viewport_ray(camera: &Camera, viewport_px: Vec2, pointer_px: Vec2) -> Option<(Vec3, Vec3)> {
    let width = viewport_px.x;
    let height = viewport_px.y;
    if width <= 0.0 || height <= 0.0 || !pointer_px.is_finite() {
        return None;
    }

    let x = (pointer_px.x / width) * 2.0 - 1.0;
    let y = 1.0 - (pointer_px.y / height) * 2.0;
    let eye = camera.eye();
    let forward = camera.view_direction();
    if forward.length_squared() <= f32::EPSILON {
        return None;
    }
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return None;
    }

    let half_height = camera.orthographic_height * 0.5;
    let half_width = half_height * (width / height);
    let origin = eye + right * x * half_width + up * y * half_height;
    Some((origin, forward))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_scene::test_support::{binary_stl_preview_smoke_mesh, binary_stl_triangle};

    #[test]
    fn preview_scene_focus_hits_visible_surface() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
        assert!(state.is_ok(), "preview state should load a simple STL");
        let Ok(mut state) = state else {
            panic!("required test setup or expected result was missing");
        };
        state.camera.orbit_pivot = Vec3::new(999.0, 999.0, 999.0);

        let moved = state.focus_pointer(Vec2::new(160.0, 90.0), [320, 180]);
        assert!(moved, "center focus should hit the scene");
        assert_eq!(
            state.camera.orbit_pivot, state.camera.target,
            "a picked preview surface should become the next orbit pivot"
        );
    }

    #[test]
    fn preview_scene_interaction_methods_report_meaningful_changes() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
        assert!(state.is_ok(), "preview state should load a simple STL");
        let Ok(mut state) = state else {
            panic!("required test setup or expected result was missing");
        };

        assert!(state.orbit_drag_delta(Vec2::new(28.0, 22.0), [320, 180]));
        assert!(state.pan_drag(Vec2::new(18.0, -12.0), [320, 180]));
        assert!(state.zoom_scroll(-120.0));
    }

    #[test]
    fn preview_orbit_uses_same_relative_motion_as_main_viewer() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
        assert!(state.is_ok(), "preview state should load a simple STL");
        let Ok(mut state) = state else {
            panic!("required test setup or expected result was missing");
        };

        assert!(
            state.orbit_drag_delta(Vec2::new(900.0, 0.0), [320, 180]),
            "preview orbit should accept large relative deltas without a virtual cursor edge"
        );
        assert!(state.camera.eye().is_finite());
    }

    #[test]
    fn preview_orbit_matches_main_viewer_direction_for_right_and_down_drags() {
        fn assert_matches_main_viewer(drag_delta_px: Vec2) {
            let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
            assert!(state.is_ok(), "preview state should load a simple STL");
            let Ok(mut state) = state else {
                panic!("required test setup or expected result was missing");
            };

            let before = state.camera;
            let viewport = [320, 180];
            let viewport_vec = Vec2::new(f32::from(viewport[0]), f32::from(viewport[1]));
            let orbit_delta = orbit_delta_from_pointer_motion(drag_delta_px, viewport_vec);
            assert!(
                orbit_delta.is_some(),
                "drag should map to a main-viewer orbit delta"
            );
            let Some(orbit_delta) = orbit_delta else {
                panic!("required test setup or expected result was missing");
            };

            let mut reference = before;
            reference.orbit_view_by(orbit_delta.x, orbit_delta.y);

            assert!(
                state.orbit_drag_delta(drag_delta_px, viewport),
                "preview orbit should accept the drag"
            );

            let preview_eye = state.camera.eye();
            let reference_eye = reference.eye();
            assert!(
                (preview_eye - reference_eye).length() < 1e-4,
                "preview orbit must match main viewer: preview={preview_eye} reference={reference_eye}"
            );
        }

        assert_matches_main_viewer(Vec2::new(96.0, 0.0));
        assert_matches_main_viewer(Vec2::new(0.0, 64.0));
    }

    /// Delta-level check. A downward Win32 drag driven through the preview input
    /// adapter (`orbit_drag_delta`) must move the camera in the same direction as
    /// the known-good app viewport handler for the same logical gesture, and
    /// horizontal drags must stay mirror-symmetric and pitch-free. The app math is
    /// inlined here so this test stands on its own if the app changes: the app feeds
    /// the raw egui delta (Y-down, same as Win32 client space) into
    /// `orbit_delta_from_pointer_motion` then `Camera::orbit_view_by` — see
    /// `occluview-app` viewer/interaction.rs.
    #[test]
    fn preview_input_adapter_matches_app_for_down_and_is_symmetric_left_right() {
        // Inline app-equivalent expectation for the same gesture (declared first
        // to satisfy `items_after_statements`).
        fn app_camera_after(drag: Vec2, viewport_vec: Vec2, base: Camera) -> Camera {
            let mut cam = base;
            if let Some(od) = orbit_delta_from_pointer_motion(drag, viewport_vec) {
                cam.orbit_view_by(od.x, od.y);
            }
            cam
        }

        let viewport = [320u16, 180u16];
        let viewport_vec = Vec2::new(f32::from(viewport[0]), f32::from(viewport[1]));

        // Each gesture starts from the same live preview default camera.
        let fresh_preview = || {
            PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle())
                .expect("preview state should load a simple STL")
        };

        // ---- downward drag (+dy in Win32 client space) matches the app ----
        let mut state = fresh_preview();
        let base = state.camera;
        let base_up = base.view_up();
        let down = Vec2::new(0.0, 64.0);
        assert!(
            state.orbit_drag_delta(down, viewport),
            "preview should accept the downward drag"
        );
        let app_down = app_camera_after(down, viewport_vec, base);
        assert!(
            (state.camera.eye() - app_down.eye()).length() < 1e-4,
            "preview down-drag must match app: preview={} app={}",
            state.camera.eye(),
            app_down.eye()
        );
        // A downward drag must move the eye upward along the view's screen axis.
        assert!(
            (state.camera.eye() - base.eye()).dot(base_up) > 1e-3,
            "downward drag must move the eye upward in view space: base={} now={}",
            base.eye(),
            state.camera.eye()
        );

        // ---- left/right symmetry: opposite drags stay level in view space ----
        let mut right_state = fresh_preview();
        let mut left_state = fresh_preview();
        let base = right_state.camera;
        let base_eye = base.eye();
        let base_up = base.view_up();
        let base_forward = base.view_direction();
        let base_right = base_forward.cross(base_up).normalize_or_zero();
        assert!(right_state.orbit_drag_delta(Vec2::new(64.0, 0.0), viewport));
        assert!(left_state.orbit_drag_delta(Vec2::new(-64.0, 0.0), viewport));
        // The eye's view-up coordinate is invariant under a horizontal orbit...
        assert!(
            ((right_state.camera.eye() - base_eye).dot(base_up)).abs() < 1e-4
                && ((left_state.camera.eye() - base_eye).dot(base_up)).abs() < 1e-4,
            "horizontal drags must not move the eye along screen-up"
        );
        // ...and the two directions must mirror along screen-right.
        let right_dx = (right_state.camera.eye() - base_eye).dot(base_right);
        let left_dx = (left_state.camera.eye() - base_eye).dot(base_right);
        assert!(
            right_dx.abs() > 1e-3 && (right_dx + left_dx).abs() < 1e-3,
            "left/right drags must be mirror-symmetric in yaw: right_dx={right_dx} left_dx={left_dx}"
        );
    }

    #[test]
    fn fit_view_recenters_on_the_scene_after_panning_away() {
        let mut state =
            PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh())
                .expect("preview state should load an asymmetric STL");

        assert!(state.pan_drag(Vec2::new(240.0, -180.0), [320, 180]));
        assert!(state.zoom_scroll(600.0));
        state.camera.orbit_pivot = Vec3::new(999.0, 999.0, 999.0);

        assert!(state.fit_view(), "fit should succeed on a non-empty scene");
        let center = state.scene.bbox().center();
        assert!(
            (state.camera.target - center).length() < 1e-3,
            "fit should recenter the target on the scene: {} vs {center}",
            state.camera.target
        );
        assert!(
            (state.camera.orbit_pivot - center).length() < 1e-3,
            "fit should reset the orbit pivot on the scene: {} vs {center}",
            state.camera.orbit_pivot
        );
        assert!(
            state.camera.orthographic_height.is_finite() && state.camera.orthographic_height > 0.0,
            "fit should restore a sane orthographic height"
        );
    }

    #[test]
    fn preview_scene_rerender_changes_pixels_after_orbit_and_zoom() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh());
        assert!(state.is_ok(), "preview state should load an asymmetric STL");
        let Ok(mut state) = state else {
            panic!("required test setup or expected result was missing");
        };

        let initial = state
            .render_rgba([320, 180])
            .expect("initial preview frame");
        assert!(
            state.orbit_drag_delta(Vec2::new(52.0, 34.0), [320, 180]),
            "orbit drag should update preview camera"
        );
        let orbit = state.render_rgba([320, 180]).expect("orbit preview frame");
        assert_ne!(orbit, initial, "orbit should change preview pixels");

        assert!(
            state.zoom_scroll(-120.0),
            "zoom should update preview camera"
        );
        let zoom = state.render_rgba([320, 180]).expect("zoom preview frame");
        assert_ne!(zoom, orbit, "zoom should change preview pixels");
    }
}

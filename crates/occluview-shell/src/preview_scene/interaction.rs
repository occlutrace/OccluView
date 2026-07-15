use super::PreviewSceneState;
use glam::{Vec2, Vec3};
// Zoom shares the CAD-tuned core mapping with the app viewport so the preview
// pane and the main window feel identical (owner rule: no braked camera).
use occluview_core::{
    orbit_delta_from_pointer_motion, zoom_factor_from_scroll, Aabb, Camera, CameraPreset,
};

/// Smallest orthographic height we will fit to, mirroring the core camera's own
/// floor (which is `pub(super)` there and not importable here).
const MIN_ORTHOGRAPHIC_HEIGHT_MM: f32 = 0.01;

/// Classic isometric elevation, `atan(1/sqrt(2))` ≈ 35.264°: the pitch that puts
/// three cube faces at equal foreshortening.
const ISOMETRIC_PITCH_RAD: f32 = 0.615_479_7;

/// Named camera reorientations offered by the Explorer preview context menu.
///
/// Preview files are arbitrary meshes (not canonicalised to a dental frame), so
/// these use neutral CAD labels rather than occlusal/buccal terminology. They
/// still reuse the shared `occluview_core` framing math so a preset frames a
/// mesh exactly like the desktop viewport would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewViewPreset {
    /// Look along -Z from the front (+Z).
    Front,
    /// Look straight down -Y from above (+Y).
    Top,
    /// Look along -X from the right (+X).
    Side,
    /// Three-quarter view from the +X/+Y/+Z octant.
    Isometric,
}

impl PreviewViewPreset {
    /// Build a camera framing `bbox` from this preset at the given vertical FOV.
    fn frame(self, bbox: Aabb, fovy: f32) -> Camera {
        match self {
            Self::Front => CameraPreset::Buccal.frame_bbox(bbox, fovy),
            Self::Side => CameraPreset::Mesial.frame_bbox(bbox, fovy),
            // Top and Isometric are derived by orbiting the framed front view in
            // the camera's own view axes: from the front (right=+X, up=+Y,
            // forward=-Z) a +90° pitch looks straight down, and a +45° yaw with
            // an isometric pitch swings the eye into the +X/+Y/+Z octant.
            Self::Top => {
                let mut camera = CameraPreset::Buccal.frame_bbox(bbox, fovy);
                camera.orbit_view_by(0.0, core::f32::consts::FRAC_PI_2);
                camera.fit_clip_planes_to_bbox(bbox);
                camera
            }
            Self::Isometric => {
                let mut camera = CameraPreset::Buccal.frame_bbox(bbox, fovy);
                camera.orbit_view_by(core::f32::consts::FRAC_PI_4, ISOMETRIC_PITCH_RAD);
                camera.fit_clip_planes_to_bbox(bbox);
                camera
            }
        }
    }
}

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

    /// Reorient the camera to a named preset, reframing to the scene bounds.
    /// Returns `false` for an empty scene where framing is undefined.
    pub(crate) fn apply_view_preset(&mut self, preset: PreviewViewPreset) -> bool {
        let bbox = self.scene.bbox();
        if bbox.is_empty() {
            return false;
        }
        self.camera = preset.frame(bbox, self.camera.fovy);
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
        self.camera.target = bbox.center();
        self.camera.orthographic_height = (radius * 2.0 / 0.7).max(MIN_ORTHOGRAPHIC_HEIGHT_MM);
        self.camera.distance = if half_fov > 1e-5 {
            radius / half_fov.tan() / 0.7
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
            .or_else(|| ray_aabb_entry(origin, direction, bbox))
        else {
            return false;
        };
        self.camera.target = target;
        true
    }
}

pub(crate) fn win32_preview_orbit_delta(pointer_delta_px: Vec2) -> Vec2 {
    pointer_delta_px
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

fn ray_aabb_entry(origin: Vec3, direction: Vec3, bbox: Aabb) -> Option<Vec3> {
    let mut t_min = 0.0_f32;
    let mut t_max = f32::INFINITY;
    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        let min = bbox.min[axis];
        let max = bbox.max[axis];
        if d.abs() <= f32::EPSILON {
            if o < min || o > max {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t0 = (min - o) * inv;
        let mut t1 = (max - o) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_max < t_min {
            return None;
        }
    }
    let t = if t_min >= 0.0 { t_min } else { t_max };
    if t.is_finite() && t >= 0.0 {
        Some(origin + direction * t)
    } else {
        None
    }
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
            return;
        };

        let moved = state.focus_pointer(Vec2::new(160.0, 90.0), [320, 180]);
        assert!(moved, "center focus should hit the scene");
    }

    #[test]
    fn preview_scene_interaction_methods_report_meaningful_changes() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
        assert!(state.is_ok(), "preview state should load a simple STL");
        let Ok(mut state) = state else {
            return;
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
            return;
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
                return;
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
                return;
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

    /// DELTA-LEVEL PIN. A downward Win32 drag driven through the FULL preview
    /// input adapter (`win32_preview_orbit_delta` -> `orbit_drag_delta`) must
    /// move the camera in the SAME direction as the known-good app viewport
    /// handler for the same logical gesture, and horizontal drags must stay
    /// mirror-symmetric and pitch-free. The app math is inlined here so this test
    /// stands on its own if the app changes: the app feeds the raw egui delta
    /// (Y-down, same as Win32 client space) into `orbit_delta_from_pointer_motion`
    /// then `Camera::orbit_view_by` — see `occluview-app` viewer/interaction.rs.
    #[test]
    fn preview_input_adapter_matches_app_for_down_and_is_symmetric_left_right() {
        // Inline app-equivalent expectation for the SAME gesture (declared first
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

        // Deterministic base: Front (look -Z, up +Y, right +X, pitch/yaw 0) so a
        // view yaw stays a clean horizontal rotation (eye.y fixed) and a view
        // pitch is a clean vertical one.
        let fresh_front = || {
            let mut s = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle())
                .expect("preview state should load a simple STL");
            assert!(s.apply_view_preset(PreviewViewPreset::Front));
            s
        };

        // ---- downward drag (+dy in Win32 client space) matches the app ----
        let mut state = fresh_front();
        let base = state.camera;
        let down = Vec2::new(0.0, 64.0);
        assert!(
            state.orbit_drag_delta(win32_preview_orbit_delta(down), viewport),
            "preview should accept the downward drag"
        );
        let app_down = app_camera_after(down, viewport_vec, base);
        assert!(
            (state.camera.eye() - app_down.eye()).length() < 1e-4,
            "preview down-drag must match app: preview={} app={}",
            state.camera.eye(),
            app_down.eye()
        );
        // It must be a real pitch change (not a no-op): from a +Y-up front view a
        // downward drag lifts the eye in +Y (we tip to look down at the top).
        assert!(
            state.camera.eye().y - base.eye().y > 1e-3,
            "downward drag must pitch the camera up-and-over (eye.y should rise): base={} now={}",
            base.eye(),
            state.camera.eye()
        );

        // ---- left/right symmetry: +dx and -dx are mirror yaws with no vertical leak ----
        let mut right_state = fresh_front();
        let mut left_state = fresh_front();
        assert!(
            right_state.orbit_drag_delta(win32_preview_orbit_delta(Vec2::new(64.0, 0.0)), viewport)
        );
        assert!(
            left_state.orbit_drag_delta(win32_preview_orbit_delta(Vec2::new(-64.0, 0.0)), viewport)
        );
        // Horizontal drags must not move the eye vertically (the vertical fix must
        // not leak into yaw)...
        assert!(
            (right_state.camera.eye().y - base.eye().y).abs() < 1e-4
                && (left_state.camera.eye().y - base.eye().y).abs() < 1e-4,
            "horizontal drag must not move the eye vertically"
        );
        // ...and the two directions must be mirror images in X.
        let right_dx = right_state.camera.eye().x - base.eye().x;
        let left_dx = left_state.camera.eye().x - base.eye().x;
        assert!(
            right_dx.abs() > 1e-3 && (right_dx + left_dx).abs() < 1e-3,
            "left/right drags must be mirror-symmetric in yaw: right_dx={right_dx} left_dx={left_dx}"
        );
    }

    /// The input adapter MUST stay identity. The preview orbits correctly only
    /// because its PRESENTED buffer is corrected to the app convention (see the
    /// parity map on `render::present_app_convention_rows`). Reintroducing a sign
    /// flip HERE would re-invert the pane on top of that fix — the exact
    /// leap-frog that made this bug recur. Vertical parity is owned by the
    /// present path, not the input; keep this raw.
    #[test]
    fn win32_preview_orbit_delta_is_not_reversed_before_shared_camera_mapping() {
        let pointer_delta = Vec2::new(48.0, 30.0);

        assert_eq!(
            win32_preview_orbit_delta(pointer_delta),
            pointer_delta,
            "the COM preview handler must pass raw pointer motion into the shared camera mapping; reversing it here makes Explorer Preview Pane orbit opposite to the app viewport"
        );
    }

    fn eye_offset_for_preset(preset: PreviewViewPreset) -> Vec3 {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh())
            .expect("preview state should load an asymmetric STL");
        let mut state = state;
        assert!(
            state.apply_view_preset(preset),
            "preset should reframe a non-empty scene"
        );
        state.camera.eye() - state.camera.target
    }

    #[test]
    fn preview_view_presets_look_from_the_expected_hemispheres() {
        let front = eye_offset_for_preset(PreviewViewPreset::Front);
        assert!(
            front.z > 0.0 && front.z.abs() > front.x.abs() && front.z.abs() > front.y.abs(),
            "front view should look along -Z from +Z: {front}"
        );

        let top = eye_offset_for_preset(PreviewViewPreset::Top);
        assert!(
            top.y > 0.0 && top.y.abs() > top.x.abs() && top.y.abs() > top.z.abs(),
            "top view should look straight down from +Y: {top}"
        );

        let side = eye_offset_for_preset(PreviewViewPreset::Side);
        assert!(
            side.x > 0.0 && side.x.abs() > side.y.abs() && side.x.abs() > side.z.abs(),
            "side view should look along -X from +X: {side}"
        );

        let iso = eye_offset_for_preset(PreviewViewPreset::Isometric);
        assert!(
            iso.x > 0.0 && iso.y > 0.0 && iso.z > 0.0,
            "isometric view should sit in the +X/+Y/+Z octant: {iso}"
        );
    }

    #[test]
    fn preview_view_presets_are_deterministic() {
        for preset in [
            PreviewViewPreset::Front,
            PreviewViewPreset::Top,
            PreviewViewPreset::Side,
            PreviewViewPreset::Isometric,
        ] {
            assert!(
                (eye_offset_for_preset(preset) - eye_offset_for_preset(preset)).length() < 1e-6,
                "preset framing must be deterministic for {preset:?}"
            );
        }
    }

    #[test]
    fn fit_view_recenters_on_the_scene_after_panning_away() {
        let mut state =
            PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh())
                .expect("preview state should load an asymmetric STL");

        assert!(state.pan_drag(Vec2::new(240.0, -180.0), [320, 180]));
        assert!(state.zoom_scroll(600.0));

        assert!(state.fit_view(), "fit should succeed on a non-empty scene");
        let center = state.scene.bbox().center();
        assert!(
            (state.camera.target - center).length() < 1e-3,
            "fit should recenter the target on the scene: {} vs {center}",
            state.camera.target
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
            return;
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

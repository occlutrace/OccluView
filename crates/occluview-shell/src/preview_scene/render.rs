use super::PreviewSceneState;
use crate::ShellError;
use glam::{Mat4, Vec3};
use occluview_core::{Camera, Scene, SceneMesh};
use occluview_render::{GpuCamera, GpuMeshUniform, PreparedSceneSource, ViewportSpec};

#[cfg(test)]
const PREVIEW_DARK_BACKGROUND_LINEAR: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

impl PreviewSceneState {
    #[cfg(test)]
    pub(crate) fn render_rgba(&self, size_px: [u16; 2]) -> Result<Vec<u8>, ShellError> {
        self.render_rgba_with_background(size_px, PREVIEW_DARK_BACKGROUND_LINEAR)
    }

    /// Toggle the technical wireframe overlay on every mesh in the preview and
    /// re-upload the prepared scene so the next render reflects it. Returns
    /// whether any mesh actually changed.
    pub(crate) fn set_wireframe(&mut self, enabled: bool) -> bool {
        let mut changed = false;
        for entry in self.scene.meshes_mut() {
            if entry.wireframe != enabled {
                entry.wireframe = enabled;
                changed = true;
            }
        }
        if changed {
            self.prepared_scene = self
                .offscreen
                .prepare_scene(&prepared_scene_sources(&self.scene));
        }
        changed
    }

    /// Whether the preview is currently drawing the wireframe overlay.
    pub(crate) fn is_wireframe(&self) -> bool {
        self.scene
            .meshes()
            .first()
            .is_some_and(|entry| entry.wireframe)
    }

    pub(crate) fn render_rgba_with_background(
        &self,
        size_px: [u16; 2],
        background: [f64; 4],
    ) -> Result<Vec<u8>, ShellError> {
        #[cfg(test)]
        let _guard = crate::acquire_render_test_guard();

        let width = size_px[0].max(1);
        let height = size_px[1].max(1);
        let aspect = f32::from(width) / f32::from(height.max(1));
        let mut camera = self.camera;
        camera.fit_clip_planes_to_bbox(self.scene.bbox());
        let gpu_camera = GpuCamera::new(
            build_view_matrix(&camera),
            build_proj_matrix(&camera, aspect),
            camera_studio_light_dir(&camera),
            camera.eye(),
        );
        let rgba = pollster::block_on(self.offscreen.render_prepared_viewport(
            &self.prepared_scene,
            &gpu_camera,
            ViewportSpec {
                size_px: [width, height],
                background,
            },
        ))?;
        Ok(present_app_convention_rows(rgba, [width, height]))
    }
}

/// ── PREVIEW-PANE VERTICAL PARITY (read this before touching any sign here) ──
///
/// The Explorer preview pane orbited *opposite* to the main app three times.
/// The recurring root cause is a **parity war between two independent flips**,
/// not a camera-math bug. The camera path is byte-identical to the app:
///
///   Win32 client delta (Y-down) ─▶ `win32_preview_orbit_delta` (identity)
///     ─▶ `orbit_delta_from_pointer_motion` ─▶ `Camera::orbit_view_by`
///
/// The app feeds the SAME shared functions with the SAME egui Y-down delta, so
/// for one gesture the camera moves the same way in both. What differs is the
/// number of vertical flips between the rendered framebuffer and the pixels the
/// user actually sees:
///
///   FRAMEBUFFER row 0 = TOP of the view (wgpu clip +Y ↦ framebuffer top row).
///
///   • APP path:   framebuffer ──(egui paints it directly, 0 flips)──▶ screen
///                 → screen row 0 = view top.  RIGHT-SIDE-UP.
///
///   • PREVIEW path: framebuffer
///        ──(`read_back_extent` REVERSES rows, +1 flip)──▶ readback (BOTTOM-UP)
///        ──(`pixels_to_hbitmap` negative `biHeight` = top-down DIB, 0 flip)──▶ screen
///                 → screen row 0 = view BOTTOM.  UPSIDE-DOWN (mirrored vs app).
///
/// So the preview differs from the app by EXACTLY ONE flip, living in the shared
/// `occluview_render::read_back_extent` (which the app never calls). That extra
/// flip is why "drag down moves the model UP": the camera pitches exactly like
/// the app, but the image it lands in is mirrored top↔bottom, so the motion
/// reads inverted. Left/right is untouched because nothing mirrors horizontally.
///
/// Every past "fix" toggled ONE side of this parity in isolation — either
/// negating the input `dy` (see `win32_preview_orbit_delta`, kept IDENTITY on
/// purpose) or flipping a DIB / readback sign in the render path — so the two
/// flips kept leap-frogging each other and the bug came back each time the other
/// side moved.
///
/// The durable fix, pinned by tests, is to make the PRESENTED buffer match the
/// APP convention right here, at the one cross-platform seam the preview owns,
/// and to keep the input mapping identical to the app (NO compensating sign in
/// the input adapter). We cancel `read_back_extent`'s flip once, explicitly, so
/// the buffer we hand to the (top-down, non-flipping) blit is right-side-up —
/// world +Y at row 0 — exactly like the app viewport. `pixels_to_hbitmap` must
/// stay a top-down, non-flipping DIB for this to hold end to end; that contract
/// is locked by `blit_is_top_down_non_flipping_contract`.
///
/// If a future change makes `read_back_extent` (or any upstream stage) stop
/// flipping, delete THIS flip in the same commit — the parity-guard test
/// `preview_presented_buffer_is_app_convention_right_side_up` will fail loudly
/// and point here rather than letting the inversion resurface silently.
fn present_app_convention_rows(mut rgba: Vec<u8>, size_px: [u16; 2]) -> Vec<u8> {
    let width = usize::from(size_px[0].max(1));
    let height = usize::from(size_px[1].max(1));
    let row_bytes = width * 4;
    if rgba.len() != row_bytes * height || height < 2 {
        return rgba;
    }
    // Reverse row order in place: readback is bottom-up, the app is top-down.
    let (mut top, mut bottom) = (0usize, height - 1);
    while top < bottom {
        let (head, tail) = rgba.split_at_mut(bottom * row_bytes);
        head[top * row_bytes..top * row_bytes + row_bytes].swap_with_slice(&mut tail[..row_bytes]);
        top += 1;
        bottom -= 1;
    }
    rgba
}

fn scene_mesh_uniform(entry: &SceneMesh) -> GpuMeshUniform {
    GpuMeshUniform {
        model: Mat4::from(entry.transform).to_cols_array(),
        tint: entry.tint,
        opacity: entry.opacity,
        has_texture: u32::from(entry.mesh.texture().is_some()),
        show_orientation: 0,
        show_vertex_colors: u32::from(entry.show_vertex_colors),
    }
}

pub(super) fn prepared_scene_sources(scene: &Scene) -> Vec<PreparedSceneSource<'_>> {
    scene
        .meshes()
        .iter()
        .map(|entry| PreparedSceneSource {
            mesh: &entry.mesh,
            uniform: scene_mesh_uniform(entry),
            visible: entry.visible,
            wireframe: entry.wireframe,
        })
        .collect()
}

fn build_view_matrix(camera: &Camera) -> Mat4 {
    occluview_render::camera_view_matrix(camera)
}

fn build_proj_matrix(camera: &Camera, aspect: f32) -> Mat4 {
    occluview_render::camera_ortho_proj_matrix(camera, aspect)
}

fn camera_studio_light_dir(camera: &Camera) -> Vec3 {
    let forward = camera.view_direction();
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    (-forward + up * 0.32 + right * 0.22).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_scene::interaction::PreviewViewPreset;
    use crate::preview_scene::test_support::{
        binary_stl_marker_above_blob, binary_stl_preview_smoke_mesh, binary_stl_triangle,
        lit_centroid_row_in_half,
    };
    use glam::Vec2;

    const PARITY_SIZE: [u16; 2] = [64, 64];

    /// PARITY GUARD (static, no drag). The preview's PRESENTED buffer must be
    /// right-side-up in the app convention: world **+Y** at the TOP of the frame.
    ///
    /// The app paints the framebuffer directly (row 0 = view top, +Y up); the
    /// preview instead goes through `read_back_extent` (which reverses rows) and
    /// a top-down non-flipping DIB, so without the compensating flip in
    /// `present_app_convention_rows` the marker lands at the BOTTOM and the whole
    /// pane reads upside-down — which the user feels as inverted vertical orbit.
    ///
    /// This is the "any future mirror-fix fails HERE, not silently in the input"
    /// guard: flip the present path again and the marker drops to the bottom half
    /// and this test fails, pointing straight at the parity map in `render.rs`.
    #[test]
    fn preview_presented_buffer_is_app_convention_right_side_up() {
        let mut state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_marker_above_blob())
            .expect("preview state should load the marker-above-blob fixture");
        assert!(
            state.apply_view_preset(PreviewViewPreset::Front),
            "front preset should frame the fixture (look -Z, up +Y)"
        );

        let w = usize::from(PARITY_SIZE[0]);
        let h = usize::from(PARITY_SIZE[1]);
        let frame = state
            .render_rgba(PARITY_SIZE)
            .expect("preview render should succeed");

        let marker_top = lit_centroid_row_in_half(&frame, w, h, true);
        let blob_bottom = lit_centroid_row_in_half(&frame, w, h, false);
        assert!(
            marker_top.is_some(),
            "the small +Y marker must light the TOP half in app convention; \
             a vertical mirror would push it into the bottom half"
        );
        assert!(
            blob_bottom.is_some(),
            "the big low blob must light the BOTTOM half in app convention"
        );

        // The marker is small, the blob is large: this asymmetry is what makes the
        // parity observable. Lock the direction, not just presence.
        let (Some(marker_row), Some(blob_row)) = (marker_top, blob_bottom) else {
            return;
        };
        assert!(
            marker_row < blob_row,
            "world +Y (marker, row {marker_row}) must sit ABOVE world -Y (blob, row {blob_row})"
        );
    }

    /// PIXEL-LEVEL END-TO-END REGRESSION (the unbreakable one). A downward drag
    /// (positive `dy` in Win32 client coords) through the SAME present path the
    /// pane blits must move the top marker DOWN the screen — its centroid row
    /// INCREASES — exactly like the main app viewport (drag down rotates the top
    /// toward the viewer). If either the input sign or the present parity flips,
    /// the marker moves UP instead and this fails.
    #[test]
    fn preview_downward_drag_moves_top_feature_down_on_screen() {
        let mut state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_marker_above_blob())
            .expect("preview state should load the marker-above-blob fixture");
        assert!(state.apply_view_preset(PreviewViewPreset::Front));

        let w = usize::from(PARITY_SIZE[0]);
        let h = usize::from(PARITY_SIZE[1]);

        let before_frame = state.render_rgba(PARITY_SIZE).expect("frame before drag");
        let before = lit_centroid_row_in_half(&before_frame, w, h, true)
            .expect("marker visible in top half before the drag");

        // Downward drag: Win32 client Y is down, so a downward gesture is +dy.
        assert!(
            state.orbit_drag_delta(Vec2::new(0.0, 18.0), PARITY_SIZE),
            "downward drag should orbit the preview camera"
        );

        let after_frame = state.render_rgba(PARITY_SIZE).expect("frame after drag");
        // Track the marker where it now lives (may have crossed the midline).
        let after = lit_centroid_row_in_half(&after_frame, w, h, true)
            .or_else(|| lit_centroid_row_in_half(&after_frame, w, h, false))
            .expect("marker still on screen after the drag");

        assert!(
            after > before + 1.0,
            "dragging DOWN must move the top marker DOWN the screen \
             (row {before} -> {after}); an inverted preview moves it up"
        );
    }

    /// BLIT SOURCE CONTRACT. The presented RGBA buffer's orientation only equals
    /// the on-screen orientation if `pixels_to_hbitmap` blits it top-down without
    /// reordering rows. That final blit is Windows-only, so we can't render it on
    /// Linux; instead we pin its contract at the source so a change to it fails
    /// here (and is caught against the render.rs parity map) rather than silently
    /// re-inverting the pane.
    #[test]
    fn blit_is_top_down_non_flipping_contract() {
        let com_src = include_str!("../com.rs");
        let start = com_src
            .find("fn pixels_to_hbitmap")
            .expect("pixels_to_hbitmap should exist in com.rs");
        let body = &com_src[start..(start + 1600).min(com_src.len())];
        assert!(
            body.contains("biHeight: -(height as i32)"),
            "the preview/thumbnail blit must use a NEGATIVE biHeight (top-down DIB) \
             so presented-buffer row 0 maps to the top of the window"
        );
        // A top-down DIB must NOT also reorder rows, or the two flips would cancel
        // and reintroduce the mirror. The swizzle must stay a straight row-preserving
        // zip (no `height - 1 - y` style row math).
        assert!(
            !body.contains("height as usize - 1 -") && !body.contains("- 1 - y"),
            "pixels_to_hbitmap must not vertically flip rows; keep it a straight \
             top-down, row-preserving swizzle"
        );
    }

    #[test]
    fn preview_scene_renders_rectangular_pixels() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_triangle());
        assert!(state.is_ok(), "preview state should load a simple STL");
        let Ok(state) = state else {
            return;
        };

        let pixels = state.render_rgba([320, 180]);
        assert!(pixels.is_ok(), "preview render should succeed");
        let Ok(pixels) = pixels else {
            return;
        };
        assert_eq!(pixels.len(), 320 * 180 * 4);
    }

    #[test]
    fn preview_scene_uses_black_background() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh());
        assert!(state.is_ok(), "preview state should load an asymmetric STL");
        let Ok(state) = state else {
            return;
        };

        let pixels = state
            .render_rgba([320, 180])
            .expect("preview render should succeed");
        let top_left = &pixels[..4];

        assert!(top_left[0] <= 4, "red={}", top_left[0]);
        assert!(top_left[1] <= 4, "green={}", top_left[1]);
        assert!(top_left[2] <= 4, "blue={}", top_left[2]);
        assert_eq!(top_left[3], 255, "alpha={}", top_left[3]);
    }

    #[test]
    fn wireframe_toggle_reports_change_and_alters_pixels() {
        let mut state =
            PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh())
                .expect("preview state should load an asymmetric STL");

        assert!(!state.is_wireframe(), "preview should start shaded");
        let shaded = state.render_rgba([320, 180]).expect("shaded frame");

        assert!(
            state.set_wireframe(true),
            "enabling wireframe should change state"
        );
        assert!(state.is_wireframe(), "wireframe flag should now be set");
        assert!(
            !state.set_wireframe(true),
            "re-enabling wireframe should be a no-op"
        );

        let wire = state.render_rgba([320, 180]).expect("wireframe frame");
        assert_ne!(shaded, wire, "wireframe overlay should change pixels");

        assert!(
            state.set_wireframe(false),
            "disabling wireframe should change state"
        );
        assert!(!state.is_wireframe(), "wireframe flag should clear");
    }

    #[test]
    fn preview_scene_accepts_light_theme_background() {
        let state = PreviewSceneState::from_bytes(Some("stl"), &binary_stl_preview_smoke_mesh());
        assert!(state.is_ok(), "preview state should load an asymmetric STL");
        let Ok(state) = state else {
            return;
        };

        let pixels = state
            .render_rgba_with_background([320, 180], [0.80, 0.82, 0.84, 1.0])
            .expect("preview render should succeed");
        let top_left = &pixels[..4];

        assert!(top_left[0] >= 198, "red={}", top_left[0]);
        assert!(top_left[1] >= 203, "green={}", top_left[1]);
        assert!(top_left[2] >= 208, "blue={}", top_left[2]);
        assert_eq!(top_left[3], 255, "alpha={}", top_left[3]);
    }
}

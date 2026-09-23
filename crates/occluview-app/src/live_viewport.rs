//! Live `wgpu` viewport bridge for the desktop app.

use eframe::{egui, egui_wgpu, wgpu};
use occluview_render::{
    ClipPlane, GpuCamera, GpuTexture, PreparedScene, PreparedSceneSource, PreparedSceneTopology,
    PreparedSceneUpdate, RenderError, Renderer, SculptBrushUniform, SculptSurfaceFeedbackRequest,
    SculptToolUniform,
};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) type SharedLiveViewport = Arc<Mutex<LiveViewport>>;

/// One frame's display-only Sculpt cursor. The target identity is kept next
/// to the GPU inputs so a stale hover cannot light a different layer after a
/// scene reorder or topology rebuild.
#[derive(Clone, Copy)]
pub(super) struct SculptCursor {
    pub(super) target_index: usize,
    pub(super) topology: PreparedSceneTopology,
    pub(super) brush: SculptBrushUniform,
    pub(super) tool: SculptToolUniform,
}

pub(super) struct LiveViewport {
    renderer: Renderer,
    fallback_texture: GpuTexture,
    camera_bind_group: wgpu::BindGroup,
    clip_buffer: wgpu::Buffer,
    clip_bind_group: wgpu::BindGroup,
    /// Whether the current clip plane is active. Drives the extra ghost pass
    /// so it runs only while a cross-section is placed.
    clip_enabled: bool,
    /// Whether the cut-away ghost pass may draw at all (a preference; the clip
    /// plane itself stays authoritative for when a section exists).
    show_ghost: bool,
    prepared_scene: Option<PreparedScene>,
    selection_overlay: Option<PreparedScene>,
    sculpt_cursor: Option<SculptCursor>,
}

impl LiveViewport {
    /// Forward the device's real texture edge; see
    /// [`occluview_render::Renderer::granted_texture_dimension`].
    pub(super) fn granted_texture_dimension(&self) -> u32 {
        self.renderer.granted_texture_dimension()
    }

    pub(super) fn from_render_state(
        render_state: &egui_wgpu::RenderState,
        sample_count: u16,
    ) -> Result<SharedLiveViewport, RenderError> {
        let renderer = Renderer::with_shared_device_sample_count(
            Arc::new(render_state.device.clone()),
            Arc::new(render_state.queue.clone()),
            render_state.target_format,
            u32::from(sample_count),
        )?;
        let fallback_texture = GpuTexture::fallback(&renderer, renderer.device(), renderer.queue());
        let camera_bind_group = renderer.camera_bind_group();
        let clip_buffer = renderer.clip_uniform_buffer();
        renderer
            .queue()
            .write_buffer(&clip_buffer, 0, bytemuck::bytes_of(&ClipPlane::disabled()));
        let clip_bind_group = renderer.clip_bind_group(&clip_buffer);
        Ok(Arc::new(Mutex::new(Self {
            renderer,
            fallback_texture,
            camera_bind_group,
            clip_buffer,
            clip_bind_group,
            clip_enabled: false,
            show_ghost: true,
            prepared_scene: None,
            selection_overlay: None,
            sculpt_cursor: None,
        })))
    }

    /// Allow drawing to resume after the operator acknowledged a fault.
    pub(super) fn clear_gpu_fault(&mut self) {
        self.renderer.clear_gpu_fault();
    }

    /// Preference gate for the cut-away ghost pass (see `paint`).
    pub(super) fn set_show_ghost(&mut self, show_ghost: bool) {
        self.show_ghost = show_ghost;
    }

    /// `splat_viewport_px` is the viewport the callback actually paints into,
    /// in physical pixels — NOT the clamped `render_extent_px`. The splat radius
    /// is a pixel quantity (`ndc_radius = POINT_SPLAT_RADIUS_PX * 2 / viewport`),
    /// so the clamped extent drew every splat at `radius * actual / clamped`:
    /// 5.25 px instead of 3.5 px on a 4K fullscreen, and undersized in a window
    /// below the floor. The camera aspect still comes from the clamped extent,
    /// which is correct.
    pub(super) fn update_view(
        &mut self,
        camera: &GpuCamera,
        splat_viewport_px: [u32; 2],
        clip_plane: ClipPlane,
    ) {
        if self.renderer.is_gpu_faulted() {
            return;
        }
        self.renderer
            .set_point_splat_viewport(splat_viewport_px[0], splat_viewport_px[1]);
        self.renderer.set_camera(camera);
        self.clip_enabled = clip_plane.enabled != 0;
        self.renderer
            .queue()
            .write_buffer(&self.clip_buffer, 0, bytemuck::bytes_of(&clip_plane));
    }

    /// Reconcile the GPU scene with the CPU one.
    ///
    /// Returns whether the vertex buffers were re-uploaded. A caller that
    /// streams its own vertices into them — the deviation map — has to know:
    /// a rebuild puts the scan's own colours back and would silently erase the
    /// map, while a uniform-only reconcile leaves it exactly where it was.
    pub(super) fn sync_scene(
        &mut self,
        sources: &[PreparedSceneSource<'_>],
        updates: &[PreparedSceneUpdate],
    ) -> bool {
        if self.renderer.is_gpu_faulted() {
            return false;
        }
        let rebuild = self
            .prepared_scene
            .as_mut()
            .is_none_or(|scene| !scene.update(&self.renderer, updates));
        if rebuild {
            let prepare_started_at = Instant::now();
            let vertex_count: usize = sources
                .iter()
                .map(|source| source.mesh.vertices().len())
                .sum();
            self.prepared_scene = Some(PreparedScene::prepare(&self.renderer, sources));
            tracing::info!(
                mesh_count = sources.len(),
                vertex_count,
                upload_ms = prepare_started_at.elapsed().as_millis(),
                "live viewport scene prepared"
            );
        }
        rebuild
    }

    /// Push only the `touched` sculpted vertices into the matching prepared
    /// entry — the hot per-dab path (see
    /// [`PreparedScene::write_entry_vertices_sparse`]).
    pub(super) fn write_scene_vertices_sparse(
        &self,
        topology: &PreparedSceneTopology,
        vertices: &[occluview_core::Vertex],
        touched: &[usize],
    ) -> bool {
        !self.renderer.is_gpu_faulted()
            && self.prepared_scene.as_ref().is_some_and(|scene| {
                scene.write_entry_vertices_sparse(&self.renderer, topology, vertices, touched)
            })
    }

    pub(super) fn write_scene_vertices(
        &self,
        topology: &PreparedSceneTopology,
        vertices: &[occluview_core::Vertex],
    ) -> bool {
        !self.renderer.is_gpu_faulted()
            && self
                .prepared_scene
                .as_ref()
                .is_some_and(|scene| scene.write_entry_vertices(&self.renderer, topology, vertices))
    }

    pub(super) fn has_prepared_scene(&self) -> bool {
        self.prepared_scene.is_some()
    }

    pub(super) fn sync_selection_overlay(&mut self, sources: &[PreparedSceneSource<'_>]) {
        if self.renderer.is_gpu_faulted() {
            self.selection_overlay = None;
            return;
        }
        self.selection_overlay =
            (!sources.is_empty()).then(|| PreparedScene::prepare(&self.renderer, sources));
    }

    /// Replace the display-only Sculpt cursor and upload its uniforms before
    /// the egui paint callback runs. Clearing it writes hidden no-op values so
    /// a cursor cannot persist after a miss, window occlusion, or scene swap.
    pub(super) fn set_sculpt_cursor(&mut self, cursor: Option<SculptCursor>) {
        if self.renderer.is_gpu_faulted() {
            self.sculpt_cursor = None;
            return;
        }
        let brush = cursor.map_or(SculptBrushUniform::hidden(), |cursor| cursor.brush);
        let tool = cursor.map_or(SculptToolUniform::hidden(), |cursor| cursor.tool);
        self.renderer.set_sculpt_brush(&brush);
        self.renderer.set_sculpt_tool(&tool);
        self.sculpt_cursor = cursor;
    }

    pub(super) fn clear(&mut self) {
        self.prepared_scene = None;
        self.selection_overlay = None;
        self.sculpt_cursor = None;
        if self.renderer.is_gpu_faulted() {
            return;
        }
        self.renderer
            .set_sculpt_brush(&SculptBrushUniform::hidden());
        self.renderer.set_sculpt_tool(&SculptToolUniform::hidden());
    }

    /// Take the most recent wgpu uncaptured error recorded by the device error
    /// handler, if any. The app polls this so a GPU fault surfaces as an honest
    /// message instead of wgpu's default panic (a hard abort in release).
    pub(super) fn take_gpu_error(&self) -> Option<String> {
        self.renderer.take_gpu_error()
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        if self.renderer.is_gpu_faulted() {
            return;
        }
        let Some(scene) = self.prepared_scene.as_ref() else {
            return;
        };
        scene.draw_with_clip(
            &self.renderer,
            render_pass,
            &self.camera_bind_group,
            &self.fallback_texture.bind_group,
            &self.clip_bind_group,
        );
        // Cut view: re-draw the cut-away side as a translucent ghost so the
        // cross-section fades geometry instead of deleting half the model.
        if self.clip_enabled && self.show_ghost {
            scene.draw_ghost_side(
                &self.renderer,
                render_pass,
                &self.camera_bind_group,
                &self.fallback_texture.bind_group,
                &self.clip_bind_group,
            );
        }
        if let Some(overlay) = self.selection_overlay.as_ref() {
            overlay.draw_with_clip(
                &self.renderer,
                render_pass,
                &self.camera_bind_group,
                &self.fallback_texture.bind_group,
                &self.clip_bind_group,
            );
        }
        if let Some(cursor) = self.sculpt_cursor {
            let drawn = scene.draw_sculpt_surface_feedback(
                render_pass,
                SculptSurfaceFeedbackRequest::new(
                    &self.renderer,
                    &self.camera_bind_group,
                    &self.clip_bind_group,
                    cursor.target_index,
                    &cursor.topology,
                ),
            );
            if drawn {
                self.renderer.draw_sculpt_tool(
                    render_pass,
                    &self.camera_bind_group,
                    &self.clip_bind_group,
                );
            }
        }
    }
}

pub(super) fn paint_callback(
    rect: egui::Rect,
    viewport: SharedLiveViewport,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, LiveViewportCallback { viewport })
}

struct LiveViewportCallback {
    viewport: SharedLiveViewport,
}

impl egui_wgpu::CallbackTrait for LiveViewportCallback {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Ok(viewport) = self.viewport.lock() else {
            return;
        };
        viewport.paint(render_pass);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_viewport_keeps_selection_overlay_separate_from_base_scene() {
        let source = crate::primary_ui_tests::production_source(include_str!("live_viewport.rs"))
            .replace("\r\n", "\n");
        let production_source = source
            .split_once("\nmod tests {")
            .map_or(source.as_str(), |(source, _)| source);

        assert!(
            production_source.contains("selection_overlay: Option<PreparedScene>"),
            "live viewport should not fold selected-face overlay into the base prepared scene"
        );
        assert!(
            production_source.contains("pub(super) fn sync_selection_overlay("),
            "selection overlay should have its own sync path"
        );
        assert!(
            crate::primary_ui_tests::appears_before(
                production_source,
                "scene.draw_with_clip(",
                "overlay.draw_with_clip(",
            ),
            "selection overlay should draw after the base scene"
        );
        assert!(
            production_source.contains("self.selection_overlay = None;"),
            "clearing the live scene should also clear stale selection overlay"
        );
    }
}

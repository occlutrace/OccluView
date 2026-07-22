//! Live `wgpu` viewport bridge for the desktop app.

use eframe::{egui, egui_wgpu, wgpu};
use occluview_render::{
    ClipPlane, GpuCamera, GpuTexture, PreparedScene, PreparedSceneSource, PreparedSceneUpdate,
    RenderError, Renderer,
};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::LIVE_VIEWPORT_SAMPLE_COUNT;

pub(super) type SharedLiveViewport = Arc<Mutex<LiveViewport>>;

pub(super) struct LiveViewport {
    renderer: Renderer,
    fallback_texture: GpuTexture,
    camera_bind_group: wgpu::BindGroup,
    clip_buffer: wgpu::Buffer,
    clip_bind_group: wgpu::BindGroup,
    /// Whether the current clip plane is active. Drives the extra ghost pass
    /// so it runs only while a cross-section is placed.
    clip_enabled: bool,
    prepared_scene: Option<PreparedScene>,
    selection_overlay: Option<PreparedScene>,
}

impl LiveViewport {
    pub(super) fn from_render_state(
        render_state: &egui_wgpu::RenderState,
    ) -> Result<SharedLiveViewport, RenderError> {
        let renderer = Renderer::with_shared_device_sample_count(
            render_state.device.clone(),
            render_state.queue.clone(),
            render_state.target_format,
            u32::from(LIVE_VIEWPORT_SAMPLE_COUNT),
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
            prepared_scene: None,
            selection_overlay: None,
        })))
    }

    pub(super) fn update_view(
        &mut self,
        camera: &GpuCamera,
        render_extent_px: [u16; 2],
        clip_plane: ClipPlane,
    ) {
        self.renderer.set_point_splat_viewport(
            u32::from(render_extent_px[0]),
            u32::from(render_extent_px[1]),
        );
        self.renderer.set_camera(camera);
        self.clip_enabled = clip_plane.enabled != 0;
        self.renderer
            .queue()
            .write_buffer(&self.clip_buffer, 0, bytemuck::bytes_of(&clip_plane));
    }

    pub(super) fn sync_scene(
        &mut self,
        sources: &[PreparedSceneSource<'_>],
        updates: &[PreparedSceneUpdate],
    ) {
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
    }

    /// Push only the `touched` sculpted vertices into the matching prepared
    /// entry — the hot per-dab path (see
    /// [`PreparedScene::write_entry_vertices_sparse`]).
    pub(super) fn write_scene_vertices_sparse(
        &self,
        topology: &occluview_render::PreparedSceneTopology,
        vertices: &[occluview_core::Vertex],
        touched: &[usize],
    ) -> bool {
        self.prepared_scene.as_ref().is_some_and(|scene| {
            scene.write_entry_vertices_sparse(&self.renderer, topology, vertices, touched)
        })
    }

    pub(super) fn write_scene_vertices(
        &self,
        topology: &occluview_render::PreparedSceneTopology,
        vertices: &[occluview_core::Vertex],
    ) -> bool {
        self.prepared_scene
            .as_ref()
            .is_some_and(|scene| scene.write_entry_vertices(&self.renderer, topology, vertices))
    }

    pub(super) fn sync_selection_overlay(&mut self, sources: &[PreparedSceneSource<'_>]) {
        self.selection_overlay =
            (!sources.is_empty()).then(|| PreparedScene::prepare(&self.renderer, sources));
    }

    pub(super) fn clear(&mut self) {
        self.prepared_scene = None;
        self.selection_overlay = None;
    }

    /// Take the most recent wgpu uncaptured error recorded by the device error
    /// handler, if any. The app polls this so a GPU fault surfaces as an honest
    /// message instead of wgpu's default panic (a hard abort in release).
    pub(super) fn take_gpu_error(&self) -> Option<String> {
        self.renderer.take_gpu_error()
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
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
        if self.clip_enabled {
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
        let source = include_str!("live_viewport.rs").replace("\r\n", "\n");
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
            production_source.find("scene.draw_with_clip(")
                < production_source.find("overlay.draw_with_clip("),
            "selection overlay should draw after the base scene"
        );
        assert!(
            production_source.contains("self.selection_overlay = None;"),
            "clearing the live scene should also clear stale selection overlay"
        );
    }
}

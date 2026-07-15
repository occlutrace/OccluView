use super::{
    helpers::is_transparent, PreparedScene, PreparedSceneEntry, PreparedSceneSource,
    PreparedSceneTopology, PreparedSceneUpdate, Renderer,
};
use crate::gpu::GpuMesh;
use crate::texture::GpuTexture;
use occluview_core::MeshKind;

impl PreparedScene {
    pub(super) fn upload(renderer: &Renderer, sources: &[PreparedSceneSource<'_>]) -> Self {
        let device = renderer.device();
        let queue = renderer.queue();
        let entries = sources
            .iter()
            .map(|source| {
                let topology = PreparedSceneTopology::from_mesh(source.mesh);
                let mesh =
                    GpuMesh::upload_with_wireframe(device, queue, source.mesh, source.wireframe);
                let uniform_buffer = renderer.mesh_uniform_buffer();
                queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&source.uniform));
                let mesh_bind_group = renderer.mesh_bind_group(&uniform_buffer);
                let texture = source
                    .mesh
                    .texture()
                    .map(|texture| GpuTexture::upload(renderer, device, queue, texture));
                PreparedSceneEntry {
                    mesh,
                    uniform_buffer,
                    mesh_bind_group,
                    texture,
                    kind: source.mesh.kind(),
                    topology,
                    opacity: source.uniform.opacity,
                    visible: source.visible,
                    wireframe: source.wireframe,
                }
            })
            .collect();
        Self { entries }
    }

    /// Upload a multi-mesh scene into GPU memory for repeated draws.
    #[must_use]
    pub fn prepare(renderer: &Renderer, sources: &[PreparedSceneSource<'_>]) -> Self {
        Self::upload(renderer, sources)
    }

    /// Update per-layer uniforms and visibility without re-uploading mesh buffers.
    ///
    /// Returns `false` if the caller's scene topology no longer matches this
    /// prepared scene and it should be rebuilt.
    pub fn update(&mut self, renderer: &Renderer, updates: &[PreparedSceneUpdate]) -> bool {
        if self.entries.len() != updates.len() {
            return false;
        }
        if self
            .entries
            .iter()
            .zip(updates)
            .any(|(entry, update)| entry.topology != update.topology)
        {
            return false;
        }
        if self
            .entries
            .iter()
            .zip(updates)
            .any(|(entry, update)| update.wireframe && !entry.mesh.has_wireframe_indices())
        {
            return false;
        }
        let queue = renderer.queue();
        for (entry, update) in self.entries.iter_mut().zip(updates) {
            queue.write_buffer(
                &entry.uniform_buffer,
                0,
                bytemuck::bytes_of(&update.uniform),
            );
            entry.opacity = update.uniform.opacity;
            entry.visible = update.visible;
            entry.wireframe = update.wireframe;
        }
        true
    }

    /// Number of GPU-resident layer entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this prepared scene contains no layers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Draw this GPU-resident scene into an existing render pass.
    pub fn draw(
        &self,
        renderer: &Renderer,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        fallback_texture_bg: &wgpu::BindGroup,
    ) {
        let clip_bg = renderer.disabled_clip_bind_group();
        self.draw_with_clip(renderer, rpass, camera_bg, fallback_texture_bg, clip_bg);
    }

    /// Draw this GPU-resident scene with an explicit clip-plane bind group.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_clip(
        &self,
        renderer: &Renderer,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        fallback_texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
    ) {
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.visible && !is_transparent(entry.opacity))
        {
            let tex_bg = entry
                .texture
                .as_ref()
                .map_or(fallback_texture_bg, |texture| &texture.bind_group);
            renderer.draw(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                tex_bg,
                clip_bg,
                &entry.mesh,
                entry.kind,
            );
        }
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.visible && is_transparent(entry.opacity))
        {
            let tex_bg = entry
                .texture
                .as_ref()
                .map_or(fallback_texture_bg, |texture| &texture.bind_group);
            renderer.draw_transparent(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                tex_bg,
                clip_bg,
                &entry.mesh,
                entry.kind,
            );
        }
        for entry in self.entries.iter().filter(|entry| {
            entry.visible && entry.wireframe && entry.kind == MeshKind::TriangleMesh
        }) {
            let tex_bg = entry
                .texture
                .as_ref()
                .map_or(fallback_texture_bg, |texture| &texture.bind_group);
            renderer.draw_wireframe(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                tex_bg,
                clip_bg,
                &entry.mesh,
            );
        }
    }

    /// Draw the cut-away side of every visible triangle mesh as a translucent
    /// ghost (OWNER cut-view rule: a cross-section fades geometry, never
    /// deletes it). Call this *after* [`Self::draw_with_clip`], which draws the
    /// kept side opaque and populates depth. `clip_bg` is the same clip-plane
    /// bind group as the opaque pass — `fs_ghost` inverts the test internally.
    ///
    /// Only solid triangle meshes are ghosted; point clouds and wireframe
    /// overlays are intentionally skipped on the cut-away side (a faint solid
    /// shell reads as "inactive"; ghosted points/edges would just add noise).
    /// A no-op when the clip plane is disabled (the shader draws nothing).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ghost_side(
        &self,
        renderer: &Renderer,
        rpass: &mut wgpu::RenderPass<'_>,
        camera_bg: &wgpu::BindGroup,
        fallback_texture_bg: &wgpu::BindGroup,
        clip_bg: &wgpu::BindGroup,
    ) {
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.visible && entry.kind == MeshKind::TriangleMesh)
        {
            let tex_bg = entry
                .texture
                .as_ref()
                .map_or(fallback_texture_bg, |texture| &texture.bind_group);
            renderer.draw_ghost(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                tex_bg,
                clip_bg,
                &entry.mesh,
            );
        }
    }
}

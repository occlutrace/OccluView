use super::{
    helpers::is_transparent, ContactPaintSource, EntryContact, PreparedScene, PreparedSceneEntry,
    PreparedSceneSource, PreparedSceneTopology, PreparedSceneUpdate,
};
use crate::contact_texture::GpuContactMaterial;
use crate::gpu::GpuMesh;
use crate::pipeline::{Renderer, SculptSurfaceFeedbackBindings};
use crate::texture::GpuTexture;
use occluview_core::{MeshKind, Vertex};

/// Inputs for the display-only Sculpt surface feedback pass. The request
/// keeps the target layer identity beside the GPU bindings so a stale cursor
/// cannot accidentally light a neighbouring mesh.
pub struct SculptSurfaceFeedbackRequest<'a> {
    renderer: &'a Renderer,
    camera_bg: &'a wgpu::BindGroup,
    clip_bg: &'a wgpu::BindGroup,
    target_index: usize,
    topology: &'a PreparedSceneTopology,
}

impl<'a> SculptSurfaceFeedbackRequest<'a> {
    /// Build a feedback request without copying any GPU state.
    #[must_use]
    pub fn new(
        renderer: &'a Renderer,
        camera_bg: &'a wgpu::BindGroup,
        clip_bg: &'a wgpu::BindGroup,
        target_index: usize,
        topology: &'a PreparedSceneTopology,
    ) -> Self {
        Self {
            renderer,
            camera_bg,
            clip_bg,
            target_index,
            topology,
        }
    }
}

/// Above this many touched vertices, a sparse (per-run) vertex update switches
/// to a single whole-buffer write — the scattered soup runs would otherwise
/// cost more in per-call overhead than one big copy.
const SPARSE_WRITE_MAX_TOUCHED: usize = 8192;

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
                let contact = source.contact.as_ref().map(|source| EntryContact {
                    // The layer's own texture is bound with the field, so the
                    // paint sits on the scan rather than replacing it.
                    material: GpuContactMaterial::upload(
                        renderer,
                        device,
                        queue,
                        source.field(),
                        texture.as_ref(),
                    ),
                    revision: source.revision(),
                });
                PreparedSceneEntry {
                    mesh,
                    uniform_buffer,
                    mesh_bind_group,
                    texture,
                    contact,
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

    /// Update per-layer uniforms, visibility and the drawn contact field
    /// without re-uploading mesh buffers.
    ///
    /// A uniform write is the whole cost of moving the one control a contact
    /// reading has (the depth that reads as fully loaded), and the field is
    /// re-uploaded only when its revision token changes — never merely because
    /// a frame went by.
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
        let device = renderer.device();
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
            sync_entry_contact(entry, renderer, device, queue, update);
        }
        true
    }

    /// Overwrite the vertex-buffer content of the entry whose uploaded
    /// topology matches `topology` with fresh CPU vertices — the live path an
    /// interactive sculpt stroke uses to show each brush frame without
    /// re-preparing the whole scene. The uploaded topology identity is
    /// untouched (indices, counts, and texture stay as-is), so subsequent
    /// uniform-only [`Self::update`] reconciles keep succeeding mid-drag.
    /// Returns `false` when no entry matches or the vertex count differs; the
    /// caller then falls back to a full re-prepare.
    pub fn write_entry_vertices(
        &self,
        renderer: &Renderer,
        topology: &PreparedSceneTopology,
        vertices: &[Vertex],
    ) -> bool {
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.topology == *topology)
        else {
            return false;
        };
        if u32::try_from(vertices.len()) != Ok(entry.mesh.vertex_count) {
            return false;
        }
        renderer
            .queue()
            .write_buffer(&entry.mesh.vertex_buffer, 0, bytemuck::cast_slice(vertices));
        true
    }

    /// Overwrite only the vertices at `touched` indices (ascending, in range)
    /// of the matching entry's vertex buffer — the hot path for an interactive
    /// sculpt drag, where a brush touches a few hundred vertices of a
    /// multi-hundred-thousand-vertex scan. Writing the whole buffer every
    /// frame (see [`Self::write_entry_vertices`]) would move megabytes per dab
    /// and stutter; this writes only the affected vertices, coalesced into
    /// contiguous runs so scattered soup duplicates still cost few GPU writes.
    /// Returns `false` when no entry matches, the vertex count differs, or the
    /// touched ids do not describe a sorted in-range slice.
    pub fn write_entry_vertices_sparse(
        &self,
        renderer: &Renderer,
        topology: &PreparedSceneTopology,
        vertices: &[Vertex],
        touched: &[usize],
    ) -> bool {
        // The run-coalescing below needs `touched` strictly ascending and
        // in-range. Validate in release builds as well: skipping a
        // bad id would report a successful upload while leaving part of the
        // GPU shadow stale, and an unsorted id could make a range slice panic.
        if !touched.windows(2).all(|pair| pair[0] < pair[1])
            || touched.iter().any(|&id| id >= vertices.len())
        {
            return false;
        }
        debug_assert!(
            touched.windows(2).all(|pair| pair[0] < pair[1]),
            "write_entry_vertices_sparse requires strictly ascending touched ids"
        );
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.topology == *topology)
        else {
            return false;
        };
        if u32::try_from(vertices.len()) != Ok(entry.mesh.vertex_count) {
            return false;
        }
        let queue = renderer.queue();
        // A big brush touches array-scattered soup vertices, which coalesce into
        // many short runs — thousands of tiny `write_buffer` calls whose per-call
        // overhead would stutter. Past a threshold, one whole-buffer write is
        // cheaper than the pile of small ones.
        if touched.len() > SPARSE_WRITE_MAX_TOUCHED {
            queue.write_buffer(&entry.mesh.vertex_buffer, 0, bytemuck::cast_slice(vertices));
            return true;
        }
        let stride = size_of::<Vertex>() as u64;
        let mut run_start: Option<usize> = None;
        let mut prev = usize::MAX;
        // `touched` is ascending; flush a run whenever the ids stop being
        // consecutive so each `write_buffer` covers one contiguous span.
        for &id in touched {
            match run_start {
                Some(_) if id == prev + 1 => {}
                Some(start) => {
                    queue.write_buffer(
                        &entry.mesh.vertex_buffer,
                        start as u64 * stride,
                        bytemuck::cast_slice(&vertices[start..=prev]),
                    );
                    run_start = Some(id);
                }
                None => run_start = Some(id),
            }
            prev = id;
        }
        if let Some(start) = run_start {
            queue.write_buffer(
                &entry.mesh.vertex_buffer,
                start as u64 * stride,
                bytemuck::cast_slice(&vertices[start..=prev]),
            );
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
}

/// Re-upload an entry's contact field, or drop it, when the update says so.
///
/// The revision is the only trigger: an update that carries the same field
/// revision as the one already bound leaves the GPU copy alone, so a caller
/// that re-derives an identical packed field every frame pays nothing for it.
fn sync_entry_contact(
    entry: &mut PreparedSceneEntry,
    renderer: &Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    update: &PreparedSceneUpdate,
) {
    let wanted = update.contact.as_ref().map(ContactPaintSource::revision);
    let bound = entry.contact.as_ref().map(|contact| contact.revision);
    if wanted == bound {
        return;
    }
    entry.contact = update.contact.as_ref().map(|source| EntryContact {
        // The layer's own texture is bound with the field, so the paint sits on
        // the scan rather than replacing it.
        material: GpuContactMaterial::upload(
            renderer,
            device,
            queue,
            source.field(),
            entry.texture.as_ref(),
        ),
        revision: source.revision(),
    });
}

impl PreparedSceneEntry {
    /// The group-2 bind group this entry draws with: a bound contact field
    /// wins, then the scan's own material texture, then the shared fallback.
    ///
    /// One method because four draw paths (opaque, transparent, wireframe,
    /// ghost) need it, and a contact field that reached only three of them
    /// would read as a map that flickers with the layer's opacity.
    fn group2<'a>(&'a self, fallback: &'a wgpu::BindGroup) -> &'a wgpu::BindGroup {
        if let Some(contact) = self.contact.as_ref() {
            return contact.material.bind_group();
        }
        self.texture
            .as_ref()
            .map_or(fallback, |texture| &texture.bind_group)
    }
}

impl PreparedScene {
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
            renderer.draw(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                entry.group2(fallback_texture_bg),
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
            renderer.draw_transparent(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                entry.group2(fallback_texture_bg),
                clip_bg,
                &entry.mesh,
                entry.kind,
            );
        }
        for entry in self.entries.iter().filter(|entry| {
            entry.visible && entry.wireframe && entry.kind == MeshKind::TriangleMesh
        }) {
            renderer.draw_wireframe(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                entry.group2(fallback_texture_bg),
                clip_bg,
                &entry.mesh,
            );
        }
    }

    /// Draw the display-only Sculpt surface field for one stable prepared
    /// entry. Both the scene index and topology token are checked so a cursor
    /// from a replaced layer cannot light a neighbouring mesh for one frame.
    pub fn draw_sculpt_surface_feedback(
        &self,
        rpass: &mut wgpu::RenderPass<'_>,
        request: SculptSurfaceFeedbackRequest<'_>,
    ) -> bool {
        let Some(entry) = self.entries.get(request.target_index) else {
            return false;
        };
        if !entry.visible
            || entry.kind != MeshKind::TriangleMesh
            || entry.topology != *request.topology
        {
            return false;
        }
        request.renderer.draw_sculpt_surface_feedback(
            rpass,
            SculptSurfaceFeedbackBindings {
                camera_bg: request.camera_bg,
                mesh_bg: &entry.mesh_bind_group,
                clip_bg: request.clip_bg,
                mesh: &entry.mesh,
            },
        );
        true
    }

    /// Draw the cut-away side of every visible triangle mesh as a translucent
    /// ghost (cut-view invariant: a cross-section fades geometry, never
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
            renderer.draw_ghost(
                rpass,
                camera_bg,
                &entry.mesh_bind_group,
                entry.group2(fallback_texture_bg),
                clip_bg,
                &entry.mesh,
            );
        }
    }
}

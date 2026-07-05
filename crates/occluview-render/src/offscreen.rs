//! Offscreen render-to-texture: used by the thumbnail worker and golden-image
//! tests. One render target + depth, one draw, read back as RGBA8.

use crate::camera::GpuCamera;
use crate::clipping::ClipPlane;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use crate::mesh_uniform::GpuMeshUniform;
use crate::pipeline::Renderer;
use crate::texture::GpuTexture;
use occluview_core::Mesh;

/// One entry in a multi-mesh offscreen scene draw: the mesh, its per-mesh
/// uniform, and an optional texture.
pub struct SceneDrawEntry<'a> {
    /// The CPU mesh to upload + draw.
    pub mesh: &'a Mesh,
    /// Per-mesh uniform (model, tint, opacity, `has_texture` flag).
    pub uniform: &'a GpuMeshUniform,
    /// Texture to sample; if `None`, the fallback 1×1 white texture is used.
    pub texture: Option<&'a GpuTexture>,
}

/// Parameters for an offscreen render.
#[derive(Clone, Copy, Debug)]
pub struct ThumbnailSpec {
    /// Square output dimension in pixels.
    pub size_px: u16,
    /// Background color (linear RGBA). Default is transparent.
    pub background: [f64; 4],
}

impl Default for ThumbnailSpec {
    fn default() -> Self {
        Self {
            size_px: 256,
            background: [0.039, 0.039, 0.039, 1.0], // OccluTrace dark, opaque
        }
    }
}

/// Offscreen renderer. Wraps a headless [`Renderer`].
pub struct Offscreen {
    renderer: Renderer,
    /// Cached identity mesh uniform + bind group (group 1). The thumbnail path
    /// renders one mesh at the origin, so the model matrix is identity.
    mesh_uniform_buffer: wgpu::Buffer,
    mesh_bind_group: wgpu::BindGroup,
    /// Cached 1×1 white fallback texture + bind group (group 2). The thumbnail
    /// path uses vertex colors (no texture), but the pipeline requires a bound
    /// group-2 resource.
    texture_bind_group: wgpu::BindGroup,
}

impl Offscreen {
    /// Create a headless renderer at any reasonable output format.
    ///
    /// # Errors
    /// Returns [`RenderError::NoAdapter`] if no GPU/adapter is available
    /// (including under WARP-less sandboxes).
    #[allow(clippy::unused_async)]
    pub async fn new() -> Result<Self, RenderError> {
        let renderer = Renderer::new_headless(wgpu::TextureFormat::Rgba8Unorm).await?;
        let device = renderer.device();
        let queue = renderer.queue();

        // Identity mesh uniform (group 1).
        let mesh_uniform_buffer = renderer.mesh_uniform_buffer();
        queue.write_buffer(
            &mesh_uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuMeshUniform::identity()),
        );
        let mesh_bind_group = renderer.mesh_bind_group(&mesh_uniform_buffer);

        // 1×1 white fallback texture + sampler (group 2).
        let texture_bind_group = make_fallback_texture_bind_group(device, queue, &renderer);

        Ok(Self {
            renderer,
            mesh_uniform_buffer,
            mesh_bind_group,
            texture_bind_group,
        })
    }

    /// Render `mesh` with the given camera into an RGBA8 buffer.
    ///
    /// Returns a flat `Vec<u8>` of length `size_px * size_px * 4` in row-major
    /// order, top-to-bottom (after the y-flip wgpu requires for offscreen).
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async)]
    pub async fn render(
        &self,
        mesh: &Mesh,
        camera: &GpuCamera,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        let gpu_mesh = GpuMesh::upload(device, queue, mesh);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview offscreen encoder"),
        });
        let targets = RenderTargets {
            color: &color_view,
            depth: &depth_view,
        };
        self.encode_pass(
            &mut encoder,
            &targets,
            &camera_bg,
            &self.mesh_bind_group,
            &self.texture_bind_group,
            &gpu_mesh,
            mesh.kind(),
            spec.background,
        );

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview offscreen readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            extent(size),
        );
        queue.submit(std::iter::once(encoder.finish()));

        Ok(self.read_back(&output_buffer, padded, spec.size_px))
    }

    /// Access the underlying renderer (for callers that need device/queue).
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Render `mesh` with an active clip plane (Approach A — fragment discard
    /// only, no stencil cap). This is the "hollow cut" preview: fragments
    /// below the plane are discarded, leaving the cut surface open.
    ///
    /// Used by the cut-view widget and the `cut_triangle_discard` golden test.
    /// The full 3-pass stencil-capped render lands in a follow-up
    /// (`render_with_cut`).
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async)]
    pub async fn render_clipped(
        &self,
        mesh: &Mesh,
        camera: &GpuCamera,
        clip: &ClipPlane,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        let gpu_mesh = GpuMesh::upload(device, queue, mesh);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        // Active clip plane uniform + bind group.
        let clip_buf = self.renderer.clip_uniform_buffer();
        queue.write_buffer(&clip_buf, 0, bytemuck::bytes_of(clip));
        let clip_bg = self.renderer.clip_bind_group(&clip_buf);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview clipped encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview clipped pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: spec.background[0],
                        g: spec.background[1],
                        b: spec.background[2],
                        a: spec.background[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.renderer.draw(
            &mut rpass,
            &camera_bg,
            &self.mesh_bind_group,
            &self.texture_bind_group,
            &clip_bg,
            &gpu_mesh,
            mesh.kind(),
        );
        drop(rpass);

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview clipped readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            extent(size),
        );
        queue.submit(std::iter::once(encoder.finish()));

        Ok(self.read_back(&output_buffer, padded, spec.size_px))
    }

    /// Render `mesh` with a **solid** cross-section cut (Approach B,
    /// stencil-capped, ADR-0011). The visible mesh is clipped by the plane
    /// (fragments below discarded), and the cut surface is filled with a cap
    /// polygon whose color is `cut.cap_color`.
    ///
    /// When `cut.show_hollow` is `true`, the stencil cap passes are skipped —
    /// only the shaded clip (Approach A) runs, leaving the cut surface open.
    ///
    /// `half_extent` is the half-size of the cap quad (oversized; the stencil
    /// test clips it). Pass `mesh.bbox_uncached().half_diagonal() * 2.0`.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async, clippy::too_many_lines, clippy::too_many_arguments)]
    pub async fn render_with_cut(
        &self,
        mesh: &Mesh,
        camera: &GpuCamera,
        cut: &crate::clipping::CutViewSpec,
        half_extent: f32,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        use crate::clipping::cap_quad;

        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        let gpu_mesh = GpuMesh::upload(device, queue, mesh);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let clip_buf = self.renderer.clip_uniform_buffer();
        queue.write_buffer(&clip_buf, 0, bytemuck::bytes_of(&cut.plane));
        let clip_bg = self.renderer.clip_bind_group(&clip_buf);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview cut encoder"),
        });

        if !cut.show_hollow {
            // Passes 1 & 2: stencil increment/decrement (no color write).
            for (i, pipe) in [
                &self.renderer.stencil_back_pipeline,
                &self.renderer.stencil_front_pipeline,
            ]
            .iter()
            .enumerate()
            {
                let color_load = if i == 0 {
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: spec.background[0],
                        g: spec.background[1],
                        b: spec.background[2],
                        a: spec.background[3],
                    })
                } else {
                    wgpu::LoadOp::Load
                };
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("occluview stencil pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: color_load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0),
                            store: wgpu::StoreOp::Store,
                        }),
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                rpass.set_pipeline(pipe);
                rpass.set_bind_group(0, &camera_bg, &[]);
                rpass.set_bind_group(1, &self.mesh_bind_group, &[]);
                rpass.set_bind_group(2, &self.texture_bind_group, &[]);
                rpass.set_bind_group(3, &clip_bg, &[]);
                gpu_mesh.draw(&mut rpass, mesh.kind());
                drop(rpass);
            }

            // Pass 3: cap polygon (stencil test NotEqual 0).
            let (cap_verts, cap_indices) = cap_quad(&cut.plane, half_extent);
            let cap_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occluview cap vertex buffer"),
                size: (cap_verts.len() * 12) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&cap_vbuf, 0, bytemuck::cast_slice(&cap_verts));
            let cap_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occluview cap index buffer"),
                size: (cap_indices.len() * 4) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&cap_index_buffer, 0, bytemuck::cast_slice(&cap_indices));
            let cap_color_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occluview cap color uniform"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&cap_color_buffer, 0, bytemuck::cast_slice(&cut.cap_color));
            let cap_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("occluview cap bind group"),
                layout: &self.renderer.cap_uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cap_color_buffer.as_entire_binding(),
                }],
            });
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("occluview cap pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.renderer.cap_pipeline);
            rpass.set_bind_group(0, &camera_bg, &[]);
            rpass.set_bind_group(1, &cap_bg, &[]);
            rpass.set_vertex_buffer(0, cap_vbuf.slice(..));
            rpass.set_index_buffer(cap_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            rpass.draw_indexed(0..cap_indices.len() as u32, 0, 0..1);
            drop(rpass);
        }

        // Pass 4: shaded mesh with clip plane enabled.
        {
            let color_load = if cut.show_hollow {
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: spec.background[0],
                    g: spec.background[1],
                    b: spec.background[2],
                    a: spec.background[3],
                })
            } else {
                wgpu::LoadOp::Load
            };
            // The shaded pass needs a fresh depth buffer (the stencil passes
            // wrote depth that would fail Less comparison). Always clear depth.
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("occluview cut shaded pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: color_load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.renderer.draw(
                &mut rpass,
                &camera_bg,
                &self.mesh_bind_group,
                &self.texture_bind_group,
                &clip_bg,
                &gpu_mesh,
                mesh.kind(),
            );
        }

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview cut readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            extent(size),
        );
        queue.submit(std::iter::once(encoder.finish()));

        Ok(self.read_back(&output_buffer, padded, spec.size_px))
    }

    /// Access the cached fallback texture bind group (group 2). Useful for
    /// multi-mesh draws where some meshes are untextured.
    pub fn fallback_texture_bind_group(&self) -> &wgpu::BindGroup {
        &self.texture_bind_group
    }

    /// Access the cached identity mesh uniform buffer. Useful for building
    /// additional bind groups.
    pub fn identity_uniform_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_uniform_buffer
    }

    /// The per-mesh uniform bind group layout (group 1). Exposed so callers
    /// can build per-mesh bind groups for multi-mesh scenes.
    pub fn mesh_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.renderer.mesh_bind_group_layout()
    }

    /// Render a **multi-mesh scene** offscreen: uploads each mesh, sets its
    /// per-mesh uniform (transform + tint + opacity + `has_texture`), binds its
    /// texture (or the fallback), and draws all meshes in one render pass.
    ///
    /// `entries` is a slice of `(GpuMesh, GpuMeshUniform, Option<&GpuTexture>,
    /// MeshKind)`. Each entry becomes one draw call within the pass.
    ///
    /// Returns RGBA8 pixels, length `size_px * size_px * 4`, top-to-bottom.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async)]
    pub async fn render_scene(
        &self,
        entries: &[SceneDrawEntry<'_>],
        camera: &GpuCamera,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        // Upload each mesh + its per-mesh uniform buffer. Keep the per-mesh
        // buffers/bind-groups alive; textures are borrowed (group 2). We
        // collect texture bind-group references up front so the borrow lives
        // across the pass.
        let mut uploaded: Vec<(
            GpuMesh,
            wgpu::Buffer,
            wgpu::BindGroup,
            occluview_core::MeshKind,
        )> = Vec::with_capacity(entries.len());
        // Texture bind group per entry: either a borrow of an entry's
        // GpuTexture.bind_group, or the fallback. Stored as Option; None =
        // fallback.
        let mut tex_bgs: Vec<Option<&wgpu::BindGroup>> = Vec::with_capacity(entries.len());
        for entry in entries {
            let gpu_mesh = GpuMesh::upload(device, queue, entry.mesh);
            let uniform_buf = self.renderer.mesh_uniform_buffer();
            queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(entry.uniform));
            let mesh_bg = self.renderer.mesh_bind_group(&uniform_buf);
            tex_bgs.push(entry.texture.map(|t| &t.bind_group));
            uploaded.push((gpu_mesh, uniform_buf, mesh_bg, entry.mesh.kind()));
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview offscene encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview offscene pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: spec.background[0],
                        g: spec.background[1],
                        b: spec.background[2],
                        a: spec.background[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let clip_bg = self.renderer.disabled_clip_bind_group();
        for (i, (gpu_mesh, _, mesh_bg, kind)) in uploaded.iter().enumerate() {
            let tex_bg = tex_bgs[i].unwrap_or(&self.texture_bind_group);
            self.renderer.draw(
                &mut rpass, &camera_bg, mesh_bg, tex_bg, clip_bg, gpu_mesh, *kind,
            );
        }
        drop(rpass);

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview offscene readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            extent(size),
        );
        queue.submit(std::iter::once(encoder.finish()));

        Ok(self.read_back(&output_buffer, padded, spec.size_px))
    }

    /// Begin the offscreen render pass against `targets` and draw `mesh`.
    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: occluview_core::MeshKind,
        background: [f64; 4],
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview offscreen pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: targets.color,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background[0],
                        g: background[1],
                        b: background[2],
                        a: background[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                // Clear stencil to 0 so the disabled-clip draw is a no-op on
                // the stencil plane (cut-view passes override this).
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.renderer.draw(
            &mut rpass,
            camera_bg,
            mesh_bg,
            texture_bg,
            self.renderer.disabled_clip_bind_group(),
            mesh,
            kind,
        );
    }

    fn read_back(
        &self,
        output_buffer: &wgpu::Buffer,
        padded_bytes_per_row: u32,
        size_px: u16,
    ) -> Vec<u8> {
        let slice = output_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.renderer.device().poll(wgpu::Maintain::Wait);

        let row_bytes = usize::from(size_px) * 4;
        let pixels = {
            let data = slice.get_mapped_range();
            let mut out = Vec::with_capacity(row_bytes * usize::from(size_px));
            for row in 0..usize::from(size_px) {
                let start = row * padded_bytes_per_row as usize;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
            out
        };
        output_buffer.unmap();

        // wgpu renders bottom-to-top; flip to top-to-bottom for consumers
        // (PNG encoders, HBITMAP interop).
        let mut flipped = Vec::with_capacity(pixels.len());
        for row in (0..usize::from(size_px)).rev() {
            flipped.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
        }
        flipped
    }
}

/// Color + depth views grouped so `encode_pass` takes one argument.
struct RenderTargets<'a> {
    color: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
}

/// Build a 1×1 white `Rgba8Unorm` texture + linear sampler + bind group for
/// group 2. Used as the bound-texture fallback for untextured meshes (the
/// shader's `has_texture=0` branch never samples it, but the pipeline layout
/// requires the binding).
fn make_fallback_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Renderer,
) -> wgpu::BindGroup {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview fallback white texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255, 255, 255, 255],
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("occluview fallback sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("occluview fallback texture bind group"),
        layout: renderer.texture_layout(),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&tex_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

fn make_color_target(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen color"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn make_depth_target(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluview offscreen depth"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn extent(size: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    }
}

/// wgpu requires buffer rows to be aligned to 256 bytes. RGBA8 = 4 bytes/pixel.
fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    unpadded.div_ceil(256) * 256
}

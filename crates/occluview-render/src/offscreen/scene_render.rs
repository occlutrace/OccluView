use super::{
    helpers::{
        extent, extent_rect, make_color_target, make_color_target_extent, make_depth_target,
        make_depth_target_extent, padded_bytes_per_row, RenderTargets,
    },
    Offscreen, PreparedScene, SceneDrawEntry, ThumbnailSpec, ViewportSpec,
};
use crate::camera::GpuCamera;
use crate::clipping::ClipPlane;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use occluview_core::MeshKind;

impl Offscreen {
    /// Render a multi-mesh scene offscreen.
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

        self.renderer.set_point_splat_viewport(size, size);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let mut uploaded: Vec<(GpuMesh, wgpu::Buffer, wgpu::BindGroup, MeshKind)> =
            Vec::with_capacity(entries.len());
        let mut tex_bgs: Vec<Option<&wgpu::BindGroup>> = Vec::with_capacity(entries.len());
        for entry in entries {
            let gpu_mesh = GpuMesh::upload(device, queue, entry.mesh);
            let uniform_buf = self.renderer.mesh_uniform_buffer();
            queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(entry.uniform));
            let mesh_bg = self.renderer.mesh_bind_group(&uniform_buf);
            tex_bgs.push(entry.texture.map(|texture| &texture.bind_group));
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
        for (index, (gpu_mesh, _, mesh_bg, kind)) in uploaded.iter().enumerate() {
            let tex_bg = tex_bgs[index].unwrap_or(&self.texture_bind_group);
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

        self.read_back(&output_buffer, padded, spec.size_px)
    }

    /// Render an already-uploaded multi-mesh scene offscreen.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::too_many_lines, clippy::unused_async)]
    pub async fn render_prepared_scene(
        &self,
        scene: &PreparedScene,
        camera: &GpuCamera,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        self.renderer.set_point_splat_viewport(size, size);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview prepared scene encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview prepared scene pass"),
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
        scene.draw(
            &self.renderer,
            &mut rpass,
            &camera_bg,
            &self.texture_bind_group,
        );
        drop(rpass);

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview prepared scene readback"),
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

        self.read_back(&output_buffer, padded, spec.size_px)
    }

    /// Render an already-uploaded scene with a clipping plane.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::too_many_lines, clippy::unused_async)]
    pub async fn render_prepared_scene_with_clip(
        &self,
        scene: &PreparedScene,
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

        self.renderer.set_point_splat_viewport(size, size);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let clip_buf = self.renderer.clip_uniform_buffer();
        queue.write_buffer(&clip_buf, 0, bytemuck::bytes_of(clip));
        let clip_bg = self.renderer.clip_bind_group(&clip_buf);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview prepared clipped scene encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview prepared clipped scene pass"),
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
        scene.draw_with_clip(
            &self.renderer,
            &mut rpass,
            &camera_bg,
            &self.texture_bind_group,
            &clip_bg,
        );
        drop(rpass);

        let padded = padded_bytes_per_row(size);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview prepared clipped scene readback"),
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

        self.read_back(&output_buffer, padded, spec.size_px)
    }

    /// Render an already-uploaded scene into a rectangular app viewport.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::too_many_lines, clippy::unused_async)]
    pub async fn render_prepared_viewport(
        &self,
        scene: &PreparedScene,
        camera: &GpuCamera,
        spec: ViewportSpec,
    ) -> Result<Vec<u8>, RenderError> {
        self.render_prepared_viewport_with_overlay(scene, None, camera, spec)
            .await
    }

    /// Render an already-uploaded scene and optional overlay into a
    /// rectangular app viewport.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::too_many_lines, clippy::unused_async)]
    pub async fn render_prepared_viewport_with_overlay(
        &self,
        scene: &PreparedScene,
        overlay: Option<&PreparedScene>,
        camera: &GpuCamera,
        spec: ViewportSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let [width_px, height_px] = spec.size_px;
        let width = u32::from(width_px);
        let height = u32::from(height_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target_extent(device, width, height);
        let (_depth_texture, depth_view) =
            make_depth_target_extent(device, width, height, self.renderer.depth_format());

        self.renderer.set_point_splat_viewport(width, height);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview prepared viewport encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview prepared viewport pass"),
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
        scene.draw(
            &self.renderer,
            &mut rpass,
            &camera_bg,
            &self.texture_bind_group,
        );
        if let Some(overlay) = overlay {
            overlay.draw(
                &self.renderer,
                &mut rpass,
                &camera_bg,
                &self.texture_bind_group,
            );
        }
        drop(rpass);

        let padded = padded_bytes_per_row(width);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview prepared viewport readback"),
            size: u64::from(padded) * u64::from(height),
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
                    rows_per_image: Some(height),
                },
            },
            extent_rect(width, height),
        );
        queue.submit(std::iter::once(encoder.finish()));

        self.read_back_extent(&output_buffer, padded, spec.size_px)
    }

    /// Render an already-uploaded scene into a rectangular app viewport with
    /// an active clipping plane.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::too_many_lines, clippy::unused_async)]
    pub async fn render_prepared_viewport_with_clip(
        &self,
        scene: &PreparedScene,
        camera: &GpuCamera,
        clip: &ClipPlane,
        spec: ViewportSpec,
    ) -> Result<Vec<u8>, RenderError> {
        self.render_prepared_viewport_with_clip_and_overlay(scene, None, camera, clip, spec)
            .await
    }

    /// Render an already-uploaded scene and optional overlay into a
    /// rectangular app viewport with an active clipping plane.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        clippy::unused_async
    )]
    pub async fn render_prepared_viewport_with_clip_and_overlay(
        &self,
        scene: &PreparedScene,
        overlay: Option<&PreparedScene>,
        camera: &GpuCamera,
        clip: &ClipPlane,
        spec: ViewportSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let [width_px, height_px] = spec.size_px;
        let width = u32::from(width_px);
        let height = u32::from(height_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target_extent(device, width, height);
        let (_depth_texture, depth_view) =
            make_depth_target_extent(device, width, height, self.renderer.depth_format());

        self.renderer.set_point_splat_viewport(width, height);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let clip_buf = self.renderer.clip_uniform_buffer();
        queue.write_buffer(&clip_buf, 0, bytemuck::bytes_of(clip));
        let clip_bg = self.renderer.clip_bind_group(&clip_buf);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview prepared clipped viewport encoder"),
        });
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("occluview prepared clipped viewport pass"),
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
        scene.draw_with_clip(
            &self.renderer,
            &mut rpass,
            &camera_bg,
            &self.texture_bind_group,
            &clip_bg,
        );
        if let Some(overlay) = overlay {
            overlay.draw_with_clip(
                &self.renderer,
                &mut rpass,
                &camera_bg,
                &self.texture_bind_group,
                &clip_bg,
            );
        }
        // Main-viewport cut view: fade the cut-away side instead of deleting it.
        // (The small slice preview, `render_prepared_scene_with_clip`, keeps its
        // hard clip and never calls this.)
        scene.draw_ghost_side(
            &self.renderer,
            &mut rpass,
            &camera_bg,
            &self.texture_bind_group,
            &clip_bg,
        );
        drop(rpass);

        let padded = padded_bytes_per_row(width);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview prepared clipped viewport readback"),
            size: u64::from(padded) * u64::from(height),
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
                    rows_per_image: Some(height),
                },
            },
            extent_rect(width, height),
        );
        queue.submit(std::iter::once(encoder.finish()));

        self.read_back_extent(&output_buffer, padded, spec.size_px)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &RenderTargets<'_>,
        camera_bg: &wgpu::BindGroup,
        mesh_bg: &wgpu::BindGroup,
        texture_bg: &wgpu::BindGroup,
        mesh: &GpuMesh,
        kind: MeshKind,
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
}

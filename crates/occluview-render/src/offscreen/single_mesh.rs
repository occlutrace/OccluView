use super::{
    helpers::{extent, make_color_target, make_depth_target, padded_bytes_per_row, RenderTargets},
    Offscreen, ThumbnailSpec,
};
use crate::camera::GpuCamera;
use crate::clipping::{cap_quad, ClipPlane, CutViewSpec};
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use occluview_core::Mesh;

impl Offscreen {
    /// Render `mesh` with the given camera into an RGBA8 buffer.
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
        self.renderer.set_point_splat_viewport(size, size);
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

        self.read_back(&output_buffer, padded, spec.size_px)
    }

    /// Render `mesh` with an active clip plane.
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
        self.renderer.set_point_splat_viewport(size, size);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

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

        self.read_back(&output_buffer, padded, spec.size_px)
    }

    /// Convenience: render a cut view with an auto-framed orthographic camera.
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(clippy::unused_async)]
    pub async fn render_cut_view(
        &self,
        mesh: &Mesh,
        cut: &CutViewSpec,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let bbox = mesh.bbox_uncached();
        let camera = crate::cut_camera::cut_view_camera(&cut.plane, bbox);
        let half_extent = bbox.half_diagonal() * 2.0;
        self.render_with_cut(mesh, &camera, cut, half_extent, spec)
            .await
    }

    /// Render `mesh` with a solid cross-section cut (stencil-capped).
    ///
    /// # Errors
    /// - [`RenderError::Surface`] on device loss or buffer-map failure.
    #[allow(
        clippy::unused_async,
        clippy::too_many_arguments,
        clippy::too_many_lines
    )]
    pub async fn render_with_cut(
        &self,
        mesh: &Mesh,
        camera: &GpuCamera,
        cut: &CutViewSpec,
        half_extent: f32,
        spec: ThumbnailSpec,
    ) -> Result<Vec<u8>, RenderError> {
        let size = u32::from(spec.size_px);
        let device = self.renderer.device();
        let queue = self.renderer.queue();

        let (color_texture, color_view) = make_color_target(device, size);
        let (_depth_texture, depth_view) =
            make_depth_target(device, size, self.renderer.depth_format());

        let gpu_mesh = GpuMesh::upload(device, queue, mesh);
        self.renderer.set_point_splat_viewport(size, size);
        self.renderer.set_camera(camera);
        let camera_bg = self.renderer.camera_bind_group();

        let clip_buf = self.renderer.clip_uniform_buffer();
        queue.write_buffer(&clip_buf, 0, bytemuck::bytes_of(&cut.plane));
        let clip_bg = self.renderer.clip_bind_group(&clip_buf);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("occluview cut encoder"),
        });

        if !cut.show_hollow {
            for (index, pipeline) in [
                &self.renderer.stencil_back_pipeline,
                &self.renderer.stencil_front_pipeline,
            ]
            .iter()
            .enumerate()
            {
                let color_load = if index == 0 {
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
                rpass.set_pipeline(pipeline);
                rpass.set_bind_group(0, &camera_bg, &[]);
                rpass.set_bind_group(1, &self.mesh_bind_group, &[]);
                rpass.set_bind_group(2, &self.texture_bind_group, &[]);
                rpass.set_bind_group(3, &clip_bg, &[]);
                gpu_mesh.draw(&mut rpass, mesh.kind());
                drop(rpass);
            }

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

        self.read_back(&output_buffer, padded, spec.size_px)
    }
}

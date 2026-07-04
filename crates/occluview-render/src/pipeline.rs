//! Render pipeline: device + shader + pipeline + per-frame camera uniform.
//!
//! [`Renderer`] owns the long-lived GPU state (bind group layout, render
//! pipeline, camera uniform buffer). Per-frame you call [`Renderer::draw`]
//! inside a render pass.

use crate::camera::GpuCamera;
use crate::error::RenderError;
use crate::gpu::{camera_bind_layout, GpuMesh};
use std::borrow::Cow;
use std::mem::size_of;

const SHADER_SRC: &str = include_str!("../shaders/mesh.wgsl");

/// Long-lived GPU state for the OccluView mesh pipeline.
pub struct Renderer {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) pipeline: wgpu::RenderPipeline,
    /// Point-list pipeline for `MeshKind::PointCloud` rendering.
    pub(crate) point_pipeline: wgpu::RenderPipeline,
    pub(crate) camera_layout: wgpu::BindGroupLayout,
    pub(crate) camera_buffer: wgpu::Buffer,
    pub(crate) depth_format: wgpu::TextureFormat,
}

impl Renderer {
    /// Create a renderer against a headless device (no surface). Used by the
    /// offscreen thumbnail path and by golden-image tests.
    ///
    /// `target_format` is the output texture's color format (caller-chosen).
    ///
    /// # Errors
    /// - [`RenderError::NoAdapter`] when no adapter is available (incl. WARP-less sandboxes).
    /// - [`RenderError::Surface`] for device-creation failure.
    pub async fn new_headless(target_format: wgpu::TextureFormat) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        // wgpu 22: request_adapter returns Option<Adapter>, not Result.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: true, // WARP-friendly; works headless
                compatible_surface: None,
            })
            .await
            .ok_or(RenderError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("occluview headless device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| RenderError::Surface(e.to_string()))?;

        Self::with_device(device, queue, target_format)
    }

    /// Build the pipeline against an externally-created device/queue (used by
    /// the live app, which owns its own surface-paired adapter).
    ///
    /// # Errors
    /// Returns [`RenderError::Surface`] only if the camera uniform size is
    /// zero (impossible in practice).
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        let depth_format = wgpu::TextureFormat::Depth32Float;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occluview mesh shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER_SRC)),
        });

        let camera_layout = camera_bind_layout(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occluview pipeline layout"),
            bind_group_layouts: &[&camera_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[GpuMesh::vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Point-list pipeline: same shader, PointList topology, no culling.
        // Used for MeshKind::PointCloud (PLY vertex-only files).
        let point_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview point pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[GpuMesh::vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::PointList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let camera_size = size_of::<GpuCamera>() as u64;
        if camera_size == 0 {
            return Err(RenderError::Surface("zero-sized camera".into()));
        }
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview camera uniform"),
            size: camera_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            point_pipeline,
            camera_layout,
            camera_buffer,
            depth_format,
        })
    }

    /// Update the camera uniform for the next frame.
    pub fn set_camera(&self, camera: &GpuCamera) {
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(camera));
    }

    /// Build the per-frame bind group binding the camera uniform at group 0.
    pub fn camera_bind_group(&self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview camera bind group"),
            layout: &self.camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.camera_buffer.as_entire_binding(),
            }],
        })
    }

    /// Issue the draw for one mesh inside a render pass. Caller has already
    /// begun the pass against a color+depth view, set the camera, and will
    /// submit the encoder. Picks the triangle or point pipeline by `kind`.
    pub fn draw<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
        mesh: &'a GpuMesh,
        kind: occluview_core::MeshKind,
    ) {
        let pipe = match kind {
            occluview_core::MeshKind::TriangleMesh => &self.pipeline,
            occluview_core::MeshKind::PointCloud => &self.point_pipeline,
        };
        rpass.set_pipeline(pipe);
        rpass.set_bind_group(0, bind_group, &[]);
        mesh.draw(rpass, kind);
    }

    /// Access the device (for buffer/texture creation by callers).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Access the queue (for buffer writes by callers).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Depth texture format used by this pipeline.
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }
}

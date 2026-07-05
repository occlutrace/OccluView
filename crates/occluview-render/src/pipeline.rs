//! Render pipeline: device + shader + pipeline + per-frame camera uniform.
//!
//! [`Renderer`] owns the long-lived GPU state (bind group layout, render
//! pipeline, camera uniform buffer). Per-frame you call [`Renderer::draw`]
//! inside a render pass.

use crate::camera::GpuCamera;
use crate::clipping::ClipPlane;
use crate::error::RenderError;
use crate::gpu::{camera_bind_layout, GpuMesh};
use crate::mesh_uniform::GpuMeshUniform;
use std::borrow::Cow;
use std::mem::size_of;

const SHADER_SRC: &str = include_str!("../shaders/mesh.wgsl");
const CAP_SHADER_SRC: &str = include_str!("../shaders/cap.wgsl");

/// Vertex layout for the cap quad: position only (vec3<f32>).
fn cap_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    }
}

/// Long-lived GPU state for the OccluView mesh pipeline.
pub struct Renderer {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) pipeline: wgpu::RenderPipeline,
    /// Point-list pipeline for `MeshKind::PointCloud` rendering.
    pub(crate) point_pipeline: wgpu::RenderPipeline,
    pub(crate) camera_layout: wgpu::BindGroupLayout,
    pub(crate) camera_buffer: wgpu::Buffer,
    /// Layout for the per-mesh uniform (group 1): model matrix + tint +
    /// opacity + `has_texture` flag.
    pub(crate) mesh_layout: wgpu::BindGroupLayout,
    /// Layout for the texture + sampler (group 2).
    pub(crate) texture_layout: wgpu::BindGroupLayout,
    /// Layout for the clip plane (group 3): `ClipPlane` uniform.
    pub(crate) clip_layout: wgpu::BindGroupLayout,
    /// Cached disabled clip-plane buffer + bind group. Bound at group 3 for
    /// all draws that don't actually clip (thumbnails, plain renders) so the
    /// shader's `clip.enabled == 0` branch runs. Kept alive behind `dead_code`
    /// because the bind group borrows the buffer.
    #[allow(dead_code)]
    pub(crate) clip_buffer_disabled: wgpu::Buffer,
    pub(crate) clip_bind_group_disabled: wgpu::BindGroup,
    pub(crate) depth_format: wgpu::TextureFormat,
    // --- Stencil capping pipelines (ADR-0011) ---
    /// Pass 1: back faces increment stencil (cull Front, `color_write` none).
    pub(crate) stencil_back_pipeline: wgpu::RenderPipeline,
    /// Pass 2: front faces decrement stencil (cull Back, `color_write` none).
    pub(crate) stencil_front_pipeline: wgpu::RenderPipeline,
    /// Pass 3: cap polygon, stencil test NotEqual(0), flat color.
    pub(crate) cap_pipeline: wgpu::RenderPipeline,
    /// Cap uniform layout (group 1 of cap shader): a single vec4 color.
    pub(crate) cap_uniform_layout: wgpu::BindGroupLayout,
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
    #[allow(clippy::too_many_lines)]
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        // Depth24PlusStencil8 provides both a depth buffer and an 8-bit stencil
        // plane. The stencil plane is required for cross-section capping
        // (ADR-0011); Depth32Float has no stencil. WARP-safe (core WebGPU).
        let depth_format = wgpu::TextureFormat::Depth24PlusStencil8;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occluview mesh shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER_SRC)),
        });

        let camera_layout = camera_bind_layout(&device);
        let mesh_layout = mesh_uniform_bind_layout(&device);
        let texture_layout = texture_bind_layout(&device);
        let clip_layout = clip_plane_bind_layout(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occluview pipeline layout"),
            bind_group_layouts: &[&camera_layout, &mesh_layout, &texture_layout, &clip_layout],
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

        // --- Stencil capping pipelines (ADR-0011) ---
        // A separate cap shader draws the cut-surface polygon. It uses the
        // camera layout (group 0) + a small cap-color uniform (group 1).
        let cap_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occluview cap shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(CAP_SHADER_SRC)),
        });
        let cap_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("occluview cap color layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                }],
            });
        let cap_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occluview cap pipeline layout"),
            bind_group_layouts: &[&camera_layout, &cap_uniform_layout],
            push_constant_ranges: &[],
        });

        // Pass 1: back faces, cull Front, increment stencil, no color write.
        let stencil_back_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview stencil-back pipeline"),
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
                        write_mask: wgpu::ColorWrites::empty(),
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState {
                        front: wgpu::StencilFaceState::default(),
                        back: wgpu::StencilFaceState {
                            compare: wgpu::CompareFunction::Always,
                            fail_op: wgpu::StencilOperation::Keep,
                            depth_fail_op: wgpu::StencilOperation::Keep,
                            pass_op: wgpu::StencilOperation::IncrementClamp,
                        },
                        read_mask: 0xFF,
                        write_mask: 0xFF,
                    },
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        // Pass 2: front faces, cull Back, decrement stencil, no color write.
        let stencil_front_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview stencil-front pipeline"),
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
                        write_mask: wgpu::ColorWrites::empty(),
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
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState {
                        front: wgpu::StencilFaceState {
                            compare: wgpu::CompareFunction::Always,
                            fail_op: wgpu::StencilOperation::Keep,
                            depth_fail_op: wgpu::StencilOperation::Keep,
                            pass_op: wgpu::StencilOperation::DecrementClamp,
                        },
                        back: wgpu::StencilFaceState::default(),
                        read_mask: 0xFF,
                        write_mask: 0xFF,
                    },
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        // Pass 3: cap polygon, stencil test NotEqual(0), flat color, depth
        // write off (the cap sits in the plane, not the mesh surface).
        let cap_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview cap pipeline"),
            layout: Some(&cap_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cap_shader,
                entry_point: "vs_main",
                buffers: &[cap_vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &cap_shader,
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
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Zero,
                    },
                    back: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Zero,
                    },
                    read_mask: 0xFF,
                    write_mask: 0xFF,
                },
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

        // Cached disabled clip-plane buffer: clip.enabled = 0 → no clipping.
        // Bound at group 3 for non-cut draws so the shader's branch is a no-op.
        let clip_buffer_disabled = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview clip plane (disabled)"),
            size: size_of::<ClipPlane>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &clip_buffer_disabled,
            0,
            bytemuck::bytes_of(&ClipPlane::disabled()),
        );
        let clip_bind_group_disabled = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview clip bind group (disabled)"),
            layout: &clip_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: clip_buffer_disabled.as_entire_binding(),
            }],
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            point_pipeline,
            camera_layout,
            camera_buffer,
            mesh_layout,
            texture_layout,
            clip_layout,
            clip_buffer_disabled,
            clip_bind_group_disabled,
            depth_format,
            stencil_back_pipeline,
            stencil_front_pipeline,
            cap_pipeline,
            cap_uniform_layout,
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

    /// Create a uniform buffer + bind group for a per-mesh [`GpuMeshUniform`]
    /// (group 1). Callers write the uniform into the returned buffer via
    /// `queue.write_buffer` before each frame.
    pub fn mesh_uniform_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview mesh uniform"),
            size: size_of::<GpuMeshUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Build the per-mesh bind group binding a uniform buffer at group 1.
    pub fn mesh_bind_group(&self, uniform_buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview mesh bind group"),
            layout: &self.mesh_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        })
    }

    /// The texture bind group layout (group 2): a `texture_2d<f32>` at binding
    /// 0 and a `sampler` at binding 1. Exposed so callers can build bind groups
    /// against their own uploaded textures.
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_layout
    }

    /// The per-mesh uniform bind group layout (group 1). Exposed so callers
    /// can build per-mesh bind groups for multi-mesh scenes.
    pub fn mesh_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.mesh_layout
    }

    /// Create a uniform buffer + bind group for a [`ClipPlane`] (group 3).
    /// Caller writes the plane into the returned buffer before each frame.
    pub fn clip_uniform_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview clip plane uniform"),
            size: size_of::<ClipPlane>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Build the clip-plane bind group (group 3) bound to `uniform_buffer`.
    pub fn clip_bind_group(&self, uniform_buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview clip bind group"),
            layout: &self.clip_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        })
    }

    /// The cached disabled-clip bind group — bound at group 3 for draws that
    /// don't clip (thumbnails, plain renders). Use this instead of building a
    /// fresh group when `clip.enabled == 0`.
    pub fn disabled_clip_bind_group(&self) -> &wgpu::BindGroup {
        &self.clip_bind_group_disabled
    }

    /// Issue the draw for one mesh inside a render pass. Caller has already
    /// begun the pass against a color+depth view, set the camera, and will
    /// submit the encoder. Picks the triangle or point pipeline by `kind`.
    ///
    /// `mesh_bg` is the per-mesh uniform bind group (group 1); `texture_bg`
    /// is the texture+sampler bind group (group 2). For untextured meshes,
    /// pass a 1×1 white fallback texture bind group. `clip_bg` (group 3) is
    /// the clip-plane bind group — pass `disabled_clip_bind_group()` for no
    /// clipping.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        camera_bg: &'a wgpu::BindGroup,
        mesh_bg: &'a wgpu::BindGroup,
        texture_bg: &'a wgpu::BindGroup,
        clip_bg: &'a wgpu::BindGroup,
        mesh: &'a GpuMesh,
        kind: occluview_core::MeshKind,
    ) {
        let pipe = match kind {
            occluview_core::MeshKind::TriangleMesh => &self.pipeline,
            occluview_core::MeshKind::PointCloud => &self.point_pipeline,
        };
        rpass.set_pipeline(pipe);
        rpass.set_bind_group(0, camera_bg, &[]);
        rpass.set_bind_group(1, mesh_bg, &[]);
        rpass.set_bind_group(2, texture_bg, &[]);
        rpass.set_bind_group(3, clip_bg, &[]);
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

/// Bind group layout for the per-mesh uniform (group 1): one uniform buffer
/// visible to both vertex and fragment stages.
fn mesh_uniform_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview mesh uniform layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<GpuMeshUniform>() as u64),
            },
            count: None,
        }],
    })
}

/// Bind group layout for the clip plane (group 3): one uniform buffer
/// holding a [`ClipPlane`], visible to the fragment stage (where discard
/// happens) and vertex stage (future: vertex-side clip distances).
fn clip_plane_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview clip plane layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<ClipPlane>() as u64),
            },
            count: None,
        }],
    })
}

/// Bind group layout for the texture + sampler (group 2): a
/// `texture_2d<f32>` at binding 0 (fragment), a filtering sampler at binding
/// 1 (fragment).
fn texture_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("occluview texture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

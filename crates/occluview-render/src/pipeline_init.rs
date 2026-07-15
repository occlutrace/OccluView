use super::{
    camera_bind_layout, cap_vertex_layout, clip_plane_bind_layout, mesh_uniform_bind_layout,
    multisample_state, point_instance_layout, texture_bind_layout, Renderer, CAP_SHADER_SRC,
    DEFAULT_POINT_SPLAT_VIEWPORT, SHADER_SRC,
};
use crate::clipping::ClipPlane;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use std::{
    borrow::Cow,
    sync::{atomic::AtomicU32, Arc},
};

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
        Self::new_headless_with_adapter_preference(target_format, false).await
    }

    /// Create a headless renderer preferring a real GPU before falling back.
    ///
    /// This is for user-facing viewer/thumbnail paths. Tests can keep using
    /// [`Self::new_headless`] so CI/headless machines stay on the stable
    /// fallback-adapter path.
    ///
    /// # Errors
    /// - [`RenderError::NoAdapter`] when no compatible adapter is available.
    /// - [`RenderError::Surface`] for device-creation failure.
    pub async fn new_headless_prefer_hardware(
        target_format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        Self::new_headless_with_adapter_preference(target_format, true).await
    }

    async fn new_headless_with_adapter_preference(
        target_format: wgpu::TextureFormat,
        prefer_hardware: bool,
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = if prefer_hardware {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
        } else {
            None
        };
        let adapter = if let Some(adapter) = adapter {
            adapter
        } else {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: true,
                    compatible_surface: None,
                })
                .await
                .ok_or(RenderError::NoAdapter)?
        };

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
        Self::with_shared_device(Arc::new(device), Arc::new(queue), target_format)
    }

    /// Build the pipeline against a device/queue owned by the windowing layer.
    ///
    /// The desktop app uses eframe's surface-paired `wgpu` device so the main
    /// viewport can render directly into the swapchain render pass.
    ///
    /// # Errors
    /// Returns [`RenderError::Surface`] only if the camera uniform size is
    /// zero (impossible in practice).
    #[allow(clippy::too_many_lines)]
    pub fn with_shared_device(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
    ) -> Result<Self, RenderError> {
        Self::with_shared_device_sample_count(device, queue, target_format, 1)
    }

    /// Build the pipeline against a shared device/queue with an explicit
    /// render-pass sample count.
    ///
    /// This is used by the live egui viewport: when eframe creates a
    /// multisampled render pass, custom callback pipelines must use the same
    /// `sample_count` or wgpu validation will reject the draw.
    ///
    /// # Errors
    /// Returns [`RenderError::Surface`] only if the camera uniform size is
    /// zero (impossible in practice).
    #[allow(clippy::too_many_lines)]
    pub fn with_shared_device_sample_count(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Result<Self, RenderError> {
        let depth_format = wgpu::TextureFormat::Depth24PlusStencil8;
        let sample_count = sample_count.max(1);
        let multisample = multisample_state(sample_count);

        // Replace wgpu's default uncaptured-error handler (which logs AND
        // panics) with one that records the message. In a release build
        // (`panic = "abort"`), the default handler would turn a recoverable GPU
        // validation error or a transient device fault into a hard crash. Every
        // renderer-creation path funnels through here, so both the live viewport
        // and the offscreen/thumbnail renderers get the safety net.
        let gpu_error: super::GpuErrorLatch = Arc::new(std::sync::Mutex::new(None));
        {
            let sink = Arc::clone(&gpu_error);
            device.on_uncaptured_error(Box::new(move |error| {
                super::record_gpu_error(&sink, error.to_string());
            }));
        }
        {
            // Device-lost is distinct from an uncaptured error: a laptop GPU
            // reset (TDR) or a driver update mid-session tears the device down.
            // `Destroyed` fires on our own normal teardown (device dropped) and
            // is NOT a fault; anything else is a real loss to surface.
            let sink = Arc::clone(&gpu_error);
            device.set_device_lost_callback(move |reason, message| {
                if matches!(reason, wgpu::DeviceLostReason::Destroyed) {
                    return;
                }
                super::record_gpu_error(
                    &sink,
                    format!("graphics device lost ({reason:?}): {message}"),
                );
            });
        }

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
            multisample,
            multiview: None,
            cache: None,
        });

        let point_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview point splat pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_point_splat",
                buffers: &[point_instance_layout()],
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
            multisample,
            multiview: None,
            cache: None,
        });

        let transparent_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview transparent mesh pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview: None,
            cache: None,
        });

        let transparent_point_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview transparent point splat pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_point_splat",
                    buffers: &[point_instance_layout()],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample,
                multiview: None,
                cache: None,
            });

        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview wireframe pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[GpuMesh::vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_wireframe",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview: None,
            cache: None,
        });

        // Cut-view ghost pass: same shader module, alpha-blended, depth-tested
        // without depth write. Built once here, never per frame.
        let ghost_pipeline = super::ghost::build_ghost_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            target_format,
            depth_format,
            multisample,
        );

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
                multisample,
                multiview: None,
                cache: None,
            });

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
                multisample,
                multiview: None,
                cache: None,
            });

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
            multisample,
            multiview: None,
            cache: None,
        });

        let camera_size = size_of::<crate::camera::GpuCamera>() as u64;
        if camera_size == 0 {
            return Err(RenderError::Surface("zero-sized camera".into()));
        }
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview camera uniform"),
            size: camera_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
            transparent_pipeline,
            transparent_point_pipeline,
            wireframe_pipeline,
            ghost_pipeline,
            camera_layout,
            camera_buffer,
            mesh_layout,
            texture_layout,
            clip_layout,
            point_splat_viewport_width_bits: AtomicU32::new(
                DEFAULT_POINT_SPLAT_VIEWPORT[0].to_bits(),
            ),
            point_splat_viewport_height_bits: AtomicU32::new(
                DEFAULT_POINT_SPLAT_VIEWPORT[1].to_bits(),
            ),
            clip_buffer_disabled,
            clip_bind_group_disabled,
            depth_format,
            stencil_back_pipeline,
            stencil_front_pipeline,
            cap_pipeline,
            cap_uniform_layout,
            sample_count,
            gpu_error,
        })
    }
}

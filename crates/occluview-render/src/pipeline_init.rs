use super::{
    camera_bind_layout, cap_vertex_layout, clip_plane_bind_layout, mesh_uniform_bind_layout,
    multisample_state, point_instance_layout, sculpt_brush_bind_layout, sculpt_tool_bind_layout,
    texture_bind_layout, Renderer, CAP_SHADER_SRC, DEFAULT_POINT_SPLAT_VIEWPORT,
    SCULPT_FEEDBACK_SHADER_SRC, SCULPT_TOOL_SHADER_SRC, SHADER_SRC,
};
use crate::clipping::ClipPlane;
use crate::error::RenderError;
use crate::gpu::GpuMesh;
use crate::sculpt_cursor::{
    cone_geometry, cylinder_geometry, vertex_layout as sculpt_tool_vertex_layout,
    SculptBrushUniform, SculptToolUniform,
};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        Arc,
    },
};

/// The depth/stencil format every live-pass pipeline declares.
///
/// eframe derives the live pass attachment from the application's
/// `depth_buffer: 24` / `stencil_buffer: 8`, which resolves to this format, and
/// egui's own pipelines are built for the same one. Keeping it in a named
/// function gives the tests something to compare a probe against instead of
/// duplicating the literal.
#[must_use]
pub const fn live_depth_format() -> wgpu::TextureFormat {
    wgpu::TextureFormat::Depth24PlusStencil8
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
        let (renderer, _adapter_result) =
            Self::new_headless_on_adapter(target_format, true).await?;
        Ok(renderer)
    }

    /// Create a headless renderer on one explicit adapter kind.
    ///
    /// The offscreen policy layer owns fallback and verification. This lower
    /// layer deliberately makes one adapter request only, so a consumer cannot
    /// mistake a successful device allocation for a verified hardware frame.
    pub(crate) async fn new_headless_on_adapter(
        target_format: wgpu::TextureFormat,
        force_fallback_adapter: bool,
    ) -> Result<(Self, crate::offscreen::AdapterResult), RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: if force_fallback_adapter {
                    wgpu::PowerPreference::LowPower
                } else {
                    wgpu::PowerPreference::HighPerformance
                },
                force_fallback_adapter,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|_| RenderError::NoAdapter)?;
        let adapter_result =
            crate::offscreen::adapter_result_for_device_type(adapter.get_info().device_type);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("occluview headless device"),
                required_features: wgpu::Features::empty(),
                // The conservative floor, raised to what the adapter
                // actually offers. `downlevel_defaults` caps
                // `max_texture_dimension_2d` at 2048, while the format
                // readers accept textures up to 8192 -- so a scan with a
                // 4096-pixel atlas decoded, cost its memory, and then had
                // nowhere to go. `using_resolution` copies the three
                // texture dimensions and nothing else, so the 256 MiB
                // buffer floor stayed: a scan of three million triangles
                // needs a 309 MiB vertex buffer, the allocation was
                // refused, and the frame came back empty.
                required_limits: wgpu::Limits {
                    max_buffer_size: adapter.limits().max_buffer_size,
                    ..wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
                },
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| RenderError::Surface(e.to_string()))?;

        Ok((
            Self::with_device(device, queue, target_format)?,
            adapter_result,
        ))
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
        let depth_format = live_depth_format();
        let sample_count = sample_count.max(1);
        let multisample = multisample_state(sample_count);

        // Replace wgpu's default uncaptured-error handler (which logs AND
        // panics) with one that records the message. In a release build
        // (`panic = "abort"`), the default handler would turn a recoverable GPU
        // validation error or a transient device fault into a hard crash. Every
        // renderer-creation path funnels through here, so both the live viewport
        // and the offscreen/thumbnail renderers get the safety net.
        let gpu_error: super::GpuErrorLatch = Arc::new(std::sync::Mutex::new(None));
        let gpu_faulted = Arc::new(AtomicBool::new(false));
        {
            let sink = Arc::clone(&gpu_error);
            let faulted = Arc::clone(&gpu_faulted);
            device.on_uncaptured_error(Arc::new(move |error| {
                super::record_gpu_fault(&sink, &faulted, error.to_string());
            }));
        }
        {
            // Device-lost is distinct from an uncaptured error: a laptop GPU
            // reset (TDR) or a driver update mid-session tears the device down.
            // `Destroyed` fires on our own normal teardown (device dropped) and
            // is NOT a fault; anything else is a real loss to surface.
            let sink = Arc::clone(&gpu_error);
            let faulted = Arc::clone(&gpu_faulted);
            device.set_device_lost_callback(move |reason, message| {
                if matches!(reason, wgpu::DeviceLostReason::Destroyed) {
                    return;
                }
                super::record_gpu_fault(
                    &sink,
                    &faulted,
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
        let sculpt_brush_layout = sculpt_brush_bind_layout(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("occluview pipeline layout"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&mesh_layout),
                Some(&texture_layout),
                Some(&clip_layout),
            ],
            immediate_size: 0,
        });
        let sculpt_feedback_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("occluview sculpt feedback pipeline layout"),
                bind_group_layouts: &[
                    Some(&camera_layout),
                    Some(&mesh_layout),
                    Some(&clip_layout),
                    Some(&sculpt_brush_layout),
                ],
                immediate_size: 0,
            });
        let sculpt_tool_layout = sculpt_tool_bind_layout(&device);
        let sculpt_tool_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("occluview sculpt tool pipeline layout"),
                bind_group_layouts: &[
                    Some(&camera_layout),
                    Some(&sculpt_tool_layout),
                    Some(&clip_layout),
                ],
                immediate_size: 0,
            });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(GpuMesh::vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        let point_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview point splat pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_point_splat"),
                buffers: &[Some(point_instance_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // The point shader emits a smooth coverage alpha for the
                    // outer part of each screen-space splat. Replacing the
                    // target ignores that alpha and leaves a hard disc edge
                    // even when live MSAA is available.
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        let transparent_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview transparent mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(GpuMesh::vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        let transparent_point_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview transparent point splat pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_point_splat"),
                    buffers: &[Some(point_instance_layout())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
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
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample,
                multiview_mask: None,
                cache: None,
            });

        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview wireframe pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(GpuMesh::vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_wireframe"),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
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

        let sculpt_feedback_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occluview sculpt feedback shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SCULPT_FEEDBACK_SHADER_SRC)),
        });
        // The surface feedback pass is additive and depth-tested against the
        // already-rendered target mesh. It emits only the brush field, so
        // textures, scan colors, heatmaps, and their alpha are never doubled.
        let sculpt_feedback_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview sculpt surface feedback pipeline"),
                layout: Some(&sculpt_feedback_layout),
                vertex: wgpu::VertexState {
                    module: &sculpt_feedback_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(GpuMesh::vertex_layout())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &sculpt_feedback_shader,
                    entry_point: Some("fs_sculpt_feedback"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::Zero,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::RED
                            | wgpu::ColorWrites::GREEN
                            | wgpu::ColorWrites::BLUE,
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
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample,
                multiview_mask: None,
                cache: None,
            });

        let sculpt_tool_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occluview sculpt tool shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SCULPT_TOOL_SHADER_SRC)),
        });
        // The tool volume remains depth-independent at the semantic level:
        // it never writes depth and always passes the depth test, so the
        // reference cursor stays readable over dense surfaces. It still has
        // to declare the live pass's depth format because eframe places this
        // draw in the same Depth24PlusStencil8 render pass.
        let sculpt_tool_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview sculpt tool volume pipeline"),
            layout: Some(&sculpt_tool_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sculpt_tool_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(sculpt_tool_vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &sculpt_tool_shader,
                entry_point: Some("fs_main"),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

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
            bind_group_layouts: &[Some(&camera_layout), Some(&cap_uniform_layout)],
            immediate_size: 0,
        });

        let stencil_back_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview stencil-back pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(GpuMesh::vertex_layout())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
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
                    // These passes build only the stencil mask. Writing their
                    // mesh depth would make the final opaque pass's `Less`
                    // test reject the same surface as equal before it can
                    // paint, and would also leave no usable depth for the
                    // cut-plane cap.
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
                multiview_mask: None,
                cache: None,
            });

        let stencil_front_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("occluview stencil-front pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(GpuMesh::vertex_layout())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
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
                    // Stencil winding is an occlusion mask, not a depth pass;
                    // preserve the clear depth for the cap and shaded draw.
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
                multiview_mask: None,
                cache: None,
            });

        let cap_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("occluview cap pipeline"),
            layout: Some(&cap_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cap_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(cap_vertex_layout())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &cap_shader,
                entry_point: Some("fs_main"),
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
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
            multiview_mask: None,
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

        let sculpt_brush_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview sculpt brush uniform"),
            size: size_of::<SculptBrushUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &sculpt_brush_buffer,
            0,
            bytemuck::bytes_of(&SculptBrushUniform::hidden()),
        );
        let sculpt_brush_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview sculpt brush bind group"),
            layout: &sculpt_brush_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sculpt_brush_buffer.as_entire_binding(),
            }],
        });

        let sculpt_tool_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluview sculpt tool uniform"),
            size: size_of::<SculptToolUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &sculpt_tool_buffer,
            0,
            bytemuck::bytes_of(&SculptToolUniform::hidden()),
        );
        let sculpt_tool_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occluview sculpt tool bind group"),
            layout: &sculpt_tool_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sculpt_tool_buffer.as_entire_binding(),
            }],
        });

        let (cone_vertices, cone_indices) = cone_geometry();
        let (sculpt_tool_cone_buffer, sculpt_tool_cone_vertex_bytes) = upload_sculpt_tool_buffer(
            &device,
            &queue,
            "occluview sculpt cone buffer",
            &cone_vertices,
            &cone_indices,
        );
        let (cylinder_vertices, cylinder_indices) = cylinder_geometry();
        let (sculpt_tool_cylinder_buffer, sculpt_tool_cylinder_vertex_bytes) =
            upload_sculpt_tool_buffer(
                &device,
                &queue,
                "occluview sculpt cylinder buffer",
                &cylinder_vertices,
                &cylinder_indices,
            );

        Ok(Self {
            device,
            queue,
            pipeline,
            point_pipeline,
            transparent_pipeline,
            transparent_point_pipeline,
            wireframe_pipeline,
            ghost_pipeline,
            sculpt_feedback_pipeline,
            sculpt_tool_pipeline,
            camera_layout,
            camera_buffer,
            mesh_layout,
            texture_layout,
            clip_layout,
            sculpt_brush_buffer,
            sculpt_brush_bind_group,
            sculpt_tool_buffer,
            sculpt_tool_bind_group,
            sculpt_tool_shape: AtomicU32::new(0),
            sculpt_tool_cone_buffer,
            sculpt_tool_cone_vertex_bytes,
            sculpt_tool_cone_index_count: u32::try_from(cone_indices.len()).unwrap_or(u32::MAX),
            sculpt_tool_cylinder_buffer,
            sculpt_tool_cylinder_vertex_bytes,
            sculpt_tool_cylinder_index_count: u32::try_from(cylinder_indices.len())
                .unwrap_or(u32::MAX),
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
            gpu_faulted,
        })
    }
}

fn upload_sculpt_tool_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    vertices: &[crate::sculpt_cursor::SculptToolVertex],
    indices: &[u32],
) -> (wgpu::Buffer, u64) {
    let vertex_bytes = bytemuck::cast_slice(vertices);
    let index_bytes = bytemuck::cast_slice(indices);
    let vertex_size = vertex_bytes.len() as u64;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: vertex_size + index_bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::INDEX
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, vertex_bytes);
    queue.write_buffer(&buffer, vertex_size, index_bytes);
    (buffer, vertex_size)
}

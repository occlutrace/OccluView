//! Ghost pipeline for the cut view.
//!
//! A second draw over the *same* mesh shader module (`vs_main` + `fs_ghost`)
//! that renders the cut-away side of a cross-section as a translucent shell,
//! so the OWNER rule holds: a cut never removes geometry from view, it fades
//! it. Built once at renderer init (never per frame). Alpha-blended, depth
//! test on / depth write off, drawn after the opaque pass.

use crate::gpu::GpuMesh;

/// Build the ghost pipeline, sharing `layout` and `shader` with the opaque
/// mesh pipeline. Blending is standard (non-premultiplied) source-alpha so the
/// faint fixed-alpha output from `fs_ghost` composites over the kept side.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_ghost_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    multisample: wgpu::MultisampleState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("occluview ghost pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &[GpuMesh::vertex_layout()],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_ghost",
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
        // Test against the opaque depth so the kept side correctly occludes the
        // ghost, but do not write depth: the ghost must never block anything.
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
    })
}

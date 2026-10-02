use super::sculpt_cursor::{
    cylinder_geometry, sculpt_footprint_field, sculpt_surface_light_intensity, sculpt_tool_length,
    SculptBrushUniform, SculptFeedbackStyle, SculptTipStamp, SculptToolShape, SculptToolUniform,
    SCULPT_CYLINDER_PLATEAU, SCULPT_KNIFE_CROSS_SHARE,
};
use std::mem::size_of;

#[test]
fn cursor_uniforms_have_the_wgsl_alignment_their_bindings_require() {
    assert_eq!(size_of::<SculptBrushUniform>(), 96);
    assert_eq!(size_of::<SculptToolUniform>(), 96);
}

/// A ball dab is brightest at the centre, fades monotonically and stops at the
/// radius, so the mark cannot read as a hard-edged disc.
#[test]
fn ball_footprint_fades_from_the_centre_to_the_rim() {
    let field = |radius_share: f32| {
        sculpt_footprint_field(
            SculptTipStamp::Ball,
            [radius_share, 0.0, 0.0],
            1.0,
            [0.0; 3],
        )
    };
    assert_eq!(field(0.0), 1.0);
    assert!(field(0.4) > field(0.7));
    assert!(field(0.7) > field(0.99));
    assert!(field(0.99) > 0.0);
    assert_eq!(field(1.0), 0.0);
    assert_eq!(field(1.5), 0.0);
}

/// A knife reaches further along its bearing than across it: at 0.6 of the
/// radius it still cuts along the axis and has already ended across it.
#[test]
fn knife_footprint_is_longer_along_its_axis_than_across() {
    let axis = [1.0, 0.0, 0.0];
    let along = sculpt_footprint_field(SculptTipStamp::Knife, [0.6, 0.0, 0.0], 1.0, axis);
    let across = sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.6, 0.0], 1.0, axis);
    assert!(along > 0.0, "the blade must still cut along its bearing");
    assert_eq!(across, 0.0, "the blade is narrower than 0.6 r across");
    assert!(across < along);
    // The transverse reach is the documented share of the along reach.
    assert!(sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.5, 0.0], 1.0, axis) > 0.0);
    assert_eq!(
        sculpt_footprint_field(
            SculptTipStamp::Knife,
            [0.0, SCULPT_KNIFE_CROSS_SHARE + 0.01, 0.0],
            1.0,
            axis
        ),
        0.0
    );
    // A press with no bearing falls back to a narrow radial footprint: it
    // reaches less far than the ball in every direction and ends before 0.8 r.
    let no_axis = sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.6, 0.0], 1.0, [0.0; 3]);
    let ball = sculpt_footprint_field(SculptTipStamp::Ball, [0.0, 0.6, 0.0], 1.0, [0.0; 3]);
    assert!(
        no_axis > 0.0 && no_axis < ball,
        "got {no_axis} against {ball}"
    );
    assert_eq!(
        sculpt_footprint_field(SculptTipStamp::Knife, [0.8, 0.0, 0.0], 1.0, [0.0; 3]),
        0.0
    );
    assert_eq!(
        sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.2, 0.0], 1.0, [0.0; 3]),
        sculpt_footprint_field(SculptTipStamp::Knife, [0.2, 0.0, 0.0], 1.0, [0.0; 3])
    );
}

/// A cylinder is flat inside its plateau and only softens over the rim band,
/// which is what makes it level a face instead of raising a mound.
#[test]
fn cylinder_footprint_is_a_plateau_with_a_soft_rim() {
    let field = |radius_share: f32| {
        sculpt_footprint_field(
            SculptTipStamp::Cylinder,
            [radius_share, 0.0, 0.0],
            1.0,
            [0.0; 3],
        )
    };
    assert_eq!(field(0.0), 1.0);
    assert_eq!(field(SCULPT_CYLINDER_PLATEAU * 0.5), 1.0);
    assert_eq!(field(SCULPT_CYLINDER_PLATEAU), 1.0);
    let rim = field(SCULPT_CYLINDER_PLATEAU + (1.0 - SCULPT_CYLINDER_PLATEAU) * 0.5);
    assert!(rim > 0.0 && rim < 1.0, "the rim must blend, got {rim}");
    assert_eq!(field(1.0), 0.0);
}

#[test]
fn tool_height_is_finite_and_bounded() {
    assert_eq!(sculpt_tool_length(f32::NEG_INFINITY), 0.65);
    assert_eq!(sculpt_tool_length(f32::NAN), 0.65);
    assert_eq!(sculpt_tool_length(0.0), 0.65);
    assert_eq!(sculpt_tool_length(1.0), 2.6);
    assert!(sculpt_tool_length(0.5) > 0.65);
    assert!(sculpt_tool_length(0.5) < 2.6);
}

#[test]
fn surface_glow_is_monotonic_without_a_zero_strength_blackout() {
    let weak = sculpt_surface_light_intensity(0.0);
    let medium = sculpt_surface_light_intensity(0.5);
    let strong = sculpt_surface_light_intensity(1.0);

    assert!(weak > 0.0);
    assert!(weak < medium);
    assert!(medium < strong);
    assert!(sculpt_surface_light_intensity(f32::INFINITY).is_finite());
}

#[test]
fn tool_shapes_have_stable_gpu_tags() {
    assert_eq!(SculptToolShape::Cone as u32, 0);
    assert_eq!(SculptToolShape::Cylinder as u32, 1);
    assert_eq!(SculptToolShape::Knife as u32, 2);
}

#[test]
fn cursor_feedback_styles_have_stable_gpu_tags() {
    assert_eq!(SculptFeedbackStyle::Solid as u32, 0);
    assert_eq!(SculptFeedbackStyle::Dashed as u32, 1);
}

#[test]
fn tool_geometry_is_open_and_uses_bounded_static_buffers() {
    let (vertices, indices) = cylinder_geometry();
    assert_eq!(vertices.len(), 32 * 4);
    assert_eq!(indices.len(), 32 * 6);
    assert!(vertices.iter().all(|vertex| {
        vertex
            .position
            .iter()
            .all(|component| component.is_finite())
            && vertex.normal.iter().all(|component| component.is_finite())
    }));
    assert!(indices
        .iter()
        .all(|&index| (index as usize) < vertices.len()));
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FieldProbe {
    offset_radius: [f32; 4],
    axis: [f32; 3],
    tip: u32,
    knife_cross_share: f32,
    cylinder_plateau: f32,
    knife_axis_min_length: f32,
    padding: f32,
}

#[test]
#[allow(clippy::expect_used)]
fn gpu_sculpt_feedback_field_matches_the_kernel_field() {
    let probes = field_probes(SculptBrushUniform::hidden());
    let renderer = pollster::block_on(crate::Renderer::new_headless(
        wgpu::TextureFormat::Rgba8Unorm,
    ))
    .expect("a headless adapter");
    let actual = gpu_field_values(&renderer, &probes);
    assert_field_values(&probes, &actual);
}

fn field_probes(brush: SculptBrushUniform) -> [FieldProbe; 12] {
    let mut zero_radius = field_probe(SculptTipStamp::Ball, [0.0; 3], [0.0; 3], brush);
    zero_radius.offset_radius[3] = 0.0;
    let mut sub_tenth_mm_radius =
        field_probe(SculptTipStamp::Ball, [0.000_075, 0.0, 0.0], [0.0; 3], brush);
    sub_tenth_mm_radius.offset_radius[3] = 0.000_05;
    [
        field_probe(SculptTipStamp::Ball, [0.0, 0.0, 0.0], [0.0; 3], brush),
        field_probe(SculptTipStamp::Ball, [0.4, 0.0, 0.0], [0.0; 3], brush),
        field_probe(SculptTipStamp::Ball, [0.9, 0.0, 0.0], [0.0; 3], brush),
        field_probe(
            SculptTipStamp::Knife,
            [0.6, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            brush,
        ),
        field_probe(
            SculptTipStamp::Knife,
            [0.0, 0.5, 0.0],
            [1.0, 0.0, 0.0],
            brush,
        ),
        field_probe(
            SculptTipStamp::Knife,
            [0.0, 0.57, 0.0],
            [1.0, 0.0, 0.0],
            brush,
        ),
        field_probe(SculptTipStamp::Knife, [0.0, 0.6, 0.0], [0.0; 3], brush),
        field_probe(SculptTipStamp::Cylinder, [0.7, 0.0, 0.0], [0.0; 3], brush),
        field_probe(SculptTipStamp::Cylinder, [0.9, 0.0, 0.0], [0.0; 3], brush),
        field_probe(SculptTipStamp::Cylinder, [1.0, 0.0, 0.0], [0.0; 3], brush),
        zero_radius,
        sub_tenth_mm_radius,
    ]
}

const FIELD_PROBE_SHADER_MAIN: &str = r"
struct FieldProbe {
    offset_radius: vec4<f32>,
    axis: vec3<f32>,
    tip: u32,
    knife_cross_share: f32,
    cylinder_plateau: f32,
    knife_axis_min_length: f32,
    padding: f32,
}
@group(0) @binding(0) var<storage, read> probes: array<FieldProbe>;
@group(0) @binding(1) var<storage, read_write> results: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index >= arrayLength(&probes) {
        return;
    }
    let probe = probes[index];
    results[index] = sculpt_brush_field(
        probe.tip,
        probe.offset_radius.xyz,
        probe.offset_radius.w,
        probe.axis,
        probe.knife_cross_share,
        probe.cylinder_plateau,
        probe.knife_axis_min_length,
    );
}
";

#[allow(clippy::expect_used)]
fn gpu_field_values(renderer: &crate::Renderer, probes: &[FieldProbe]) -> Vec<f32> {
    use std::sync::mpsc;

    let device = renderer.device();
    let queue = renderer.queue();
    let pipeline = field_probe_pipeline(device);
    let input_bytes: &[u8] = bytemuck::cast_slice(probes);
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sculpt field probes"),
        size: input_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&input, 0, input_bytes);
    let result_bytes = size_of::<f32>() * probes.len();
    let results = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sculpt field results"),
        size: result_bytes as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sculpt field readback"),
        size: result_bytes as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sculpt field probe bindings"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: results.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sculpt field parity commands"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sculpt field parity"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(probes.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&results, 0, &readback, 0, result_bytes as u64);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("GPU map callback")
        .expect("GPU map succeeds");
    assert_eq!(renderer.take_gpu_error(), None, "GPU field probe is valid");
    let mapped = slice.get_mapped_range().expect("read mapped field results");
    let actual = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    assert_eq!(renderer.take_gpu_error(), None);
    actual
}

#[allow(clippy::expect_used)]
fn field_probe_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    use std::borrow::Cow;

    let source = format!(
        "{}\n{}",
        include_str!("../shaders/sculpt_field.wgsl"),
        FIELD_PROBE_SHADER_MAIN
    );
    let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sculpt field parity test"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(source)),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("sculpt field parity test"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let pipeline_error = pollster::block_on(error_scope.pop());
    assert!(
        pipeline_error.is_none(),
        "compute field shader: {pipeline_error:?}"
    );
    pipeline
}

#[allow(clippy::expect_used)]
fn assert_field_values(probes: &[FieldProbe], actual: &[f32]) {
    use glam::DVec3;
    use occluview_geometry::stamp_weight;

    assert_eq!(probes.len(), actual.len());
    for (index, (probe, actual)) in probes.iter().zip(actual).enumerate() {
        let offset = DVec3::new(
            f64::from(probe.offset_radius[0]),
            f64::from(probe.offset_radius[1]),
            f64::from(probe.offset_radius[2]),
        );
        let axis = DVec3::new(
            f64::from(probe.axis[0]),
            f64::from(probe.axis[1]),
            f64::from(probe.axis[2]),
        );
        let axis = axis.is_finite().then_some(axis);
        let expected = stamp_weight(
            SculptTipStamp::from_u32(probe.tip).expect("known tip tag"),
            offset,
            offset.length(),
            axis,
            f64::from(probe.offset_radius[3]),
        ) as f32;
        assert!(
            (actual - expected).abs() < 1e-5,
            "field probe {index} returned {actual}, kernel returned {expected}"
        );
    }
}

fn field_probe(
    tip: SculptTipStamp,
    offset: [f32; 3],
    axis: [f32; 3],
    brush: SculptBrushUniform,
) -> FieldProbe {
    FieldProbe {
        offset_radius: [offset[0], offset[1], offset[2], brush.radius],
        axis,
        tip: tip as u32,
        knife_cross_share: brush.knife_cross_share,
        cylinder_plateau: brush.cylinder_plateau,
        knife_axis_min_length: brush.knife_axis_min_length,
        padding: 0.0,
    }
}

// Additive, display-only Sculpt surface feedback.
//
// Kept in its own shader module so the ordinary mesh pipeline remains exactly
// the material/depth path used by thumbnails, cut views, and every non-Sculpt
// frame. The selected mesh is drawn a second time with only this light field.

struct Camera {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    light_dir: vec3<f32>,
    point_viewport_width: f32,
    camera_pos: vec3<f32>,
    point_viewport_height: f32,
}

struct MeshUniform {
    model: mat4x4<f32>,
    tint: vec4<f32>,
    opacity: f32,
    has_texture: u32,
    show_orientation: u32,
    show_vertex_colors: u32,
    show_texture: u32,
    measured_map: u32,
    _padding_0: u32,
    _padding_1: u32,
}

struct ClipPlane {
    normal: vec3<f32>,
    distance: f32,
    enabled: u32,
    _pad: u32,
}

struct SculptBrushUniform {
    center: vec3<f32>,
    radius: f32,
    normal: vec3<f32>,
    intensity: f32,
    axis: vec3<f32>,
    tip: u32,
    color: vec4<f32>,
    visible: u32,
    knife_cross_share: f32,
    cylinder_plateau: f32,
    knife_axis_min_length: f32,
    edge_style: u32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> mesh_uniform: MeshUniform;
@group(2) @binding(0) var<uniform> clip: ClipPlane;
@group(3) @binding(0) var<uniform> sculpt_brush: SculptBrushUniform;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<u32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let world = mesh_uniform.model * vec4<f32>(in.position, 1.0);
    var out: VertexOut;
    out.clip_pos = camera.projection * camera.view * world;
    out.world_pos = world.xyz;
    out.normal = (mesh_uniform.model * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

// Emit only an additive RGB field. The target mesh's material was already
// written by the normal scene pass, so this cannot double scan colors,
// textures, heatmap hues, or opacity.
@fragment
fn fs_sculpt_feedback(in: VertexOut) -> @location(0) vec4<f32> {
    if sculpt_brush.visible == 0u {
        discard;
    }
    if clip.enabled != 0u && dot(in.world_pos, clip.normal) - clip.distance < 0.0 {
        discard;
    }
    let radius = max(sculpt_brush.radius, 0.0001);
    let offset = in.world_pos - sculpt_brush.center;
    let field = sculpt_brush_field(
        sculpt_brush.tip,
        offset,
        radius,
        sculpt_brush.axis,
        sculpt_brush.knife_cross_share,
        sculpt_brush.cylinder_plateau,
        sculpt_brush.knife_axis_min_length,
    );
    var n = in.normal;
    if length(n) < 0.001 {
        n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        n = normalize(n);
    }
    var brush_n = sculpt_brush.normal;
    if length(brush_n) < 0.001 {
        brush_n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        brush_n = normalize(brush_n);
    }
    let edge_rho = sculpt_brush_edge_coordinate(
        sculpt_brush.tip,
        offset,
        radius,
        sculpt_brush.axis,
        sculpt_brush.knife_cross_share,
        sculpt_brush.knife_axis_min_length,
    );
    let rim_width = max(fwidth(edge_rho), 0.00001);
    var rim = 1.0 - smoothstep(0.0, 1.5 * rim_width, abs(edge_rho - 1.0));
    if sculpt_brush.edge_style == 1u {
        let angle = sculpt_brush_rim_angle(offset, brush_n);
        let dash = fract((angle + 3.14159265) / 6.2831853 * 20.0);
        rim *= select(0.0, 1.0, dash < 0.62);
    }
    if field <= 0.0 && rim <= 0.0 {
        discard;
    }
    let alignment = abs(dot(n, brush_n));
    let visible_field = field * (0.58 + 0.42 * alignment);
    return vec4<f32>(
        sculpt_brush.color.rgb * (sculpt_brush.intensity * visible_field + 0.24 * rim),
        0.0,
    );
}

fn sculpt_brush_edge_coordinate(
    tip: u32,
    offset: vec3<f32>,
    radius: f32,
    axis_input: vec3<f32>,
    knife_cross_share: f32,
    knife_axis_min_length: f32,
) -> f32 {
    if tip == 1u {
        let axis_length = length(axis_input);
        if axis_length > knife_axis_min_length {
            let axis = axis_input / axis_length;
            let along = dot(offset, axis);
            let across = length(offset - axis * along);
            return length(vec2<f32>(along, across / knife_cross_share)) / radius;
        }
        return length(offset) / (radius * sqrt(knife_cross_share));
    }
    return length(offset) / radius;
}

fn sculpt_brush_rim_angle(offset: vec3<f32>, normal_input: vec3<f32>) -> f32 {
    let normal = normalize(normal_input);
    let reference = select(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 1.0, 0.0),
        abs(normal.x) > 0.8,
    );
    let tangent = normalize(cross(normal, reference));
    let bitangent = cross(normal, tangent);
    return atan2(dot(offset, bitangent), dot(offset, tangent));
}

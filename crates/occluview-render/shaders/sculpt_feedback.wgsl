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
    color: vec4<f32>,
    tip: u32,
    visible: u32,
    _padding_0: u32,
    _padding_1: u32,
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
    let rho = length(offset) / radius;
    if rho >= 1.0 {
        discard;
    }

    // Ball/knife-style soft field for Add/Remove; a wider plateau for Smooth.
    var field = pow(max(0.0, 1.0 - rho), 2.0);
    if sculpt_brush.tip == 1u {
        field = 1.0 - smoothstep(0.80, 1.0, rho);
    }

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
    let alignment = abs(dot(n, brush_n));
    let visible_field = field * (0.58 + 0.42 * alignment);
    return vec4<f32>(
        sculpt_brush.color.rgb * sculpt_brush.intensity * visible_field,
        0.0,
    );
}

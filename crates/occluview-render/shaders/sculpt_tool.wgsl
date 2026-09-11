// Translucent, display-only Sculpt tool volume.
//
// The canonical geometry is an open cone or cylinder. Its transform, color,
// and opacity come from SculptToolUniform; it never writes depth and never
// participates in picking or mesh editing.

struct Camera {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    light_dir: vec3<f32>,
    point_viewport_width: f32,
    camera_pos: vec3<f32>,
    point_viewport_height: f32,
}

struct SculptToolUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
    opacity: f32,
    shape: u32,
    visible: u32,
    _padding: u32,
}

struct ClipPlane {
    normal: vec3<f32>,
    distance: f32,
    enabled: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> tool: SculptToolUniform;
@group(2) @binding(0) var<uniform> clip: ClipPlane;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let world = tool.model * vec4<f32>(in.position, 1.0);
    var out: VertexOut;
    out.clip_pos = camera.projection * camera.view * world;
    out.world_pos = world.xyz;
    out.normal = (tool.model * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(
    in: VertexOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    if tool.visible == 0u {
        discard;
    }
    if clip.enabled != 0u && dot(in.world_pos, clip.normal) - clip.distance < 0.0 {
        discard;
    }

    var n = in.normal;
    if length(n) < 0.001 {
        n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        n = normalize(n);
    }
    var view_dir = camera.camera_pos - in.world_pos;
    if length(view_dir) < 0.001 {
        view_dir = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        view_dir = normalize(view_dir);
    }
    if dot(n, view_dir) < 0.0 {
        n = -n;
    }
    let key = normalize(camera.light_dir);
    let ndotl = max(dot(n, key), 0.0);
    let half_vec = normalize(key + view_dir);
    let specular = pow(max(dot(n, half_vec), 0.0), 36.0);
    let edge = 1.0 - clamp(dot(n, view_dir), 0.0, 1.0);
    let front_mix = select(0.90, 1.0, front_facing);
    let form = front_mix * (0.72 + 0.28 * ndotl);
    let rgb = clamp(tool.color.rgb * form + vec3<f32>(0.22 * specular + 0.08 * edge), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb, clamp(tool.opacity * (0.86 + 0.14 * ndotl), 0.0, 0.72));
}

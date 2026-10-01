// Translucent, display-only Sculpt tool volume.
//
// The display geometry is an open cylinder. The vertex shader shapes it for
// the active tip and modifier state; it never writes depth or participates in
// picking or mesh editing.

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
    action: vec2<f32>,
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
    // Position along the tool axis, 0 at the contact end and 1 at the far end.
    @location(2) axial: f32,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var profile_radius = 1.0;
    var cylinder = 0.0;
    if tool.shape == 1u {
        cylinder = 1.0;
    } else if tool.shape == 2u {
        profile_radius = 0.45;
    }
    let invert = clamp(tool.action.x, 0.0, 1.0);
    let flat = clamp(tool.action.y, 0.0, 1.0);
    let z = in.position.z;
    let rising = profile_radius * (1.0 - z);
    let pressing = profile_radius * z;
    let cone = mix(rising, pressing, invert);
    let radius = mix(mix(cone, 1.0, cylinder), mix(profile_radius, 1.0, cylinder), flat);
    let cone_slope = mix(-profile_radius, profile_radius, invert);
    let slope = mix(mix(cone_slope, 0.0, cylinder), 0.0, flat);
    let radial = normalize(in.position.xy);
    let shaped = vec3<f32>(radial * radius, z);
    let width_scale = max(length(tool.model[0].xyz), 0.000001);
    let height_scale = max(length(tool.model[2].xyz), 0.000001);
    let shaped_normal = vec3<f32>(radial, -slope * width_scale / height_scale);
    let world = tool.model * vec4<f32>(shaped, 1.0);
    var out: VertexOut;
    out.clip_pos = camera.projection * camera.view * world;
    out.world_pos = world.xyz;
    out.normal = (tool.model * vec4<f32>(shaped_normal, 0.0)).xyz;
    out.axial = z;
    return out;
}

@fragment
fn fs_main(
    in: VertexOut,
) -> @location(0) vec4<f32> {
    if tool.opacity <= 0.0 {
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
    // The body is a hollow glass shell: its brightness is the rim, not the
    // surface. Fresnel drives both the colour and the alpha, and the ends fade
    // so the contact point reads as the tool's tip rather than a cut cylinder.
    let fresnel = pow(1.0 - abs(dot(n, view_dir)), 1.35);
    let invert = clamp(tool.action.x, 0.0, 1.0);
    let far = mix(0.7, 0.84, invert);
    let axial = smoothstep(0.0, 0.1, in.axial) * (1.0 - smoothstep(far, 1.0, in.axial));
    let alpha = tool.opacity * (0.18 + 0.82 * fresnel) * axial;
    if alpha < 0.02 {
        discard;
    }
    return vec4<f32>(tool.color.rgb, alpha);
}

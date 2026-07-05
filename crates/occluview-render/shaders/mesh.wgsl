// OccluView mesh shader: flat-shaded, vertex-color, or texture-mapped.
//
// Vertex format matches occluview_core::Vertex (#[repr(C)], 36 bytes):
//   position: [f32; 3]  @ offset 0
//   normal:   [f32; 3]  @ offset 12
//   color:    [u8; 4]   @ offset 24
//   uv:       [f32; 2]  @ offset 28
//
// Bindings:
//   group 0 binding 0: camera uniform (view + projection + light + eye)
//   group 1 binding 0: per-mesh uniform (model matrix + tint + opacity +
//                      has_texture flag)
//   group 2 binding 0: texture_2d (optional; bound only when has_texture != 0)
//   group 2 binding 1: sampler    (optional; same)

struct Camera {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    light_dir: vec3<f32>,
    _pad0: f32,
    camera_pos: vec3<f32>,
    _pad1: f32,
}

struct MeshUniform {
    model: mat4x4<f32>,
    tint: vec4<f32>,
    opacity: f32,
    has_texture: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> mesh_uniform: MeshUniform;

@group(2) @binding(0) var mesh_texture: texture_2d<f32>;
@group(2) @binding(1) var mesh_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<u32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // World position via the per-mesh model matrix.
    let world_pos = mesh_uniform.model * vec4<f32>(in.position, 1.0);
    out.clip_pos = camera.projection * camera.view * world_pos;
    // Normalize u8 color channels to 0..1. wgsl has no direct u32->f32 on
    // vectors, so unpack element by element.
    out.color = vec3<f32>(
        f32(in.color.r) / 255.0,
        f32(in.color.g) / 255.0,
        f32(in.color.b) / 255.0,
    );
    // Transform the normal by the model matrix (ignoring translation). For
    // uniform-scale transforms this is correct; non-uniform scale would need
    // the inverse-transpose, which OccluView does not use for scene placement.
    out.normal = (mesh_uniform.model * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Lambertian shading with a soft ambient. If the normal is zero (STL with
    // no normals), fall back to a flat mid-grey so the silhouette is visible.
    var n = in.normal;
    let n_len = length(n);
    if (n_len < 0.001) {
        n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        n = normalize(n);
    }
    let l = normalize(camera.light_dir);
    let ndotl = max(dot(n, l), 0.0);
    let ambient = 0.35;
    let lit = ambient + (1.0 - ambient) * ndotl;

    // Base color: textured or vertex color.
    var base_rgb: vec3<f32>;
    var base_a: f32;
    if (mesh_uniform.has_texture != 0u) {
        let tex = textureSample(mesh_texture, mesh_sampler, in.uv);
        base_rgb = tex.rgb;
        base_a = tex.a;
    } else {
        base_rgb = in.color;
        base_a = 1.0;
    }

    // Apply tint + opacity, then lighting.
    let tinted = vec4<f32>(base_rgb, base_a) * mesh_uniform.tint;
    let rgb = tinted.rgb * lit;
    return vec4<f32>(rgb, tinted.a * mesh_uniform.opacity);
}

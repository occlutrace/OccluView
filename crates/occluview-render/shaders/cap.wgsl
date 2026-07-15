// OccluView cap shader: draws the cut-surface cap polygon as a flat color.
//
// Used by the 3-pass stencil capping sequence. The cap quad is a
// 4-vertex strip lying in the clip plane, sized to cover the mesh bbox. It is
// drawn with stencil test = NotEqual(0) so only fragments inside the solid
// cross-section pass — the stencil was incremented/decremented by the prior
// back/front face passes.
//
// Vertex format: position only (vec3<f32>), no normals/colors/UVs. The color
// comes from a uniform.

struct Camera {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    light_dir: vec3<f32>,
    _pad0: f32,
    camera_pos: vec3<f32>,
    _pad1: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;

struct CapUniform {
    color: vec4<f32>,
}
@group(1) @binding(0) var<uniform> cap: CapUniform;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = camera.projection * camera.view * vec4<f32>(position, 1.0);
    return out;
}

@fragment
fn fs_main(_in: VertexOut) -> @location(0) vec4<f32> {
    return cap.color;
}

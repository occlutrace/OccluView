// OccluView mesh shader: studio-lit, vertex-color, or texture-mapped, with an
// optional per-fragment occlusal-contact ramp.
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
//                      has_texture flag + the contact ramp)
//   group 2 binding 0: texture_2d (optional; bound only when has_texture != 0)
//   group 2 binding 1: sampler    (optional; same)
//   group 2 binding 2: contact field, Rgba8Unorm holding one f32 bit pattern
//                      per texel; read with `textureLoad` in the vertex stage
//                      (never sampled, never filtered)
//
// The contact field is a signed distance in millimetres per vertex: positive is
// a gap to the opposing surface, zero is exact touch, negative is penetration
// depth. `vs_main` decodes it, the varying interpolates it, and `fs_main` looks
// the colour up per fragment — see `contact_ramp_color`.

const POINT_SPLAT_RADIUS_PX: f32 = 3.5;
const BACKFACE_INSPECTION_TINT: vec3<f32> = vec3<f32>(0.52, 0.60, 0.66);
// Flat neutral material shown when `show_vertex_colors` is off — a scan
// colored or textured but displayed as plain material. Must match
// `occluview_core::scene::material::DEFAULT_UNTEXTURED_MESH_TINT` (pinned by
// `neutral_material_matches_the_core_untextured_tint` in mesh_uniform.rs).
const NEUTRAL_MATERIAL_RGB: vec3<f32> = vec3<f32>(0.82, 0.68, 0.42);

// How much of the studio lighting a measured colour map keeps. Keep most of the
// uploaded hue at the screen: the heatmap is metrology, not a clay material.
const MEASURED_MAP_SHADE: f32 = 0.42;
// Extra form for a measured map, folded INTO the shared shading factor rather
// than added as a white highlight. A specular term would move the hue at every
// bright pixel, and a false-colour map is read by matching its hue against the
// legend — so the only thing the shader may do to it is scale all three
// channels together (pinned by `measured_map.rs`). Brightening the grazing
// edge inside that one factor is legal, and it is what makes a cusp, a groove,
// and a margin read as geometry instead of a coloured blob.
const MEASURED_MAP_FORM: f32 = 0.42;
// A scalar gloss term gives cusps a controlled highlight without adding white
// to the measured RGB channels and corrupting the deviation hue.
const MEASURED_MAP_GLOSS: f32 = 0.30;

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
    // The dental CAD "Show triangle orientation": paint back-facing fragments red.
    show_orientation: u32,
    // 0 = ignore scan color/texture, shade with NEUTRAL_MATERIAL_RGB instead.
    show_vertex_colors: u32,
    // 0 = do not sample an attached texture; vertex colors remain independent.
    show_texture: u32,
    // 1 = this layer shows a measured colour map (deviation heatmap).
    measured_map: u32,
    // 1 = this layer paints the occlusal contact field from `contact_stops`.
    contact_map: u32,
    // Texels per row of the packed field texture, so a vertex index becomes a
    // texture coordinate without `textureDimensions`.
    contact_field_width: f32,
    // (widest painted gap mm, far-edge feather mm, 0, 0)
    contact_gap: vec4<f32>,
    // (mm, L, a, b) per stop in Oklab, descending in mm — the ramp's own
    // numbers, produced by occluview_contact::stop_table on the CPU.
    contact_stops: array<vec4<f32>, 16>,
    // Stops in use; at least 1, so the `stop_count - 1` walk stays in range.
    contact_stop_count: u32,
    contact_padding_0: u32,
    contact_padding_1: u32,
    contact_padding_2: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> mesh_uniform: MeshUniform;

@group(2) @binding(0) var mesh_texture: texture_2d<f32>;
@group(2) @binding(1) var mesh_sampler: sampler;
// The packed signed field, one f32 bit pattern per texel. Loaded — never
// sampled — so no filtering can average a distance with a sentinel, and so the
// vertex stage may read it (textureSample with implicit derivatives is
// fragment-only; textureLoad is not).
@group(2) @binding(2) var contact_field: texture_2d<f32>;

// Cross-section clipping plane (group 3). When enabled, fragments on the
// "below" side of the plane (dot(world_pos, normal) - distance < 0) are
// discarded.
struct ClipPlane {
    normal: vec3<f32>,
    distance: f32,
    enabled: u32,
    _pad: u32,
}
@group(3) @binding(0) var<uniform> clip: ClipPlane;

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
    @location(3) world_pos: vec3<f32>,
    @location(4) splat_uv: vec2<f32>,
    @location(5) splat_enabled: f32,
    // Signed contact field in millimetres for this vertex, or 0 when the layer
    // paints no contact field. Interpolated: the FIELD is linear across a
    // triangle, so its value at a fragment is exact, and the non-linear ramp is
    // looked up from it per fragment rather than interpolated as a colour.
    @location(6) contact_mm: f32,
};

/// Decode this vertex's signed contact field, in millimetres, from the packed
/// field texture.
///
/// One f32 per texel, stored as the little-endian byte pattern across the RGBA
/// channels of an Rgba8Unorm texture: four-byte formats cannot be bound next to
/// a filtering sampler (`unfilterable-float`), so the value is smuggled through
/// a filterable one and unpacked by hand. `contact_field_width` turns the vertex
/// index into a coordinate, which keeps the shader free of
/// `textureDimensions` and lets the field be row-padded on the CPU.
fn contact_field_mm(vertex_index: u32) -> f32 {
    let width = u32(max(mesh_uniform.contact_field_width, 1.0));
    let x = vertex_index % width;
    let y = vertex_index / width;
    let packed = textureLoad(contact_field, vec2<i32>(i32(x), i32(y)), 0);
    let bits = u32(packed.r * 255.0 + 0.5)
        | (u32(packed.g * 255.0 + 0.5) << 8u)
        | (u32(packed.b * 255.0 + 0.5) << 16u)
        | (u32(packed.a * 255.0 + 0.5) << 24u);
    return bitcast<f32>(bits);
}

fn point_splat_corner(vertex_index: u32) -> vec2<f32> {
    let corner = vertex_index % 6u;
    if (corner == 0u) {
        return vec2<f32>(-1.0, -1.0);
    }
    if (corner == 1u) {
        return vec2<f32>(1.0, -1.0);
    }
    if (corner == 2u) {
        return vec2<f32>(-1.0, 1.0);
    }
    if (corner == 3u) {
        return vec2<f32>(-1.0, 1.0);
    }
    if (corner == 4u) {
        return vec2<f32>(1.0, -1.0);
    }
    return vec2<f32>(1.0, 1.0);
}

fn vertex_out(
    in: VertexIn,
    clip_pos: vec4<f32>,
    world_pos: vec4<f32>,
    splat_uv: vec2<f32>,
    splat_enabled: f32,
    contact_mm: f32,
) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = clip_pos;
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
    out.world_pos = world_pos.xyz;
    out.splat_uv = splat_uv;
    out.splat_enabled = splat_enabled;
    out.contact_mm = contact_mm;
    return out;
}

@vertex
fn vs_main(in: VertexIn, @builtin(vertex_index) vertex_index: u32) -> VertexOut {
    // World position via the per-mesh model matrix.
    let world_pos = mesh_uniform.model * vec4<f32>(in.position, 1.0);
    let clip_pos = camera.projection * camera.view * world_pos;
    // Guarded: a layer without contacts must not read a field texture bound for
    // some other layer's material.
    var contact_mm = 0.0;
    if (mesh_uniform.contact_map != 0u) {
        contact_mm = contact_field_mm(vertex_index);
    }
    return vertex_out(in, clip_pos, world_pos, vec2<f32>(0.0, 0.0), 0.0, contact_mm);
}

@vertex
fn vs_point_splat(in: VertexIn, @builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let world_pos = mesh_uniform.model * vec4<f32>(in.position, 1.0);
    let center_clip = camera.projection * camera.view * world_pos;
    let corner = point_splat_corner(vertex_index);
    let viewport = max(
        vec2<f32>(camera.point_viewport_width, camera.point_viewport_height),
        vec2<f32>(1.0, 1.0),
    );
    let ndc_radius = vec2<f32>(
        POINT_SPLAT_RADIUS_PX * 2.0 / viewport.x,
        POINT_SPLAT_RADIUS_PX * 2.0 / viewport.y,
    );
    let clip_offset = corner * ndc_radius * center_clip.w;
    let clip_pos = center_clip + vec4<f32>(clip_offset, 0.0, 0.0);
    // A point cloud never carries contacts (a point has no opposing surface to
    // measure along), so this pass reports the neutral value.
    return vertex_out(in, clip_pos, world_pos, corner, 1.0, 0.0);
}

// ---------------------------------------------------------------------------
// Occlusal contact ramp.
//
// The stop table arrives in Oklab from `occluview_contact::stop_table`, which
// its own tests pin against the CPU `ContactScale::color_at`; this is the same
// evaluation, in the same space, so a colour the legend shows is the colour the
// surface wears.
//
// Interpolating in Oklab rather than sRGB is what keeps a blue→cyan→green→
// yellow→red run vivid: the perceptual straight line has no neon band and no
// hue overshoot, while an sRGB lerp between the same stops passes through
// washed-out mud. The encode back to sRGB at the end is required, not
// decorative: the CPU hands back display-sRGB bytes and the whole pipeline
// writes raw values into an Rgba8Unorm target, so the shader has to arrive in
// the same space.
// ---------------------------------------------------------------------------

fn oklab_to_linear_contact(oklab: vec3<f32>) -> vec3<f32> {
    let l3 = oklab.x + 0.3963377774 * oklab.y + 0.2158037573 * oklab.z;
    let m3 = oklab.x - 0.1055613458 * oklab.y - 0.0638541728 * oklab.z;
    let s3 = oklab.x - 0.0894841775 * oklab.y - 1.291485548 * oklab.z;
    let l = l3 * l3 * l3;
    let m = m3 * m3 * m3;
    let s = s3 * s3 * s3;
    return clamp(
        vec3<f32>(
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
        ),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

fn linear_to_srgb_contact(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return clamp(select(hi, lo, c <= vec3<f32>(0.0031308)), vec3<f32>(0.0), vec3<f32>(1.0));
}

/// The ramp's colour at a signed field value, in display sRGB.
///
/// Walks the descending stop table, clamps outside it to the end stops, and
/// treats a zero-width span as its upper stop — a duplicate stop is a caller's
/// choice (the articulating-paper law carries its touch colour flat across a
/// 10 um measurement tolerance), not a division by zero.
fn contact_ramp_color(signed_mm: f32) -> vec3<f32> {
    let count = max(mesh_uniform.contact_stop_count, 1u);
    let first = mesh_uniform.contact_stops[0];
    let last = mesh_uniform.contact_stops[count - 1u];
    let value = clamp(signed_mm, last.x, first.x);
    var oklab = last.yzw;
    for (var i = 0u; i + 1u < count; i = i + 1u) {
        let high = mesh_uniform.contact_stops[i];
        let low = mesh_uniform.contact_stops[i + 1u];
        if (value > high.x || value < low.x) {
            continue;
        }
        let span = high.x - low.x;
        var t = 0.0;
        if (span > 0.0) {
            t = (high.x - value) / span;
        }
        oklab = mix(high.yzw, low.yzw, t);
        break;
    }
    return linear_to_srgb_contact(oklab_to_linear_contact(oklab));
}

@fragment
fn fs_main(
    in: VertexOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    var splat_coverage = 1.0;
    if (in.splat_enabled > 0.5) {
        let splat_dist = length(in.splat_uv);
        if (splat_dist > 1.0) {
            discard;
        }
        splat_coverage = 1.0 - smoothstep(0.72, 1.0, splat_dist);
    }
    // Cross-section: discard fragments below the clip plane.
    if (clip.enabled != 0u && dot(in.world_pos, clip.normal) - clip.distance < 0.0) {
        discard;
    }
    // Studio clay material. Dental scans need readable cusps/fissures without
    // heavy cast shadows, so use two-sided normals, soft key/fill/rim lighting,
    // and restrained highlights instead of a flat ambient wash.
    var n = in.normal;
    let n_len = length(n);
    if (n_len < 0.001) {
        n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        n = normalize(n);
    }
    var view_dir = camera.camera_pos - in.world_pos;
    let view_len = length(view_dir);
    if (view_len < 0.001) {
        view_dir = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        view_dir = normalize(view_dir);
    }
    // Two-sided shading: flip the normal to face the viewer so a grazing or
    // slightly inverted face still lights evenly instead of dropping into a
    // dark grazing wash (owner rule: even dental light, no cast shadows).
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }

    let key = normalize(camera.light_dir);
    let camera_fill = normalize(view_dir * 0.72 - key * 0.20);
    let ndotl = max(dot(n, key), 0.0);
    let wrapped_key = pow(clamp(ndotl * 0.66 + 0.34, 0.0, 1.0), 0.96);
    let fill_lit = pow(max(dot(n, camera_fill), 0.0), 0.82);
    let fresnel = pow(1.0 - clamp(dot(n, view_dir), 0.0, 1.0), 2.60);
    let rim_lit = pow(fresnel, 1.45);
    let half_vec = normalize(key + view_dir);
    let half_dot = max(dot(n, half_vec), 0.0);
    let tight_specular = pow(half_dot, 96.0);
    let broad_specular = pow(half_dot, 30.0);
    let view_form = pow(clamp(dot(n, view_dir), 0.0, 1.0), 0.62);
    // Form-giving studio light: a lit floor keeps every visible face out of a
    // cast shadow (two-sided flip above), while a full key/fill/rim swing
    // sculpts the side walls so curvature reads with real depth — the look the
    // owner called "great". The dark grazing WASH is what was wrong (removed
    // below, not here); flattening this coefficient set killed the depth.
    let lit = clamp(
        0.50 + 0.36 * wrapped_key + 0.095 * fill_lit + 0.018 * rim_lit,
        0.48,
        1.05,
    );

    // Base color: neutral material, textured, or vertex color.
    var base_rgb: vec3<f32>;
    var base_a: f32;
    if (mesh_uniform.show_vertex_colors == 0u) {
        base_rgb = NEUTRAL_MATERIAL_RGB;
        base_a = 1.0;
    } else if (mesh_uniform.has_texture != 0u && mesh_uniform.show_texture != 0u) {
        let tex = textureSample(mesh_texture, mesh_sampler, in.uv);
        base_rgb = tex.rgb;
        base_a = tex.a;
    } else {
        base_rgb = in.color;
        base_a = 1.0;
    }

    // A measured colour map keeps its hue and skips the tint, so a ramp reaches
    // the screen at the colour it was measured at. Lighting is REDUCED, not
    // removed: full studio light multiplies a saturated ramp down towards mud,
    // but no light at all leaves a flat silhouette with no readable form — and
    // a heat map you cannot read the shape of tells you nothing about a scan.
    if (mesh_uniform.measured_map != 0u) {
        let gloss = 0.75 * tight_specular + 0.25 * broad_specular;
        let map_form = clamp(
            lit + MEASURED_MAP_FORM * fresnel + MEASURED_MAP_GLOSS * gloss,
            0.78,
            1.10,
        );
        let shade = clamp(mix(1.0, map_form, MEASURED_MAP_SHADE), 0.96, 1.05);
        return vec4<f32>(base_rgb * shade, base_a * mesh_uniform.opacity * splat_coverage);
    }

    // Apply tint + opacity, then lighting.
    let tinted = vec4<f32>(base_rgb, base_a) * mesh_uniform.tint;
    let form_contrast = 0.96 + 0.055 * view_form + 0.018 * fresnel;
    // Neutral material reads as matte stone, never the textured glaze.
    let textured = mesh_uniform.has_texture != 0u
        && mesh_uniform.show_texture != 0u
        && mesh_uniform.show_vertex_colors != 0u;
    let texture_glaze = select(0.0, 1.0, textured);
    let clay_highlight = (1.0 - texture_glaze) * (0.018 * tight_specular + 0.008 * fresnel);
    let glaze_highlight =
        texture_glaze * (0.040 * tight_specular + 0.024 * broad_specular + 0.007 * fresnel);
    let highlight = vec3<f32>(clay_highlight + glaze_highlight);
    let lit_rgb = clamp(tinted.rgb * lit * form_contrast + highlight, vec3<f32>(0.0), vec3<f32>(1.0));
    // Genuine back-facing triangles get only a faint cool tint (not a dark
    // grey) so an inside-out surface stays distinguishable while the front
    // stays evenly lit — the loud "flipped normal" cue is the explicit
    // show_orientation diagnostic below, not an implicit half-shadow.
    let backface_mix = select(0.0, 0.14, !front_facing);
    var rgb = mix(lit_rgb, BACKFACE_INSPECTION_TINT * lit, backface_mix);
    // Orientation diagnostic: back-facing fragments render solid red so an
    // inside-out surface is unmistakable (the dental CAD "Show triangle
    // orientation" convention).
    if (mesh_uniform.show_orientation != 0u && !front_facing) {
        rgb = vec3<f32>(0.80, 0.10, 0.10);
    }

    // Occlusal contact paint, LAST and per fragment.
    //
    // Last, because a reading must not change how the scan looks where it marks
    // nothing: the surface is finished — tinted, lit, backface-corrected — and
    // the ramp is mixed over it. Painting into the base colour instead meant the
    // whole layer had to switch to the measured-map treatment to keep the ramp's
    // hue, which dropped the operator's tint and flattened the lighting across
    // the entire scan the moment a reading opened.
    //
    // Per fragment, because the field is linear across a triangle and the ramp
    // is not: interpolating the field and looking the colour up here keeps the
    // ramp's hues and puts the edge of the band exactly where the field crosses
    // it, with no smear and no washed-out rim.
    //
    // The paint ends by WEIGHT, never by fading toward white: a ramp that washes
    // out reads as a lighting artefact rather than as data. Lighting inside the
    // band is REDUCED rather than removed, at the same constants the deviation
    // heatmap uses, so a saturated ramp still shows the cusps it sits on.
    if (mesh_uniform.contact_map != 0u) {
        let far = mesh_uniform.contact_gap.x;
        let fade = mesh_uniform.contact_gap.y;
        let t = clamp((far - in.contact_mm) / max(fade, 1e-6), 0.0, 1.0);
        let weight = t * t * (3.0 - 2.0 * t);
        if (weight > 0.0) {
            let gloss = 0.75 * tight_specular + 0.25 * broad_specular;
            let map_form = clamp(
                lit + MEASURED_MAP_FORM * fresnel + MEASURED_MAP_GLOSS * gloss,
                0.78,
                1.10,
            );
            let shade = clamp(mix(1.0, map_form, MEASURED_MAP_SHADE), 0.96, 1.05);
            rgb = mix(rgb, contact_ramp_color(in.contact_mm) * shade, weight);
        }
    }
    return vec4<f32>(rgb, tinted.a * mesh_uniform.opacity * splat_coverage);
}

@fragment
fn fs_wireframe(in: VertexOut) -> @location(0) vec4<f32> {
    if (clip.enabled != 0u && dot(in.world_pos, clip.normal) - clip.distance < 0.0) {
        discard;
    }
    let tint_strength = clamp(max(max(mesh_uniform.tint.r, mesh_uniform.tint.g), mesh_uniform.tint.b), 0.0, 1.0);
    let graphite = vec3<f32>(0.08, 0.105, 0.12);
    let cool_edge = vec3<f32>(0.18, 0.23, 0.25);
    let rgb = mix(graphite, cool_edge, tint_strength * 0.35);
    return vec4<f32>(rgb, clamp(mesh_uniform.opacity * 0.68, 0.32, 0.72));
}

// Ghost pass for the cut view. The OWNER rule is that a cross-section must not
// remove geometry from view: the kept side draws opaque via `fs_main`, and this
// entry point re-draws the *cut-away* side (inverted clip test) as a faint,
// cool, semi-transparent shell so nothing ever fully disappears. Used only by
// the ghost pipeline (alpha-blended, depth-tested, no depth write).
const GHOST_ALPHA: f32 = 0.18;
const GHOST_COOL_TINT: vec3<f32> = vec3<f32>(0.82, 0.92, 1.08);

@fragment
fn fs_ghost(in: VertexOut) -> @location(0) vec4<f32> {
    // A ghost pass only means anything while clipping is active; with no cut
    // it draws nothing so a stray invocation is a harmless no-op.
    if (clip.enabled == 0u) {
        discard;
    }
    // Inverted clip test: keep the removed (below) side and discard the kept
    // side (already drawn opaque by fs_main). The conditions are complementary
    // (fs_main discards `< 0.0`, this discards `>= 0.0`), so the two passes
    // never shade the same fragment at the seam — no z-fighting.
    if (dot(in.world_pos, clip.normal) - clip.distance >= 0.0) {
        discard;
    }
    var n = in.normal;
    let n_len = length(n);
    if (n_len < 0.001) {
        n = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        n = normalize(n);
    }
    var view_dir = camera.camera_pos - in.world_pos;
    let view_len = length(view_dir);
    if (view_len < 0.001) {
        view_dir = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        view_dir = normalize(view_dir);
    }
    // Two-sided: face the normal toward the viewer so the shell shades evenly.
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }
    let ndotv = clamp(dot(n, view_dir), 0.0, 1.0);
    // Soft form term plus a grazing-angle rim so the shell reads as a solid
    // volume rather than a flat wash.
    let form = 0.55 + 0.35 * ndotv;
    let fresnel = pow(1.0 - ndotv, 2.4);
    // The mesh's own color: sample the texture for textured meshes (HPS
    // dental scans carry their color in the texture with a WHITE vertex color),
    // else the vertex color — exactly as `fs_main` picks its base. Ghosting the
    // vertex color alone paints a textured scan a flat cool-white shell that
    // reads as raw normal shading; sampling the texture keeps the ghost a faded
    // version of the REAL surface so it still reads as the removed half.
    var base_rgb: vec3<f32>;
    if (mesh_uniform.show_vertex_colors == 0u) {
        base_rgb = NEUTRAL_MATERIAL_RGB;
    } else if (mesh_uniform.has_texture != 0u && mesh_uniform.show_texture != 0u) {
        base_rgb = textureSample(mesh_texture, mesh_sampler, in.uv).rgb;
    } else {
        base_rgb = in.color;
    }
    let base = base_rgb * mesh_uniform.tint.rgb;
    let luma = dot(base, vec3<f32>(0.299, 0.587, 0.114));
    let desat = mix(base, vec3<f32>(luma), 0.6);
    let ghost_rgb = clamp(
        desat * GHOST_COOL_TINT * form + fresnel * 0.14,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    return vec4<f32>(ghost_rgb, GHOST_ALPHA);
}

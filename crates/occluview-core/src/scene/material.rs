use crate::mesh::Mesh;

/// Default display material for untextured dental scans: a warm Type IV stone
/// tint instead of neutral white.
pub const DEFAULT_UNTEXTURED_MESH_TINT: [f32; 4] = [0.82, 0.68, 0.42, 1.0];
/// Neutral tint for meshes that already carry scan color or texture data.
pub const DEFAULT_COLORED_MESH_TINT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

pub(super) fn default_mesh_tint(mesh: &Mesh) -> [f32; 4] {
    if mesh.texture().is_some() || mesh.has_vertex_colors() {
        DEFAULT_COLORED_MESH_TINT
    } else {
        DEFAULT_UNTEXTURED_MESH_TINT
    }
}

/// Approximate sRGB to linear conversion (no external color crate in `core`).
///
/// Used only for the default background tint; precise color management is an
/// explicit future concern.
pub(super) fn linear_srgb_from_srgb(srgb: [f32; 4]) -> [f32; 4] {
    let f = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(srgb[0]), f(srgb[1]), f(srgb[2]), srgb[3]]
}

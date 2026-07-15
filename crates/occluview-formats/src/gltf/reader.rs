use super::error::malformed;
use super::json;
use super::scene::{first_primitive_material, walk_node};
use super::texture::resolve_material_texture;
use crate::error::FormatError;
use glam::Mat4;
use occluview_core::{Mesh, MeshBuilder};

pub(super) fn read_doc(doc: &json::GltfDoc, bin_chunk: &[u8]) -> Result<Mesh, FormatError> {
    let scene_idx = doc.scene.unwrap_or(0);
    let scene = doc
        .scenes
        .get(scene_idx)
        .ok_or_else(|| malformed("scene out of range"))?;
    let mut builder = MeshBuilder::new().with_name("glTF");
    // Track the first primitive's material so we can resolve a texture after
    // the build (the builder only handles geometry).
    let mut first_material: Option<usize> = None;
    for &node_idx in &scene.nodes {
        walk_node(doc, node_idx, Mat4::IDENTITY, bin_chunk, &mut builder)?;
        if first_material.is_none() {
            first_material = first_primitive_material(doc, node_idx);
        }
    }
    let mut mesh = builder.build().map_err(FormatError::Core)?;
    // If the first primitive references a textured material, decode + attach.
    if let Some(mat_idx) = first_material {
        if let Some(tex) = resolve_material_texture(doc, mat_idx, bin_chunk)? {
            mesh.set_texture(tex);
        }
    }
    Ok(mesh)
}

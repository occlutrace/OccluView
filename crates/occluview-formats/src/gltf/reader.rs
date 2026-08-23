use super::error::malformed;
use super::json;
use super::scene::{first_primitive_material, SceneWalk, VisitedNodes};
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
    {
        let mut walk = SceneWalk::new(doc, bin_chunk, &mut builder);
        // One visit set for the whole search, not one per root: it used to be
        // allocated and zeroed inside the loop, which made a document with no
        // material anywhere cost a byte per node per root.
        let mut material_visited = VisitedNodes::for_document(doc);
        for &node_idx in &scene.nodes {
            walk.node(node_idx, Mat4::IDENTITY)?;
            if first_material.is_none() {
                first_material = first_primitive_material(doc, node_idx, &mut material_visited);
            }
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

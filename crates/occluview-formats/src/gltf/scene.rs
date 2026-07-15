use super::error::malformed;
use super::json;
use super::primitive::emit_primitive;
use crate::error::FormatError;
use glam::{Mat4, Quat, Vec3};
use occluview_core::MeshBuilder;

/// Return the material index of the first primitive of the mesh referenced by
/// `node_idx`, if any.
pub(super) fn first_primitive_material(doc: &json::GltfDoc, node_idx: usize) -> Option<usize> {
    let node = doc.nodes.get(node_idx)?;
    if let Some(mesh_idx) = node.mesh {
        let mesh = doc.meshes.get(mesh_idx)?;
        if let Some(material) = mesh.primitives.first()?.material {
            return Some(material);
        }
    }
    for &child_idx in &node.children {
        if let Some(material) = first_primitive_material(doc, child_idx) {
            return Some(material);
        }
    }
    None
}

pub(super) fn walk_node(
    doc: &json::GltfDoc,
    node_idx: usize,
    parent_transform: Mat4,
    bin_chunk: &[u8],
    builder: &mut MeshBuilder,
) -> Result<(), FormatError> {
    let node = doc
        .nodes
        .get(node_idx)
        .ok_or_else(|| malformed("node out of range"))?;
    let world_transform = parent_transform * node_transform(node)?;
    if let Some(mesh_idx) = node.mesh {
        let mesh = doc
            .meshes
            .get(mesh_idx)
            .ok_or_else(|| malformed("mesh out of range"))?;
        for prim in &mesh.primitives {
            emit_primitive(doc, prim, world_transform, bin_chunk, builder)?;
        }
    }
    for &child_idx in &node.children {
        walk_node(doc, child_idx, world_transform, bin_chunk, builder)?;
    }
    Ok(())
}

fn node_transform(node: &json::Node) -> Result<Mat4, FormatError> {
    if let Some(matrix) = &node.matrix {
        let cols: [f32; 16] = matrix
            .as_slice()
            .try_into()
            .map_err(|_| malformed("node matrix must have 16 elements"))?;
        return Ok(Mat4::from_cols_array(&cols));
    }

    let translation = Vec3::from_array(node.translation.unwrap_or([0.0, 0.0, 0.0]));
    let rotation = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let rotation = {
        let quat = Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]);
        if quat.length_squared() > 0.0 {
            quat.normalize()
        } else {
            Quat::IDENTITY
        }
    };
    let scale = Vec3::from_array(node.scale.unwrap_or([1.0, 1.0, 1.0]));

    Ok(Mat4::from_scale_rotation_translation(
        scale,
        rotation,
        translation,
    ))
}

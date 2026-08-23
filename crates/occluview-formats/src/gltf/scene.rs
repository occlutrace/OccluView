use super::error::malformed;
use super::json;
use super::primitive::emit_primitive;
use crate::error::FormatError;
use glam::{Mat4, Quat, Vec3};
use occluview_core::MeshBuilder;

/// Which nodes a single document traversal has already entered.
///
/// glTF requires the node hierarchy to be a strict tree: a node has at most
/// one parent and cycles are forbidden. Nothing in the file enforces that, and
/// `children` comes straight out of attacker-controlled JSON, so a node that
/// lists itself — or two nodes that list each other — recursed until the stack
/// was exhausted. A stack overflow is not a panic: it is a guard-page fault,
/// so none of the `catch_unwind` barriers around the COM entry points can
/// intercept it, and `.glb` is registered machine-wide for both the thumbnail
/// provider and the preview handler. One crafted file in a folder took down
/// the Explorer host and blanked every thumbnail around it.
///
/// Refusing a second entry also bounds the diamond case, where no cycle exists
/// but shared children multiply the traversal exponentially with depth.
pub(super) struct VisitedNodes(Vec<bool>);

impl VisitedNodes {
    pub(super) fn for_document(doc: &json::GltfDoc) -> Self {
        Self(vec![false; doc.nodes.len()])
    }

    /// Mark `node_idx` as entered, or report the document as malformed if it
    /// was entered before.
    fn enter(&mut self, node_idx: usize) -> Result<(), FormatError> {
        match self.0.get_mut(node_idx) {
            Some(seen) if *seen => Err(malformed("node graph is cyclic or shares a child node")),
            Some(seen) => {
                *seen = true;
                Ok(())
            }
            // Out of range is the caller's error to report with its own
            // message; leave it to the lookup that follows.
            None => Ok(()),
        }
    }
}

/// Return the material index of the first primitive of the mesh referenced by
/// `node_idx`, if any.
///
/// Walks the same hierarchy as [`walk_node`] and is bounded the same way, with
/// its own visit set: the two traversals are independent, and a graph that
/// [`walk_node`] already rejected never reaches this one.
pub(super) fn first_primitive_material(doc: &json::GltfDoc, node_idx: usize) -> Option<usize> {
    let mut visited = VisitedNodes::for_document(doc);
    first_primitive_material_from(doc, node_idx, &mut visited)
}

fn first_primitive_material_from(
    doc: &json::GltfDoc,
    node_idx: usize,
    visited: &mut VisitedNodes,
) -> Option<usize> {
    visited.enter(node_idx).ok()?;
    let node = doc.nodes.get(node_idx)?;
    if let Some(mesh_idx) = node.mesh {
        let mesh = doc.meshes.get(mesh_idx)?;
        if let Some(material) = mesh.primitives.first()?.material {
            return Some(material);
        }
    }
    for &child_idx in &node.children {
        if let Some(material) = first_primitive_material_from(doc, child_idx, visited) {
            return Some(material);
        }
    }
    None
}

/// One traversal of one document's node hierarchy.
///
/// The visit set lives here rather than in a parameter so it spans every root
/// of the scene: a node reached from two roots is the same violation as a
/// cycle, and just as unbounded.
pub(super) struct SceneWalk<'a> {
    doc: &'a json::GltfDoc,
    bin_chunk: &'a [u8],
    builder: &'a mut MeshBuilder,
    visited: VisitedNodes,
}

impl<'a> SceneWalk<'a> {
    pub(super) fn new(
        doc: &'a json::GltfDoc,
        bin_chunk: &'a [u8],
        builder: &'a mut MeshBuilder,
    ) -> Self {
        let visited = VisitedNodes::for_document(doc);
        Self {
            doc,
            bin_chunk,
            builder,
            visited,
        }
    }

    /// Emit `node_idx` and everything under it, placed by `parent_transform`.
    pub(super) fn node(
        &mut self,
        node_idx: usize,
        parent_transform: Mat4,
    ) -> Result<(), FormatError> {
        self.visited.enter(node_idx)?;
        let node = self
            .doc
            .nodes
            .get(node_idx)
            .ok_or_else(|| malformed("node out of range"))?;
        let world_transform = parent_transform * node_transform(node)?;
        if let Some(mesh_idx) = node.mesh {
            let mesh = self
                .doc
                .meshes
                .get(mesh_idx)
                .ok_or_else(|| malformed("mesh out of range"))?;
            for prim in &mesh.primitives {
                emit_primitive(
                    self.doc,
                    prim,
                    world_transform,
                    self.bin_chunk,
                    self.builder,
                )?;
            }
        }
        for &child_idx in &node.children {
            self.node(child_idx, world_transform)?;
        }
        Ok(())
    }
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

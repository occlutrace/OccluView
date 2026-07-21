//! Atomic scene replacement for an accepted Bridge Split preview.
//!
//! The source layer keeps its stable identity as Part A so file provenance,
//! layer selection, and structural history remain coherent. Part B is inserted
//! immediately after it with a fresh identity and the same presentation state.

use occluview_core::{CoreBridgeSplitResult, Scene, SceneMesh, SceneMeshId};

use super::BridgeSplitTarget;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BridgeSplitSceneApplyError {
    TargetUnavailable,
    TargetChanged,
    InvalidPreview,
}

pub(crate) struct BridgeSplitSceneResult {
    pub(crate) scene: Scene,
    pub(crate) source_layer_id: SceneMeshId,
    pub(crate) part_b_layer_id: SceneMeshId,
}

pub(crate) fn apply_preview_to_scene(
    scene: &Scene,
    target: BridgeSplitTarget,
    preview: &CoreBridgeSplitResult,
) -> Result<BridgeSplitSceneResult, BridgeSplitSceneApplyError> {
    if preview.part_a.triangle_count() == 0 || preview.part_b.triangle_count() == 0 {
        return Err(BridgeSplitSceneApplyError::InvalidPreview);
    }

    let mut draft = scene.clone();
    let Some(index) = draft
        .meshes()
        .iter()
        .position(|entry| entry.id() == target.layer_id)
    else {
        return Err(BridgeSplitSceneApplyError::TargetUnavailable);
    };
    let source = draft.meshes()[index].clone();
    if !source.visible || source.mesh.is_point_cloud() {
        return Err(BridgeSplitSceneApplyError::TargetUnavailable);
    }
    if BridgeSplitTarget::capture(&source) != target {
        return Err(BridgeSplitSceneApplyError::TargetChanged);
    }

    draft.meshes_mut()[index].mesh = preview.part_a.clone();
    let source_layer_id = source.id();
    let part_b = layer_like(&source, preview.part_b.clone());
    let part_b_layer_id = part_b.id();
    draft.insert(index + 1, part_b);

    Ok(BridgeSplitSceneResult {
        scene: draft,
        source_layer_id,
        part_b_layer_id,
    })
}

fn layer_like(source: &SceneMesh, mesh: occluview_core::Mesh) -> SceneMesh {
    let mut layer = SceneMesh::new(mesh)
        .with_transform(source.transform)
        .with_tint(source.tint)
        .with_opacity(source.opacity)
        .with_wireframe(source.wireframe)
        .with_show_vertex_colors(source.show_vertex_colors)
        .with_show_texture(source.show_texture);
    layer.visible = source.visible;
    layer.show_orientation = source.show_orientation;
    layer
}

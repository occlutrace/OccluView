//! A layer-pose transaction needs scene content and its history controller.
//! Installation and render/tool reconciliation remain with the scene owner.

use crate::edit_mode::{EditModeCommand, EditModeController};
use glam::Affine3A;
use occluview_core::{Scene, SceneMeshId};

pub(super) enum PoseMutation {
    Refused,
    Unchanged,
    Changed(Scene),
}

pub(super) fn stage_layer_pose(
    scene: &Scene,
    history: &mut EditModeController,
    moving: SceneMeshId,
    pose: Affine3A,
) -> PoseMutation {
    let Some(index) = scene.meshes().iter().position(|layer| layer.id() == moving) else {
        return PoseMutation::Refused;
    };
    if scene.meshes()[index].transform == pose {
        return PoseMutation::Unchanged;
    }
    let mut next = scene.clone();
    let Some(token) = history.begin_scene_edit(&next, moving, EditModeCommand::MoveLayer) else {
        return PoseMutation::Refused;
    };
    next.meshes_mut()[index].transform = pose;
    let _ = history.finish_scene_edit_success(token, &next);
    PoseMutation::Changed(next)
}

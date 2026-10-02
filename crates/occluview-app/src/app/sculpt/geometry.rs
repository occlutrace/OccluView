//! Hit testing and target resolution for the sculpt brushes.

use super::super::{egui, SceneContext};
use super::stroke::{local_clip_plane, local_ray_hit_is_visible};
use crate::sculpt::sculpt_tool::uniform_scene_scale;
use crate::viewer::viewport_ray;
use glam::{Quat, Vec3};
use occluview_core::{SceneMeshId, ScenePickHit};

impl SceneContext<'_> {
    pub(super) fn sculpt_surface_hit(
        &self,
        viewport_rect: egui::Rect,
        pointer: egui::Pos2,
    ) -> Option<ScenePickHit> {
        let camera = self.render.camera?;
        let scene = self.document.scene.as_ref()?;
        let layer_id = self.sculpt_target_layer_id(scene)?;
        let entry = scene.meshes().iter().find(|entry| entry.id() == layer_id)?;
        let (ray_origin, direction) = viewport_ray(&camera, viewport_rect, pointer)?;
        let direction = direction.normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let origin = ray_origin + direction * camera.near;
        let inverse = entry.transform.inverse();
        let local_origin = inverse.transform_point3(origin);
        let local_direction = inverse.transform_vector3(direction).normalize_or_zero();
        let worker = self.tools.sculpt.worker.as_ref().filter(|worker| {
            worker.layer_id == layer_id && worker.topology_id == entry.mesh.topology_id()
        });
        let local_per_world = worker.map_or_else(
            || Some(1.0 / uniform_scene_scale(&entry.transform)?),
            |w| Some(w.local_per_world),
        )?;
        let far_mm = (camera.far - camera.near) * local_per_world;
        let clip_plane = local_clip_plane(inverse, self.active_viewport_clip_plane(scene.bbox()));
        let keep = |point| {
            local_ray_hit_is_visible(point, local_origin, local_direction, far_mm, clip_plane)
        };
        // Preparation warms the shared tree off the UI thread.
        if worker.is_none() && !entry.mesh.bvh_is_ready() {
            return None;
        }
        // When the live tree is contended, defer this cursor frame instead of
        // showing a footprint on committed geometry under the live surface.
        let (triangle_index, local_point) = if let Some(worker) = worker {
            worker.pick_local_ray(local_origin, local_direction, keep)?
        } else {
            entry
                .mesh
                .pick_ray_local(local_origin, local_direction, keep)?
        };
        let point = entry.transform.transform_point3(local_point);
        let distance = (point - origin).dot(direction);
        (distance.is_finite() && distance >= 0.0 && distance <= camera.far - camera.near).then_some(
            ScenePickHit {
                layer_index: scene
                    .meshes()
                    .iter()
                    .position(|candidate| candidate.id() == layer_id)?,
                layer_id,
                triangle_index,
                point,
                distance,
            },
        )
    }

    pub(super) fn sculpt_target_layer_id(
        &self,
        scene: &occluview_core::Scene,
    ) -> Option<SceneMeshId> {
        sculpt_target(scene, self.document.edit_mode.session_layer_id())
            .map(|(_, layer_id)| layer_id)
    }
}

pub(super) fn orient_tool_axis(base: Quat, surface_normal: Vec3, requested_axis: Vec3) -> Quat {
    let target =
        (requested_axis - surface_normal * requested_axis.dot(surface_normal)).normalize_or_zero();
    let current = (base * Vec3::Y).normalize_or_zero();
    if !target.is_finite()
        || target.length_squared() <= f32::EPSILON
        || current.length_squared() <= f32::EPSILON
    {
        return base;
    }
    let angle = surface_normal
        .dot(current.cross(target))
        .atan2(current.dot(target));
    Quat::from_axis_angle(surface_normal, angle) * base
}

pub(super) fn sculpt_face_normal(
    scene: &occluview_core::Scene,
    hit: &ScenePickHit,
    camera: &occluview_core::Camera,
    live_local_normal: Option<Vec3>,
) -> Option<Vec3> {
    let entry = scene.meshes().get(hit.layer_index)?;
    if entry.id() != hit.layer_id {
        return None;
    }
    let local = live_local_normal.unwrap_or_else(|| {
        let base = hit.triangle_index.saturating_mul(3);
        let Some(indices) = entry.mesh.indices().get(base..base.saturating_add(3)) else {
            return Vec3::ZERO;
        };
        let vertex = |index: u32| {
            entry
                .mesh
                .vertices()
                .get(usize::try_from(index).ok()?)
                .map(|vertex| Vec3::from_array(vertex.position))
        };
        let (Some(a), Some(b), Some(c)) =
            (vertex(indices[0]), vertex(indices[1]), vertex(indices[2]))
        else {
            return Vec3::ZERO;
        };
        (b - a).cross(c - a).normalize_or_zero()
    });
    if !local.is_finite() || local.length_squared() <= f32::EPSILON {
        return None;
    }
    let determinant = entry.transform.matrix3.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let normal = entry
        .transform
        .matrix3
        .inverse()
        .transpose()
        .mul_vec3(local)
        .normalize_or_zero();
    if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
        return None;
    }
    let toward_camera = (camera.eye() - hit.point).normalize_or_zero();
    if toward_camera.length_squared() > f32::EPSILON && normal.dot(toward_camera) < 0.0 {
        Some(-normal)
    } else {
        Some(normal)
    }
}

pub(super) fn sculpt_target(
    scene: &occluview_core::Scene,
    preferred: Option<SceneMeshId>,
) -> Option<(usize, SceneMeshId)> {
    let valid = |entry: &occluview_core::SceneMesh| {
        entry.visible && !entry.mesh.is_point_cloud() && entry.mesh.triangle_count() > 0
    };
    preferred
        .and_then(|layer_id| {
            scene
                .meshes()
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.id() == layer_id && valid(entry))
                .map(|(index, _)| (index, layer_id))
        })
        .or_else(|| {
            scene
                .meshes()
                .iter()
                .enumerate()
                .find(|(_, entry)| valid(entry))
                .map(|(index, entry)| (index, entry.id()))
        })
}

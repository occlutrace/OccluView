use super::Scene;
use crate::Aabb;
use glam::{Affine3A, Vec3};

impl Scene {
    /// The aggregate axis-aligned bounding box of all **visible** meshes,
    /// each transformed by its `SceneMesh::transform`. Returns
    /// [`Aabb::EMPTY`] if there are no visible meshes.
    ///
    /// This is the framing box the camera should fit (upper + lower arch as
    /// one extent). Uses each mesh's constructor-time bbox cache so repaint
    /// and status updates do not walk every vertex.
    #[must_use]
    pub fn bbox(&self) -> Aabb {
        self.meshes
            .iter()
            .filter(|m| m.visible)
            .map(|entry| transform_bbox(entry.mesh.bbox_cached(), entry.transform))
            .fold(Aabb::EMPTY, Aabb::enclose_box)
    }
}

/// Transform an [`Aabb`] by an [`Affine3A`] and return the axis-aligned box
/// enclosing the 8 transformed corners. Rotation may grow the box (AABB of an
/// OBB), which is expected and conservative for framing.
fn transform_bbox(bbox: Aabb, t: Affine3A) -> Aabb {
    if bbox.is_empty() {
        return Aabb::EMPTY;
    }
    let corners = [
        bbox.min,
        Vec3::new(bbox.min.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.min.z),
        bbox.max,
    ];
    corners
        .into_iter()
        .map(|c| t.transform_point3(c))
        .fold(Aabb::EMPTY, Aabb::enclose_point)
}

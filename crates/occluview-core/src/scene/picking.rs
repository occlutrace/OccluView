use super::{Scene, SceneMesh, SceneMeshId};
use glam::Vec3;

/// Nearest triangle hit returned by scene picking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenePickHit {
    /// Index into [`Scene::meshes`] at the time of the pick.
    pub layer_index: usize,
    /// Stable layer identity used to reject stale edit operations.
    pub layer_id: SceneMeshId,
    /// Triangle index inside the picked mesh (`indices` chunk index).
    pub triangle_index: usize,
    /// World-space hit position.
    pub point: Vec3,
    /// Positive ray distance from the origin to `point`.
    pub distance: f32,
}

impl Scene {
    /// Pick the nearest visible triangle hit by a world-space ray.
    ///
    /// Returns `None` for point clouds, hidden meshes, degenerate triangles,
    /// invalid rays, or misses. Each mesh is picked through its own lazily-built
    /// cached BVH, so this is O(log n) per mesh and cheap enough to run every
    /// frame under an interactive sculpt cursor.
    #[must_use]
    pub fn pick_ray(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        self.pick_ray_hit(origin, direction).map(|hit| hit.point)
    }

    /// Pick the nearest visible triangle hit by a world-space ray and return
    /// the scene/layer identity needed by mesh-edit selection tools.
    ///
    /// Returns `None` for point clouds, hidden meshes, degenerate triangles,
    /// invalid rays, or misses. O(log n) per mesh via each mesh's cached BVH.
    #[must_use]
    pub fn pick_ray_hit(&self, origin: Vec3, direction: Vec3) -> Option<ScenePickHit> {
        self.pick_ray_hit_with(origin, direction, |_| true)
    }

    /// Pick a visible triangle on one stable layer, ignoring every other layer.
    ///
    /// Interactive tools use this when their target was chosen before pointer
    /// placement. A nearer scan must not steal a placement intended for the
    /// selected layer. Hidden, point-cloud, stale, or invalid targets return
    /// `None` under the same ray rules as [`Self::pick_ray_hit`].
    #[must_use]
    pub fn pick_layer_ray_hit(
        &self,
        origin: Vec3,
        direction: Vec3,
        layer_id: SceneMeshId,
    ) -> Option<ScenePickHit> {
        let direction = direction.normalize_or_zero();
        if !origin.is_finite() || direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let (layer_index, entry) = self
            .meshes
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.id() == layer_id)?;
        if !entry.visible || entry.mesh.is_point_cloud() {
            return None;
        }
        pick_mesh_ray(layer_index, entry, origin, direction, &|_| true)
    }

    /// Shared ray traversal: nearest visible triangle hit whose point satisfies
    /// `keep`, so the ray math is written once.
    fn pick_ray_hit_with<K>(&self, origin: Vec3, direction: Vec3, keep: K) -> Option<ScenePickHit>
    where
        K: Fn(Vec3) -> bool,
    {
        let direction = direction.normalize_or_zero();
        if !origin.is_finite() || direction.length_squared() <= f32::EPSILON {
            return None;
        }

        self.meshes
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.visible && !entry.mesh.is_point_cloud())
            .filter_map(|(layer_index, entry)| {
                pick_mesh_ray(layer_index, entry, origin, direction, &keep)
            })
            .min_by(|left, right| left.distance.total_cmp(&right.distance))
    }
}

fn pick_mesh_ray<K>(
    layer_index: usize,
    entry: &SceneMesh,
    origin: Vec3,
    direction: Vec3,
    keep: &K,
) -> Option<ScenePickHit>
where
    K: Fn(Vec3) -> bool,
{
    // Ray-pick in the mesh's own local space via its cached BVH (O(log n)),
    // then lift the hit back to world. The `keep` predicate is world-space, so
    // wrap it to transform each local candidate point before testing.
    let inverse = entry.transform.inverse();
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);
    let transform = entry.transform;
    let (triangle_index, local_point) =
        entry
            .mesh
            .pick_ray_local(local_origin, local_direction, |local| {
                keep(transform.transform_point3(local))
            })?;
    let point = transform.transform_point3(local_point);
    // World distance along the (unit) world ray direction — robust to any scale
    // baked into the transform, and the value `min_by` compares across layers.
    let distance = (point - origin).dot(direction);
    Some(ScenePickHit {
        layer_index,
        layer_id: entry.id(),
        triangle_index,
        point,
        distance,
    })
}

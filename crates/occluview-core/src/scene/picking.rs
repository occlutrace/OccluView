use super::{Scene, SceneMesh, SceneMeshId};
use glam::Vec3;
use occlu_mesh_edit::SectionPlane;

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

    /// Pick the nearest visible hit that lies on the kept side of an optional
    /// cut plane. When `plane` is `None` this is exactly [`Self::pick_ray_hit`].
    ///
    /// With a plane, hits on the clipped-away side (`n·p - d < 0`, matching the
    /// renderer's fragment discard) are ignored, so a pick through the cut
    /// returns the first surface actually visible instead of hidden geometry.
    #[must_use]
    pub fn pick_scene_hit_clipped(
        &self,
        origin: Vec3,
        direction: Vec3,
        plane: Option<SectionPlane>,
    ) -> Option<ScenePickHit> {
        match plane {
            None => self.pick_ray_hit(origin, direction),
            Some(plane) => self.pick_ray_hit_with(origin, direction, move |point| {
                plane.signed_distance(point) >= 0.0
            }),
        }
    }

    /// Shared ray traversal: nearest visible triangle hit whose point satisfies
    /// `keep`. Both the plain and clipped picks funnel through here so the ray
    /// math is written once.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Mesh, Vertex};
    use crate::scene::{Scene, SceneMesh};

    /// A unit square sheet in the z = `z` plane, spanning x,y in [0, 1].
    fn sheet(z: f32) -> Mesh {
        Mesh::new(
            None,
            vec![
                Vertex::at(Vec3::new(0.0, 0.0, z)),
                Vertex::at(Vec3::new(1.0, 0.0, z)),
                Vertex::at(Vec3::new(1.0, 1.0, z)),
                Vertex::at(Vec3::new(0.0, 1.0, z)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("sheet")
    }

    #[test]
    fn clipped_pick_skips_the_clipped_side_and_returns_the_next_hit() {
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(sheet(5.0))); // near surface
        scene.add(SceneMesh::new(sheet(0.0))); // far surface
        let origin = Vec3::new(0.5, 0.5, 10.0);
        let direction = -Vec3::Z;

        // Without a plane, the near surface is picked.
        let plain = scene.pick_ray_hit(origin, direction).expect("hit");
        assert!((plain.point.z - 5.0).abs() < 1e-5);

        // A plane that clips the near surface returns the far one.
        let plane = SectionPlane::new(-Vec3::Z, -2.0).expect("plane");
        let clipped = scene
            .pick_scene_hit_clipped(origin, direction, Some(plane))
            .expect("kept-side hit");
        assert!(clipped.point.z.abs() < 1e-5);

        // A `None` plane behaves exactly like `pick_ray_hit`.
        let passthrough = scene
            .pick_scene_hit_clipped(origin, direction, None)
            .expect("hit");
        assert_eq!(passthrough, plain);
    }

    /// Closed UV sphere of `radius` about the origin.
    fn uv_sphere(radius: f32, nlat: usize, nlon: usize) -> Mesh {
        let mut verts = Vec::new();
        for a in 0..=nlat {
            let lat = std::f32::consts::PI * (a as f32 / nlat as f32);
            for o in 0..nlon {
                let lon = std::f32::consts::TAU * (o as f32 / nlon as f32);
                verts.push(Vertex::at(Vec3::new(
                    radius * lat.sin() * lon.cos(),
                    radius * lat.sin() * lon.sin(),
                    radius * lat.cos(),
                )));
            }
        }
        let mut idx = Vec::new();
        for a in 0..nlat {
            for o in 0..nlon {
                let o2 = (o + 1) % nlon;
                let p00 = (a * nlon + o) as u32;
                let p01 = (a * nlon + o2) as u32;
                let p10 = ((a + 1) * nlon + o) as u32;
                let p11 = ((a + 1) * nlon + o2) as u32;
                idx.extend_from_slice(&[p00, p10, p11, p00, p11, p01]);
            }
        }
        Mesh::new(None, verts, idx).expect("sphere")
    }

    #[test]
    fn clipped_pick_two_shells_returns_the_visible_surface_per_side() {
        // Nested shells: outer sphere r=10 (layer 0), inner sphere r=6 (layer 1).
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(uv_sphere(10.0, 48, 64))); // outer
        scene.add(SceneMesh::new(uv_sphere(6.0, 48, 64))); // inner
        let down = -Vec3::Z;

        // Keep z <= 0: normal -Z, distance 0 -> keep (-Z)·p - 0 >= 0 <=> z <= 0.
        let clip_top = SectionPlane::new(-Vec3::Z, 0.0).expect("plane");

        // A ray inside the inner radius: the outer top is clipped away, so the
        // pick lands on the INNER surface on the kept (lower) side.
        let inside = Vec3::new(2.0, 0.0, 20.0);
        let hit_inner = scene
            .pick_scene_hit_clipped(inside, down, Some(clip_top))
            .expect("inner hit");
        assert_eq!(hit_inner.layer_index, 1, "expected the inner shell");
        assert!(
            hit_inner.point.z < 0.0,
            "inner hit must be on the kept side"
        );
        // Faceted sphere: allow tessellation error around the analytic surface.
        let expect_inner_z = -(36.0_f32 - 4.0).sqrt(); // -sqrt(r^2 - x^2)
        assert!(
            (hit_inner.point.z - expect_inner_z).abs() < 5e-2,
            "inner z {} != {expect_inner_z}",
            hit_inner.point.z
        );

        // A ray between the two radii misses the inner sphere entirely, so with
        // the same clip it returns the OUTER surface on the kept side.
        let between = Vec3::new(8.0, 0.0, 20.0);
        let hit_outer = scene
            .pick_scene_hit_clipped(between, down, Some(clip_top))
            .expect("outer hit");
        assert_eq!(hit_outer.layer_index, 0, "expected the outer shell");
        let expect_outer_z = -(100.0_f32 - 64.0).sqrt();
        assert!(
            (hit_outer.point.z - expect_outer_z).abs() < 5e-2,
            "outer z {} != {expect_outer_z}",
            hit_outer.point.z
        );
        assert!(hit_outer.point.z <= 0.0, "hit must be on the kept side");

        // Without clipping, the same inside ray hits the outer top first.
        let unclipped = scene.pick_ray_hit(inside, down).expect("hit");
        assert_eq!(unclipped.layer_index, 0);
        assert!(unclipped.point.z > 0.0, "outer top is on the +z side");
    }
}

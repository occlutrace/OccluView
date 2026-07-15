//! Scene-level section assembly and caching.
//!
//! [`SceneSection::compute`] runs the [`occlu_mesh_edit`] plane–mesh kernel over
//! every included triangle layer and maps each layer's contour into world space
//! using that layer's `Affine3A` transform in `f64`. Point-cloud layers carry no
//! faces and contribute no contour. [`SectionCache`] memoizes the result so
//! camera motion never recomputes it; only geometry, transform, visibility, or
//! plane changes invalidate it.

use super::{Scene, SceneMesh, SceneMeshId};
use glam::{Affine3A, DAffine3, DMat3};
use occlu_mesh_edit::{plane_section, SectionPlane, SectionPolyline};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Which layers a section should include.
pub enum VisibilityFilter<'a> {
    /// Use each layer's own `visible` flag.
    SceneVisibility,
    /// Include only layers whose id is in this set (point clouds still skipped).
    Ids(&'a BTreeSet<SceneMeshId>),
}

impl VisibilityFilter<'_> {
    /// Whether `entry` contributes a contour. Never true for point clouds.
    fn includes(&self, entry: &SceneMesh) -> bool {
        if entry.mesh.is_point_cloud() {
            return false;
        }
        match self {
            Self::SceneVisibility => entry.visible,
            Self::Ids(ids) => ids.contains(&entry.id()),
        }
    }
}

/// The section contour of one scene layer, in world space (`f64`).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerSection {
    /// Stable identity of the source layer.
    pub layer_id: SceneMeshId,
    /// World-space contour polylines for this layer.
    pub polylines: Vec<SectionPolyline>,
}

/// Section contours for every intersected, included layer of a scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSection {
    /// Per-layer contours in scene draw order; layers with no contour omitted.
    pub per_layer: Vec<LayerSection>,
}

impl SceneSection {
    /// Compute world-space section contours for every included layer.
    #[must_use]
    pub fn compute(scene: &Scene, plane: SectionPlane, visible: &VisibilityFilter<'_>) -> Self {
        let mut per_layer = Vec::new();
        for entry in scene.meshes() {
            if !visible.includes(entry) {
                continue;
            }
            let Some(local) = world_plane_to_local(plane, &entry.transform) else {
                continue;
            };
            let positions = layer_positions(entry);
            let result = plane_section(&positions, entry.mesh.indices(), local);
            if result.polylines.is_empty() {
                continue;
            }
            per_layer.push(LayerSection {
                layer_id: entry.id(),
                polylines: to_world(result.polylines, &entry.transform),
            });
        }
        Self { per_layer }
    }
}

/// Collect a layer's vertex positions for the kernel.
fn layer_positions(entry: &SceneMesh) -> Vec<[f32; 3]> {
    entry.mesh.vertices().iter().map(|v| v.position).collect()
}

/// Map the world plane `n·x = d` into `transform`'s local space.
///
/// For `x = M·p + t`, `n·x = d` becomes `(Mᵀn)·p = d - n·t`; the local normal
/// is renormalized so the result stays a valid unit-normal plane under scaled
/// transforms. Returns `None` for a degenerate (zero-scale) transform.
///
/// The pull-back is computed in `f64`: `d - n·t` cancels catastrophically when
/// the layer sits far from the origin (both terms grow with the translation),
/// and an `f32` subtraction there would shift the local plane by a visible
/// amount (~0.1 unit at a 1e6 offset). This matches the module's `f64`
/// arithmetic contract and the `f64` forward map in [`to_world`].
#[allow(clippy::cast_possible_truncation)] // f64 pull-back stored back into the f32 SectionPlane
fn world_plane_to_local(plane: SectionPlane, transform: &Affine3A) -> Option<SectionPlane> {
    let normal = plane.normal.as_dvec3();
    let m = transform.matrix3;
    let matrix = DMat3::from_cols(
        m.x_axis.as_dvec3(),
        m.y_axis.as_dvec3(),
        m.z_axis.as_dvec3(),
    );
    let local_normal = matrix.transpose() * normal;
    let local_distance = f64::from(plane.distance) - normal.dot(transform.translation.as_dvec3());
    let length = local_normal.length();
    if !length.is_finite() || length < 1.0e-6 {
        return None;
    }
    SectionPlane::new(
        (local_normal / length).as_vec3(),
        (local_distance / length) as f32,
    )
    .ok()
}

/// Apply a layer transform to its contour polylines in `f64`.
fn to_world(polylines: Vec<SectionPolyline>, transform: &Affine3A) -> Vec<SectionPolyline> {
    let affine = daffine3(transform);
    polylines
        .into_iter()
        .map(|mut line| {
            for point in &mut line.points {
                *point = affine.transform_point3(*point);
            }
            line
        })
        .collect()
}

/// Promote an `Affine3A` to a double-precision affine transform.
fn daffine3(transform: &Affine3A) -> DAffine3 {
    let m = transform.matrix3;
    let matrix = DMat3::from_cols(
        m.x_axis.as_dvec3(),
        m.y_axis.as_dvec3(),
        m.z_axis.as_dvec3(),
    );
    DAffine3::from_mat3_translation(matrix, transform.translation.as_dvec3())
}

/// Content fingerprint of the inputs `SceneSection::compute` consumes.
///
/// Keyed on the plane bits and, per included layer, the stable layer id, the
/// mesh `topology_id` (geometry identity), and the transform bits. This is a
/// sound value key: it recomputes on geometry/transform/visibility changes and
/// reuses on material-only changes (which preserve `topology_id`).
#[derive(Clone, PartialEq, Eq)]
struct SectionKey {
    plane: [u32; 4],
    layers: Vec<LayerFingerprint>,
}

/// Per-layer fingerprint: identity, geometry id, and transform.
#[derive(Clone, PartialEq, Eq)]
struct LayerFingerprint {
    id: u64,
    topology: u64,
    transform: [u32; 12],
}

/// Build the content fingerprint for these inputs.
fn section_key(scene: &Scene, plane: SectionPlane, visible: &VisibilityFilter<'_>) -> SectionKey {
    let plane = [
        plane.normal.x.to_bits(),
        plane.normal.y.to_bits(),
        plane.normal.z.to_bits(),
        plane.distance.to_bits(),
    ];
    let layers = scene
        .meshes()
        .iter()
        .filter(|entry| visible.includes(entry))
        .map(|entry| LayerFingerprint {
            id: entry.id().get(),
            topology: entry.mesh.topology_id(),
            transform: affine_bits(&entry.transform),
        })
        .collect();
    SectionKey { plane, layers }
}

/// Raw bit patterns of an `Affine3A` (three matrix columns + translation).
fn affine_bits(transform: &Affine3A) -> [u32; 12] {
    let m = transform.matrix3;
    let cols = [
        m.x_axis.to_array(),
        m.y_axis.to_array(),
        m.z_axis.to_array(),
        transform.translation.to_array(),
    ];
    let mut bits = [0u32; 12];
    for (col, chunk) in cols.iter().zip(bits.chunks_exact_mut(3)) {
        for (value, slot) in col.iter().zip(chunk) {
            *slot = value.to_bits();
        }
    }
    bits
}

/// Memoizes the last computed [`SceneSection`] keyed on its content fingerprint.
#[derive(Default)]
pub struct SectionCache {
    cached: Option<(SectionKey, Arc<SceneSection>)>,
}

impl SectionCache {
    /// Construct an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached section for these inputs, computing it on a miss.
    pub fn get_or_compute(
        &mut self,
        scene: &Scene,
        plane: SectionPlane,
        visible: &VisibilityFilter<'_>,
    ) -> Arc<SceneSection> {
        let key = section_key(scene, plane, visible);
        if let Some((cached_key, cached)) = &self.cached {
            if *cached_key == key {
                return Arc::clone(cached);
            }
        }
        let section = Arc::new(SceneSection::compute(scene, plane, visible));
        self.cached = Some((key, Arc::clone(&section)));
        section
    }

    /// Drop any cached section (e.g. on scene clear).
    pub fn clear(&mut self) {
        self.cached = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Mesh, Vertex};
    use crate::scene::{Scene, SceneMesh};
    use glam::{DVec3, Vec3};

    /// Closed unit cube [0,1]^3 with two triangles per face.
    fn cube() -> Mesh {
        let corner = |x: f32, y: f32, z: f32| Vertex::at(Vec3::new(x, y, z));
        let vertices = vec![
            corner(0.0, 0.0, 0.0),
            corner(1.0, 0.0, 0.0),
            corner(1.0, 1.0, 0.0),
            corner(0.0, 1.0, 0.0),
            corner(0.0, 0.0, 1.0),
            corner(1.0, 0.0, 1.0),
            corner(1.0, 1.0, 1.0),
            corner(0.0, 1.0, 1.0),
        ];
        let indices = vec![
            0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 5, 1, 0, 4, 5, 3, 2, 6, 3, 6, 7, 0, 3, 7, 0, 7,
            4, 1, 5, 6, 1, 6, 2,
        ];
        Mesh::new(None, vertices, indices).expect("valid cube")
    }

    fn xplane(distance: f32) -> SectionPlane {
        SectionPlane::new(Vec3::X, distance).expect("unit normal")
    }

    fn scene_with(mesh: Mesh) -> Scene {
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        scene
    }

    #[test]
    fn cube_layer_yields_one_closed_contour() {
        let scene = scene_with(cube());
        let section =
            SceneSection::compute(&scene, xplane(0.5), &VisibilityFilter::SceneVisibility);
        assert_eq!(section.per_layer.len(), 1);
        let layer = &section.per_layer[0];
        assert_eq!(layer.polylines.len(), 1);
        assert!(layer.polylines[0].closed);
        assert!(layer.polylines[0].points.iter().all(|p| p.x == 0.5));
    }

    #[test]
    fn translated_layer_translates_the_contour() {
        let base = SceneSection::compute(
            &scene_with(cube()),
            xplane(0.5),
            &VisibilityFilter::SceneVisibility,
        );

        let shift = Vec3::new(3.0, -2.0, 1.5);
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(cube()).with_transform(Affine3A::from_translation(shift)));
        // Translate the plane by n·shift so the contour rides along with it.
        let moved = SceneSection::compute(
            &scene,
            xplane(0.5 + Vec3::X.dot(shift)),
            &VisibilityFilter::SceneVisibility,
        );

        assert_eq!(base.per_layer.len(), 1);
        assert_eq!(moved.per_layer.len(), 1);
        let delta = DVec3::new(f64::from(shift.x), f64::from(shift.y), f64::from(shift.z));
        let base_lines = &base.per_layer[0].polylines;
        let moved_lines = &moved.per_layer[0].polylines;
        assert_eq!(base_lines.len(), moved_lines.len());
        for (b, m) in base_lines.iter().zip(moved_lines) {
            assert_eq!(b.points.len(), m.points.len());
            for (bp, mp) in b.points.iter().zip(&m.points) {
                assert_eq!(*mp, *bp + delta);
            }
        }
    }

    #[test]
    fn point_cloud_layers_are_skipped() {
        let cloud = Mesh::point_cloud(None, vec![Vertex::at(Vec3::ZERO), Vertex::at(Vec3::X)]);
        let section = SceneSection::compute(
            &scene_with(cloud),
            xplane(0.5),
            &VisibilityFilter::SceneVisibility,
        );
        assert!(section.per_layer.is_empty());
    }

    #[test]
    fn cache_reuses_until_geometry_transform_or_plane_changes() {
        let scene = scene_with(cube());
        let mut cache = SectionCache::new();
        let first = cache.get_or_compute(&scene, xplane(0.5), &VisibilityFilter::SceneVisibility);
        let second = cache.get_or_compute(&scene, xplane(0.5), &VisibilityFilter::SceneVisibility);
        assert!(
            Arc::ptr_eq(&first, &second),
            "unchanged inputs reuse the cache"
        );

        // Transform change invalidates.
        let mut moved = scene.clone();
        moved.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::X);
        let third = cache.get_or_compute(&moved, xplane(0.5), &VisibilityFilter::SceneVisibility);
        assert!(!Arc::ptr_eq(&first, &third));

        // Plane change invalidates.
        let fourth = cache.get_or_compute(&scene, xplane(0.6), &VisibilityFilter::SceneVisibility);
        assert!(!Arc::ptr_eq(&third, &fourth));
    }

    #[test]
    fn cache_reuses_across_material_only_change() {
        let scene = scene_with(cube());
        let mut cache = SectionCache::new();
        let first = cache.get_or_compute(&scene, xplane(0.5), &VisibilityFilter::SceneVisibility);
        // Re-tint the layer (clones the mesh, preserving topology_id).
        let mut retinted = scene.clone();
        retinted.meshes_mut()[0].tint = [0.1, 0.2, 0.3, 1.0];
        let second =
            cache.get_or_compute(&retinted, xplane(0.5), &VisibilityFilter::SceneVisibility);
        assert!(Arc::ptr_eq(&first, &second), "material-only change reuses");
    }

    #[test]
    fn far_affine_layer_contour_stays_on_the_world_plane() {
        // Regression for the f64 world→local plane pull-back. A rotated,
        // non-uniformly scaled, far-translated layer must still yield a contour
        // that lies on the world section plane. Computed in f32, the `d - n·t`
        // term cancels catastrophically and the contour drifts ~0.1 unit
        // off-plane; in f64 it stays exact.
        use glam::Quat;
        let scale = Vec3::new(3.0, 7.0, 2.0);
        let rot = Quat::from_axis_angle(Vec3::new(0.3, 0.8, 0.5).normalize(), 0.9);
        let trans = Vec3::new(1.0e6, -2.0e6, 3.0e6);
        let transform = Affine3A::from_scale_rotation_translation(scale, rot, trans);
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(cube()).with_transform(transform));

        let n = Vec3::new(0.4, 0.5, 0.768_221).normalize();
        // Cut through the transformed cube centroid (local (0.5, 0.5, 0.5)).
        let world_centroid = transform.transform_point3(Vec3::splat(0.5));
        let d = n.dot(world_centroid);
        let section = SceneSection::compute(
            &scene,
            SectionPlane::new(n, d).expect("unit normal"),
            &VisibilityFilter::SceneVisibility,
        );
        assert_eq!(section.per_layer.len(), 1);
        let nd = n.as_dvec3();
        let dd = f64::from(d);
        let mut max_off = 0.0_f64;
        for line in &section.per_layer[0].polylines {
            assert!(!line.points.is_empty());
            for p in &line.points {
                max_off = max_off.max((nd.dot(*p) - dd).abs());
            }
        }
        assert!(
            max_off < 1.0e-3,
            "contour drifted {max_off} off the world plane (f32 cancellation regression)"
        );
    }
}

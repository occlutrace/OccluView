use super::*;
use crate::mesh::{Mesh, MeshTexture, Vertex};
use glam::{Affine3A, Vec3};
use std::path::PathBuf;

fn source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(path).unwrap_or_default()
}

fn tri() -> Mesh {
    Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh")
}

fn colored_tri() -> Mesh {
    Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO).with_color([120, 80, 70, 255]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)).with_color([121, 81, 71, 255]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)).with_color([122, 82, 72, 255]),
        ],
        vec![0, 1, 2],
    )
    .expect("valid colored mesh")
}

fn textured_tri() -> Mesh {
    let mut mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO).with_uv([0.0, 0.0]),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)).with_uv([1.0, 0.0]),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)).with_uv([0.0, 1.0]),
        ],
        vec![0, 1, 2],
    )
    .expect("valid textured mesh");
    mesh.set_texture(MeshTexture::new(1, 1, vec![120, 80, 70, 255]));
    mesh
}

fn two_triangle_mesh() -> Mesh {
    Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(3.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(2.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 3, 4, 5],
    )
    .expect("valid two-triangle mesh")
}

#[test]
fn scene_module_is_split_by_responsibility_not_single_file() {
    let facade = source_file("src/scene/mod.rs");
    let id = source_file("src/scene/id.rs");
    let material = source_file("src/scene/material.rs");
    let mesh_entry = source_file("src/scene/mesh_entry.rs");
    let graph = source_file("src/scene/graph.rs");
    let bounds = source_file("src/scene/bounds.rs");
    let picking = source_file("src/scene/picking.rs");

    assert!(
        facade.contains("mod bounds;")
            && facade.contains("mod graph;")
            && facade.contains("mod id;")
            && facade.contains("mod material;")
            && facade.contains("mod mesh_entry;")
            && facade.contains("mod picking;"),
        "scene should be a private module directory split by graph, layer, bounds, and picking responsibilities"
    );
    assert!(
        facade.contains("pub struct Scene")
            && facade.contains("pub use id::SceneMeshId;")
            && facade.contains("pub use mesh_entry::SceneMesh;"),
        "scene facade should keep the public core API stable"
    );
    assert!(
        id.contains("pub struct SceneMeshId")
            && material.contains("DEFAULT_UNTEXTURED_MESH_TINT")
            && mesh_entry.contains("pub struct SceneMesh")
            && graph.contains("pub fn append_scene")
            && bounds.contains("pub fn bbox(&self) -> Aabb")
            && picking.contains("pub fn pick_ray(&self"),
        "scene responsibilities should live in focused modules"
    );
}

#[test]
fn scene_public_surface_stays_reexported_from_core_root_and_scene_module() {
    use crate::{
        Scene as RootScene, SceneMesh as RootSceneMesh, SceneMeshId as RootSceneMeshId,
        DEFAULT_UNTEXTURED_MESH_TINT as ROOT_DEFAULT_UNTEXTURED_MESH_TINT,
    };

    let mut scene = RootScene::new();
    let layer = RootSceneMesh::new(Mesh::empty());
    let layer_id: RootSceneMeshId = layer.id();
    let _ = layer_id.get();

    scene.add(layer.with_tint(ROOT_DEFAULT_UNTEXTURED_MESH_TINT));

    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(DEFAULT_COLORED_MESH_TINT, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn empty_scene_has_no_meshes() {
    let s = Scene::new();
    assert_eq!(s.meshes().len(), 0);
    assert_eq!(s.visible_count(), 0);
}

#[test]
fn add_two_meshes_for_upper_lower() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()).with_tint([0.6, 0.8, 1.0, 1.0]));
    s.add(SceneMesh::new(tri()).with_tint([1.0, 0.7, 0.6, 1.0]));
    assert_eq!(s.meshes().len(), 2);
    assert_eq!(s.visible_count(), 2);
}

#[test]
fn new_scene_mesh_uses_dental_stone_tint() {
    let entry = SceneMesh::new(tri());
    assert_eq!(entry.tint, DEFAULT_UNTEXTURED_MESH_TINT);
}

#[test]
fn colored_scene_mesh_uses_neutral_tint() {
    let entry = SceneMesh::new(colored_tri());
    assert_eq!(entry.tint, DEFAULT_COLORED_MESH_TINT);
}

#[test]
fn textured_scene_mesh_uses_neutral_tint() {
    let entry = SceneMesh::new(textured_tri());
    assert_eq!(entry.tint, DEFAULT_COLORED_MESH_TINT);
}

#[test]
fn opacity_is_clamped() {
    let e = SceneMesh::new(tri()).with_opacity(5.0);
    assert_eq!(e.opacity, 1.0);
    let e = SceneMesh::new(tri()).with_opacity(-1.0);
    assert_eq!(e.opacity, 0.0);
}

#[test]
fn wireframe_overlay_defaults_off_and_can_be_enabled() {
    let e = SceneMesh::new(tri());
    assert!(!e.wireframe);
    let e = e.with_wireframe(true);
    assert!(e.wireframe);
}

#[test]
fn hide_affects_visible_count() {
    let mut s = Scene::new();
    let i = s.add(SceneMesh::new(tri()));
    s.add(SceneMesh::new(tri()));
    s.meshes_mut()[i].visible = false;
    assert_eq!(s.visible_count(), 1);
}

#[test]
fn scene_bbox_unions_visible_meshes() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()));
    s.add(
        SceneMesh::new(tri()).with_transform(Affine3A::from_translation(Vec3::new(10.0, 0.0, 0.0))),
    );
    let b = s.bbox();
    assert!(!b.is_empty());
    assert_eq!(b.min.x, 0.0);
    assert!((b.max.x - 11.0).abs() < 1e-5, "max.x = {}", b.max.x);
}

#[test]
fn scene_bbox_skips_hidden_meshes() {
    let mut s = Scene::new();
    let i = s.add(SceneMesh::new(tri()));
    s.add(
        SceneMesh::new(tri())
            .with_transform(Affine3A::from_translation(Vec3::new(100.0, 0.0, 0.0))),
    );
    s.meshes_mut()[i].visible = false;
    let b = s.bbox();
    assert!(!b.is_empty());
    assert!(b.min.x >= 100.0, "min.x = {}", b.min.x);
}

#[test]
fn scene_bbox_empty_scene() {
    let s = Scene::new();
    assert!(s.bbox().is_empty());
}

#[test]
fn scene_bbox_uses_mesh_bbox_cache_for_repaint_safety() {
    let bbox_source = source_file("src/scene/bounds.rs");

    assert!(
        bbox_source.contains("entry.mesh.bbox_cached()"),
        "scene bbox should use each mesh's constructor-time bbox cache"
    );
    assert!(
        !bbox_source.contains("entry.mesh.bbox_uncached()"),
        "scene bbox must not walk mesh vertices on every repaint"
    );
}

#[test]
fn pick_ray_hits_visible_triangle_surface() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()));

    let hit = s.pick_ray(Vec3::new(0.25, 0.25, 10.0), Vec3::NEG_Z);

    assert!(hit.is_some(), "expected surface hit");
    let Some(hit) = hit else {
        return;
    };
    assert!(
        (hit - Vec3::new(0.25, 0.25, 0.0)).length() < 1e-5,
        "hit={hit}"
    );
}

#[test]
fn pick_ray_returns_nearest_visible_hit() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()));
    s.add(SceneMesh::new(tri()).with_transform(Affine3A::from_translation(Vec3::Z * 5.0)));

    let hit = s.pick_ray(Vec3::new(0.25, 0.25, 10.0), Vec3::NEG_Z);

    assert!(hit.is_some(), "expected nearest hit");
    let Some(hit) = hit else {
        return;
    };
    assert!((hit.z - 5.0).abs() < 1e-5, "hit={hit}");
}

#[test]
fn pick_ray_hit_reports_layer_identity_and_triangle_index() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()));
    let layer_index = s.add(
        SceneMesh::new(two_triangle_mesh())
            .with_transform(Affine3A::from_translation(Vec3::Z * 5.0)),
    );
    let layer_id = s.meshes()[layer_index].id();

    let hit = s.pick_ray_hit(Vec3::new(2.25, 0.25, 10.0), Vec3::NEG_Z);

    assert!(hit.is_some(), "expected editable face hit");
    let Some(hit) = hit else {
        return;
    };
    assert_eq!(hit.layer_index, layer_index);
    assert_eq!(hit.layer_id, layer_id);
    assert_eq!(hit.triangle_index, 1);
    assert!(
        (hit.point - Vec3::new(2.25, 0.25, 5.0)).length() < 1e-5,
        "hit={hit:?}"
    );
}

#[test]
fn pick_layer_ray_hit_ignores_nearer_non_target_layers() {
    let mut scene = Scene::new();
    let target_index = scene.add(SceneMesh::new(tri()));
    let target_id = scene.meshes()[target_index].id();
    scene.add(SceneMesh::new(tri()).with_transform(Affine3A::from_translation(Vec3::Z * 5.0)));

    let hit = scene.pick_layer_ray_hit(Vec3::new(0.25, 0.25, 10.0), Vec3::NEG_Z, target_id);

    let hit = hit.expect("expected a hit on the requested layer");
    assert_eq!(hit.layer_index, target_index);
    assert_eq!(hit.layer_id, target_id);
    assert!(hit.point.z.abs() < 1e-5, "hit={hit:?}");
}

#[test]
fn pick_layer_ray_hit_rejects_hidden_or_stale_targets() {
    let mut scene = Scene::new();
    let mut target = SceneMesh::new(tri());
    target.visible = false;
    let target_id = target.id();
    scene.add(target);
    let stale_id = SceneMesh::new(tri()).id();

    let origin = Vec3::new(0.25, 0.25, 10.0);
    assert!(scene
        .pick_layer_ray_hit(origin, Vec3::NEG_Z, target_id)
        .is_none());
    assert!(scene
        .pick_layer_ray_hit(origin, Vec3::NEG_Z, stale_id)
        .is_none());
}

#[test]
fn pick_ray_ignores_hidden_meshes() {
    let mut s = Scene::new();
    let mut hidden =
        SceneMesh::new(tri()).with_transform(Affine3A::from_translation(Vec3::Z * 5.0));
    hidden.visible = false;
    s.add(hidden);
    s.add(SceneMesh::new(tri()));

    let hit = s.pick_ray(Vec3::new(0.25, 0.25, 10.0), Vec3::NEG_Z);

    assert!(hit.is_some(), "expected visible hit");
    let Some(hit) = hit else {
        return;
    };
    assert!(hit.z.abs() < 1e-5, "hit={hit}");
}

#[test]
fn remove_drops_requested_entry() {
    let mut s = Scene::new();
    let first = s.add(SceneMesh::new(tri()));
    let second = s.add(SceneMesh::new(tri()).with_tint([1.0, 0.7, 0.6, 1.0]));
    let remaining_id = s.meshes()[second].id();

    let removed = s.remove(first).expect("entry removed");

    assert_eq!(removed.tint, DEFAULT_UNTEXTURED_MESH_TINT);
    assert_eq!(s.meshes().len(), 1);
    assert_eq!(s.meshes()[0].tint, [1.0, 0.7, 0.6, 1.0]);
    assert_eq!(s.meshes()[0].id(), remaining_id);
    assert!(
        s.remove(second).is_none(),
        "index is now out of date after removal"
    );
}

#[test]
fn scene_mesh_ids_are_stable_across_clone_remove_and_append() {
    let mut first_scene = Scene::new();
    first_scene.add(SceneMesh::new(tri()));
    first_scene.add(SceneMesh::new(tri()));
    let first_id = first_scene.meshes()[0].id();
    let second_id = first_scene.meshes()[1].id();

    assert_ne!(first_id, second_id);
    assert_eq!(first_scene.clone().meshes()[0].id(), first_id);

    let removed = first_scene.remove(0).expect("first layer removed");
    assert_eq!(removed.id(), first_id);
    assert_eq!(first_scene.meshes()[0].id(), second_id);

    let mut second_scene = Scene::new();
    second_scene.add(SceneMesh::new(tri()));
    let appended_id = second_scene.meshes()[0].id();
    first_scene.append_scene(second_scene);

    assert_eq!(first_scene.meshes()[0].id(), second_id);
    assert_eq!(first_scene.meshes()[1].id(), appended_id);
    assert_ne!(first_scene.meshes()[0].id(), first_scene.meshes()[1].id());
}

#[test]
fn append_scene_keeps_existing_order_and_appends_new_entries() {
    let mut s = Scene::new();
    s.add(SceneMesh::new(tri()).with_tint([1.0, 1.0, 1.0, 1.0]));

    let mut other = Scene::new();
    other.add(SceneMesh::new(tri()).with_tint([0.82, 0.90, 1.0, 1.0]));
    other.add(SceneMesh::new(tri()).with_tint([1.0, 0.88, 0.78, 1.0]));

    s.append_scene(other);

    assert_eq!(s.meshes().len(), 3);
    assert_eq!(s.meshes()[0].tint, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(s.meshes()[1].tint, [0.82, 0.90, 1.0, 1.0]);
    assert_eq!(s.meshes()[2].tint, [1.0, 0.88, 0.78, 1.0]);
}

#[test]
fn append_scene_preserves_existing_scene_settings() {
    let mut s = Scene::new();
    s.background = [0.1, 0.2, 0.3, 1.0];
    s.ambient = 0.6;
    s.key_light_dir = Vec3::new(1.0, 0.0, 0.0);

    let mut other = Scene::new();
    other.background = [0.9, 0.8, 0.7, 1.0];
    other.ambient = 0.1;
    other.key_light_dir = Vec3::new(0.0, 1.0, 0.0);
    other.add(SceneMesh::new(tri()));

    s.append_scene(other);

    assert_eq!(s.background, [0.1, 0.2, 0.3, 1.0]);
    assert_eq!(s.ambient, 0.6);
    assert_eq!(s.key_light_dir, Vec3::new(1.0, 0.0, 0.0));
}

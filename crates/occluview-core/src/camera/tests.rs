use super::*;
use crate::Aabb;
use glam::{Vec2, Vec3};

fn cube_bbox() -> Aabb {
    Aabb::from_min_max(Vec3::new(-10.0, -10.0, -10.0), Vec3::new(10.0, 10.0, 10.0))
}

/// View-depths (projection onto the forward axis, relative to the eye) of every
/// bbox corner for the given camera.
fn corner_view_depths(camera: &Camera, bbox: Aabb) -> [f32; 8] {
    let eye = camera.eye();
    let forward = camera.view_direction();
    let min = bbox.min;
    let max = bbox.max;
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
    .map(|corner| (corner - eye).dot(forward))
}

/// Assert that after a refit every bbox corner's view-depth lies within
/// `[near, far]`, so nothing is clipped.
fn assert_all_corners_within_clip(camera: &Camera, bbox: Aabb, ctx: &str) {
    assert!(
        camera.far > camera.near,
        "{ctx}: clip span collapsed near={} far={}",
        camera.near,
        camera.far
    );
    for depth in corner_view_depths(camera, bbox) {
        assert!(
            depth >= camera.near && depth <= camera.far,
            "{ctx}: corner depth {depth} escaped clip range [{}, {}]",
            camera.near,
            camera.far
        );
    }
}

/// Combined bbox of ~10 small objects spread across the scene, as produced when
/// several files are opened together from Explorer.
fn spread_multi_object_bbox() -> Aabb {
    Aabb::from_min_max(
        Vec3::new(-100.0, -3.0, -100.0),
        Vec3::new(100.0, 3.0, 100.0),
    )
}

mod behavior;
mod zoom;

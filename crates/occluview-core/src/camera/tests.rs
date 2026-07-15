use super::*;
use crate::Aabb;
use glam::{Vec2, Vec3};
use std::path::PathBuf;

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

/// Assert the invariant the near-plane bug violated: after a refit, EVERY bbox
/// corner's view-depth lies within `[near, far]`, so nothing is clipped.
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

fn source_file(relative_path: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(relative_path);
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn camera_module_is_split_by_responsibility_not_single_file() {
    let facade = source_file("src/camera/mod.rs");
    let input = source_file("src/camera/input.rs");
    let orientation = source_file("src/camera/orientation.rs");
    let orbit = source_file("src/camera/orbit.rs");
    let framing = source_file("src/camera/framing.rs");
    let presets = source_file("src/camera/presets.rs");
    let movement = source_file("src/camera/movement.rs");
    let lib = source_file("src/lib.rs");

    assert!(
        facade.contains("mod framing;")
            && facade.contains("mod input;")
            && facade.contains("mod movement;")
            && facade.contains("mod orbit;")
            && facade.contains("mod orientation;")
            && facade.contains("mod presets;"),
        "camera should be a private module directory split by input, orientation, orbit, movement, framing, and presets"
    );
    assert!(
        facade.contains("pub struct Camera")
            && facade.contains("pub enum CameraProjection")
            && facade.contains("orbit_delta_from_pointer_motion")
            && facade.contains("zoom_factor_from_scroll")
            && facade.contains("CAD_ORBIT_DRAG_GAIN"),
        "camera facade should keep the public core API stable"
    );
    assert!(
        input.contains("pub fn orbit_delta_from_pointer_motion")
            && orientation.contains("fn orientation_from_yaw_pitch")
            && orbit.contains("pub fn orbit_view_by")
            && framing.contains("pub fn frame_occlusal")
            && presets.contains("pub enum CameraPreset")
            && movement.contains("pub fn pan_screen"),
        "camera responsibilities should live in focused modules"
    );
    assert!(
        lib.contains("pub use camera::{")
            && lib.contains("orbit_delta_from_pointer_motion")
            && lib.contains("Camera,")
            && lib.contains("CameraAxisView")
            && lib.contains("CameraPreset")
            && lib.contains("CameraProjection")
            && lib.contains("CAD_ORBIT_DRAG_GAIN"),
        "occluview-core should keep the same public camera reexports"
    );
}

mod behavior;

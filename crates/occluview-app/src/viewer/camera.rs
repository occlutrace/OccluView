use glam::{Mat4, Vec3};
use occluview_core::{Camera, Scene};

const CAMERA_FOVY_RAD: f32 = 45.0_f32.to_radians();

pub(crate) fn home_camera_for_scene(scene: &Scene) -> Camera {
    Camera::default().frame_occlusal(scene.bbox(), CAMERA_FOVY_RAD)
}

pub(crate) fn build_view_matrix(cam: &Camera) -> Mat4 {
    occluview_render::camera_view_matrix(cam)
}

pub(crate) fn build_proj_matrix(cam: &Camera, aspect: f32) -> Mat4 {
    occluview_render::camera_ortho_proj_matrix(cam, aspect)
}

pub(crate) fn camera_studio_light_dir(cam: &Camera) -> Vec3 {
    let forward = cam.view_direction();
    let up = cam.view_up();
    let right = forward.cross(up).normalize_or_zero();
    (-forward + up * 0.32 + right * 0.22).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_matrix_is_orthographic_and_ignores_fovy() {
        let camera = Camera {
            orthographic_height: 120.0,
            near: 0.25,
            far: 2000.0,
            fovy: 20.0_f32.to_radians(),
            ..Camera::default()
        };
        let mut changed_fovy = camera;
        changed_fovy.fovy = 80.0_f32.to_radians();

        let baseline = build_proj_matrix(&camera, 1.5).to_cols_array();
        let changed = build_proj_matrix(&changed_fovy, 1.5).to_cols_array();
        assert!(
            baseline
                .into_iter()
                .zip(changed)
                .all(|(left, right)| (left - right).abs() <= f32::EPSILON),
            "orthographic projection should not depend on fovy"
        );
    }

    #[test]
    fn studio_light_tracks_camera_basis() {
        let camera = Camera::default();
        let dir = camera_studio_light_dir(&camera);

        assert!(dir.is_finite());
        assert!((dir.length() - 1.0).abs() < 1e-5, "dir={dir}");
    }
}

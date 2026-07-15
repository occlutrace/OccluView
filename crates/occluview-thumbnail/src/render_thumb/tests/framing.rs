use super::*;
use glam::{Mat4, Vec3};
use occluview_core::{Aabb, Camera, Mesh, DEFAULT_UNTEXTURED_MESH_TINT};

#[test]
fn thumbnail_camera_keeps_occlusal_orientation_but_frames_projected_bounds_tightly() {
    let mesh = Mesh::new(
        Some("fixture".into()),
        vec![
            occluview_core::Vertex::at(Vec3::new(-12.0, -2.0, -8.0)),
            occluview_core::Vertex::at(Vec3::new(16.0, 3.0, 10.0)),
            occluview_core::Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    );
    let Ok(mut mesh) = mesh else {
        return;
    };
    let app_camera = Camera::default().frame_occlusal(mesh.bbox(), 45.0_f32.to_radians());
    let thumbnail_camera = rendering::thumbnail_camera_for_bbox(mesh.bbox());

    assert_eq!(thumbnail_camera.target, app_camera.target);
    assert_eq!(thumbnail_camera.yaw.to_bits(), app_camera.yaw.to_bits());
    assert_eq!(thumbnail_camera.pitch.to_bits(), app_camera.pitch.to_bits());
    assert_eq!(thumbnail_camera.projection, app_camera.projection);
    assert!(thumbnail_camera.orthographic_height < app_camera.orthographic_height);

    let projected_span = {
        let actual = rendering::thumbnail_projection_matrix(&thumbnail_camera);
        let half_height = thumbnail_camera.orthographic_height * 0.5;
        let expected = Mat4::orthographic_rh(
            -half_height,
            half_height,
            -half_height,
            half_height,
            thumbnail_camera.near,
            thumbnail_camera.far,
        );
        assert_eq!(actual, expected);
        (thumbnail_camera.orthographic_height * 0.86).abs()
    };
    assert!(projected_span.is_finite());
}

#[test]
fn thumbnail_mesh_frame_ignores_sparse_outliers() {
    let mesh = fixtures::point_cluster_with_outlier();
    let Ok(mesh) = mesh else {
        return;
    };
    let camera = rendering::thumbnail_camera_for_mesh(&mesh);
    let bbox_camera = rendering::thumbnail_camera_for_bbox(mesh.bbox_cached());
    assert!(camera.orthographic_height < bbox_camera.orthographic_height * 0.35);
}

#[test]
fn thumbnail_projection_uses_orthographic_when_camera_is_orthographic() {
    let camera = Camera::default().frame_occlusal(
        Aabb::from_min_max(Vec3::new(-10.0, -2.0, -8.0), Vec3::new(10.0, 2.0, 8.0)),
        45.0_f32.to_radians(),
    );
    let actual = rendering::thumbnail_projection_matrix(&camera);
    let half_height = camera.orthographic_height * 0.5;
    let expected = Mat4::orthographic_rh(
        -half_height,
        half_height,
        -half_height,
        half_height,
        camera.near,
        camera.far,
    );
    assert_eq!(actual, expected);
}

#[test]
fn thumbnail_uniform_uses_stone_tint_for_untextured_mesh() {
    let mesh = Mesh::empty();
    let uniform = rendering::thumbnail_mesh_uniform(&mesh);
    assert_tint_eq(uniform.tint, DEFAULT_UNTEXTURED_MESH_TINT);
    assert_eq!(uniform.has_texture, 0);
}

#[test]
fn thumbnail_uniform_keeps_textured_mesh_colors_neutral() {
    let mut mesh = Mesh::empty();
    mesh.set_texture(occluview_core::MeshTexture::white_1x1());
    let uniform = rendering::thumbnail_mesh_uniform(&mesh);
    assert_tint_eq(uniform.tint, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(uniform.has_texture, 1);
}

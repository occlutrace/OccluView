use eframe::egui;
use glam::{Vec2, Vec3};
use occluview_core::{orbit_delta_from_pointer_motion, Aabb, Camera, Scene, ScenePickHit};

// Wheel zoom shares the CAD-tuned core mapping with the shell preview so the
// two surfaces feel identical (owner rule: no braked camera, no reversals).
pub(crate) use occluview_core::zoom_factor_from_scroll;

pub(crate) fn orbit_delta_from_drag(
    delta_px: egui::Vec2,
    viewport_size: egui::Vec2,
) -> Option<Vec2> {
    orbit_delta_from_pointer_motion(
        Vec2::new(delta_px.x, delta_px.y),
        Vec2::new(viewport_size.x, viewport_size.y),
    )
}

pub(crate) fn viewport_pan_drag_active(ctx: &egui::Context, response: &egui::Response) -> bool {
    let primary_secondary_down = ctx.input(|i| {
        i.pointer.button_down(egui::PointerButton::Primary)
            && i.pointer.button_down(egui::PointerButton::Secondary)
    });
    response.dragged_by(egui::PointerButton::Middle)
        || viewport_combined_pan_drag_active(
            primary_secondary_down,
            response.is_pointer_button_down_on(),
        )
}

pub(crate) fn viewport_combined_pan_drag_active(
    primary_secondary_down: bool,
    viewport_press_owned: bool,
) -> bool {
    primary_secondary_down && viewport_press_owned
}

pub(crate) fn viewport_orbit_drag_active(
    pan_drag_active: bool,
    secondary_down: bool,
    orbit_cursor_grabbed: bool,
    secondary_owned_motion: Option<egui::Vec2>,
) -> bool {
    !pan_drag_active
        && secondary_down
        && (orbit_cursor_grabbed
            || secondary_owned_motion.is_some_and(|motion| motion.length_sq() > f32::EPSILON))
}

pub(crate) fn pick_scene_point(
    camera: &Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    scene: &Scene,
) -> Option<Vec3> {
    let (origin, direction) = viewport_ray_for_scene(camera, viewport_rect, pointer, scene)?;
    scene
        .pick_ray_hit(origin, direction)
        .map(|hit| hit.point)
        .or_else(|| ray_aabb_entry(origin, direction, scene.bbox()))
}

pub(crate) fn pick_scene_hit(
    camera: &Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    scene: &Scene,
) -> Option<ScenePickHit> {
    let (origin, direction) = viewport_ray_for_scene(camera, viewport_rect, pointer, scene)?;
    scene.pick_ray_hit(origin, direction)
}

pub(crate) fn project_world_to_viewport(
    camera: &Camera,
    viewport_rect: egui::Rect,
    point: Vec3,
) -> Option<(egui::Pos2, f32)> {
    let width = viewport_rect.width();
    let height = viewport_rect.height();
    if width <= 0.0 || height <= 0.0 || !point.is_finite() {
        return None;
    }

    let eye = camera.eye();
    let forward = camera.view_direction();
    if forward.length_squared() <= f32::EPSILON {
        return None;
    }
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return None;
    }

    let offset = point - eye;
    let depth = offset.dot(forward);
    if !depth.is_finite() {
        return None;
    }

    let half_height = camera.orthographic_height * 0.5;
    let half_width = half_height * (width / height);
    if half_height <= f32::EPSILON || half_width <= f32::EPSILON {
        return None;
    }

    let x = offset.dot(right) / half_width;
    let y = offset.dot(up) / half_height;
    let screen = egui::pos2(
        viewport_rect.left() + ((x + 1.0) * 0.5 * width),
        viewport_rect.top() + ((1.0 - (y + 1.0) * 0.5) * height),
    );
    Some((screen, depth))
}

fn viewport_ray_for_scene(
    camera: &Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    scene: &Scene,
) -> Option<(Vec3, Vec3)> {
    let bbox = scene.bbox();
    if bbox.is_empty() || !viewport_rect.contains(pointer) {
        return None;
    }
    viewport_ray(camera, viewport_rect, pointer)
}

pub(crate) fn viewport_ray(
    camera: &Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
) -> Option<(Vec3, Vec3)> {
    let width = viewport_rect.width();
    let height = viewport_rect.height();
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    let x = ((pointer.x - viewport_rect.left()) / width) * 2.0 - 1.0;
    let y = 1.0 - ((pointer.y - viewport_rect.top()) / height) * 2.0;
    let eye = camera.eye();
    let forward = camera.view_direction();
    if forward.length_squared() <= f32::EPSILON {
        return None;
    }
    let up = camera.view_up();
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
        return None;
    }

    let half_height = camera.orthographic_height * 0.5;
    let half_width = half_height * (width / height);
    let origin = eye + right * x * half_width + up * y * half_height;
    Some((origin, forward))
}

fn ray_aabb_entry(origin: Vec3, direction: Vec3, bbox: Aabb) -> Option<Vec3> {
    let mut t_min = 0.0_f32;
    let mut t_max = f32::INFINITY;
    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        let min = bbox.min[axis];
        let max = bbox.max[axis];
        if d.abs() <= f32::EPSILON {
            if o < min || o > max {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t0 = (min - o) * inv;
        let mut t1 = (max - o) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_max < t_min {
            return None;
        }
    }
    let t = if t_min >= 0.0 { t_min } else { t_max };
    if t.is_finite() && t >= 0.0 {
        Some(origin + direction * t)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::{CameraProjection, Mesh, SceneMesh, Vertex};

    #[test]
    fn orbit_delta_from_drag_is_relative_and_unbounded() {
        let viewport = egui::vec2(400.0, 200.0);
        let right = orbit_delta_from_drag(egui::vec2(100.0, 0.0), viewport);
        let down = orbit_delta_from_drag(egui::vec2(0.0, 50.0), viewport);
        let beyond_edge = orbit_delta_from_drag(egui::vec2(360.0, 0.0), viewport);
        assert!(right.is_some(), "right should map");
        assert!(down.is_some(), "down should map");
        assert!(
            beyond_edge.is_some(),
            "large relative drags should keep rotating"
        );
        let right = right.unwrap_or(Vec2::ZERO);
        let down = down.unwrap_or(Vec2::ZERO);
        let beyond_edge = beyond_edge.unwrap_or(Vec2::ZERO);

        assert!(right.x < -1.5 && right.y.abs() < 1e-6, "right={right}");
        assert!(down.y > 0.75 && down.x.abs() < 1e-6, "down={down}");
        assert!(
            beyond_edge.x < right.x,
            "relative orbit must not clamp at a virtual trackball edge: right={right} beyond={beyond_edge}"
        );
    }

    #[test]
    fn zoom_factor_maps_scroll_direction_to_dolly() {
        assert!((zoom_factor_from_scroll(0.0) - 1.0).abs() < 1e-6);
        assert!(zoom_factor_from_scroll(120.0) < 1.0);
        assert!(zoom_factor_from_scroll(-120.0) > 1.0);
    }

    #[test]
    fn combined_primary_secondary_drag_pans_on_any_drag_source() {
        assert!(viewport_combined_pan_drag_active(true, true));
        assert!(!viewport_combined_pan_drag_active(true, false));
        assert!(!viewport_combined_pan_drag_active(false, true));
    }

    #[test]
    fn orbit_starts_on_the_first_sub_threshold_pointer_motion() {
        let tiny_motion = egui::vec2(0.25, -0.1);
        assert!(viewport_orbit_drag_active(
            false,
            true,
            false,
            Some(tiny_motion)
        ));
        assert!(!viewport_orbit_drag_active(
            false,
            true,
            false,
            Some(egui::Vec2::ZERO)
        ));
        assert!(viewport_orbit_drag_active(false, true, true, None));
        assert!(!viewport_orbit_drag_active(
            true,
            true,
            true,
            Some(tiny_motion)
        ));
        assert!(
            !viewport_orbit_drag_active(false, false, true, Some(tiny_motion)),
            "releasing RMB must end orbit even if cursor capture was active"
        );
    }

    #[test]
    fn viewport_center_pick_hits_scene_surface() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 100.0,
            fovy: 45.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let mesh_result = Mesh::new(
            Some("surface".into()),
            vec![
                Vertex::at(Vec3::new(-10.0, -10.0, 0.0)),
                Vertex::at(Vec3::new(10.0, -10.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 10.0, 0.0)),
            ],
            vec![0, 1, 2],
        );
        assert!(mesh_result.is_ok(), "valid mesh should construct");
        let Ok(mesh) = mesh_result else {
            return;
        };
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));

        let picked = pick_scene_point(&camera, viewport, viewport.center(), &scene);

        assert!(picked.is_some(), "center ray should hit surface");
        if let Some(picked) = picked {
            assert!(picked.z.abs() < 1e-3, "picked={picked}");
            assert!(picked.x.abs() < 1e-3, "picked={picked}");
        }
    }

    #[test]
    fn viewport_center_pick_can_return_editable_scene_hit_identity() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 100.0,
            fovy: 45.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let mesh_result = Mesh::new(
            Some("surface".into()),
            vec![
                Vertex::at(Vec3::new(-10.0, -10.0, 0.0)),
                Vertex::at(Vec3::new(10.0, -10.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 10.0, 0.0)),
            ],
            vec![0, 1, 2],
        );
        assert!(mesh_result.is_ok(), "valid mesh should construct");
        let Ok(mesh) = mesh_result else {
            return;
        };
        let mut scene = Scene::new();
        let layer_index = scene.add(SceneMesh::new(mesh));
        let layer_id = scene.meshes()[layer_index].id();

        let picked = pick_scene_hit(&camera, viewport, viewport.center(), &scene);

        assert!(picked.is_some(), "center ray should hit surface");
        let Some(picked) = picked else {
            return;
        };
        assert_eq!(picked.layer_index, layer_index);
        assert_eq!(picked.layer_id, layer_id);
        assert_eq!(picked.triangle_index, 0);
    }

    #[test]
    fn orthographic_ray_origin_follows_pointer_in_view_plane() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 100.0,
            fovy: 80.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));

        let center = viewport_ray(&camera, viewport, viewport.center());
        let right = viewport_ray(&camera, viewport, egui::pos2(300.0, 200.0));

        assert!(center.is_some(), "center ray should build");
        assert!(right.is_some(), "right ray should build");
        let Some((center_origin, center_dir)) = center else {
            return;
        };
        let Some((right_origin, right_dir)) = right else {
            return;
        };
        assert_eq!(center_dir, right_dir);
        assert_ne!(center_origin, right_origin);
    }

    #[test]
    fn world_projection_maps_target_center_to_viewport_center() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            orientation: None,
            projection: CameraProjection::Orthographic,
            orthographic_height: 100.0,
            fovy: 45.0_f32.to_radians(),
            near: 0.1,
            far: 10_000.0,
        };
        let viewport = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(400.0, 200.0));

        let projected = project_world_to_viewport(&camera, viewport, Vec3::ZERO);

        assert!(projected.is_some(), "target center should project");
        let Some((screen, depth)) = projected else {
            return;
        };
        assert!(
            (screen.x - viewport.center().x).abs() < 1e-3,
            "screen={screen:?}"
        );
        assert!(
            (screen.y - viewport.center().y).abs() < 1e-3,
            "screen={screen:?}"
        );
        assert!(depth > 0.0, "depth should stay positive");
    }
}

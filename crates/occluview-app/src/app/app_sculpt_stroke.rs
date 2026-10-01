//! Viewport-ray conversion and brush settings for interactive sculpting.

use super::{egui, mesh_editor_overlay};
use crate::app::workspace::id::SceneKey;
use crate::sculpt_kernel::BrushRayStep;
use crate::sculpt_tool::{SculptTip, SculptToolKind};
use crate::viewer::viewport_ray;
use glam::{Affine3A, Vec3};
use occluview_core::Camera;
use occluview_render::ClipPlane;

const WHEEL_NOTCH_PX: f32 = 40.0;
const WHEEL_GESTURE_GAP_SEC: f64 = 0.3;

pub(super) fn has_sculpt_settings_wheel(ctx: &egui::Context, scene_key: SceneKey) -> bool {
    collect_sculpt_wheel_notches(ctx, scene_key).0
}

#[derive(Clone, Copy, Default)]
struct SculptWheelAccumulator {
    travel: f32,
    last_at: Option<f64>,
    processed_frame: Option<u64>,
    frame_owned: bool,
}

pub(super) fn apply_sculpt_wheel_settings(
    ctx: &egui::Context,
    scene_key: SceneKey,
    kind: Option<SculptToolKind>,
) -> bool {
    let (consumed, notches) = collect_sculpt_wheel_notches(ctx, scene_key);
    if !consumed {
        return false;
    }
    for (ctrl, direction) in notches {
        // Ctrl changes strength; Shift changes size. Ctrl takes priority.
        if ctrl {
            let kind = kind.unwrap_or(SculptToolKind::AddRemove);
            let next = kind.step_strength(
                mesh_editor_overlay::sculpt_strength(ctx, scene_key, kind),
                direction,
            );
            mesh_editor_overlay::set_sculpt_strength(ctx, scene_key, kind, next);
        } else {
            let tip = mesh_editor_overlay::sculpt_tip(ctx, scene_key);
            let next = tip.step_radius_mm(
                mesh_editor_overlay::sculpt_radius_mm(ctx, scene_key, tip),
                direction,
            );
            mesh_editor_overlay::set_sculpt_radius_mm(ctx, scene_key, tip, next);
        }
    }
    true
}

/// Claim and translate modified wheel input at most once per egui frame.
/// Egui can rerun UI code in another pass for the same frame; the cached
/// ownership result keeps that replay from changing the brush or reaching zoom.
#[allow(
    clippy::float_cmp,
    reason = "Wheel zero is a protocol sentinel; any nonzero sub-notch travel accumulates."
)]
fn collect_sculpt_wheel_notches(
    ctx: &egui::Context,
    scene_key: SceneKey,
) -> (bool, Vec<(bool, f32)>) {
    let frame = ctx.cumulative_frame_nr();
    let (events, now) = ctx.input(|input| (input.raw.events.clone(), input.time));
    ctx.data_mut(|data| {
        let id = egui::Id::new(("occluview_sculpt_wheel_accumulator", scene_key));
        let mut accumulator = data
            .get_temp::<SculptWheelAccumulator>(id)
            .unwrap_or_default();
        if accumulator.processed_frame == Some(frame) {
            return (accumulator.frame_owned, Vec::new());
        }

        accumulator.processed_frame = Some(frame);
        let mut consumed = false;
        let mut notches = Vec::new();
        for event in events {
            let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let ctrl = modifiers.ctrl || modifiers.command;
            let shift = modifiers.shift;
            if !ctrl && !shift {
                continue;
            }
            consumed = true;
            // On common platforms Shift moves vertical wheel travel to X.
            let scroll = if delta.y == 0.0 { delta.x } else { delta.y };
            if scroll == 0.0 {
                continue;
            }
            // egui-winit keeps native scroll signs; positive Y is this
            // viewport's zoom-in direction, opposite to DOM `deltaY`.
            let direction = match unit {
                egui::MouseWheelUnit::Point => {
                    if scroll.abs() >= WHEEL_NOTCH_PX {
                        accumulator.travel = 0.0;
                        accumulator.last_at = None;
                        Some(if scroll > 0.0 { 1.0 } else { -1.0 })
                    } else {
                        if accumulator
                            .last_at
                            .is_none_or(|last| now - last > WHEEL_GESTURE_GAP_SEC)
                            || accumulator.travel * scroll < 0.0
                        {
                            accumulator.travel = 0.0;
                        }
                        accumulator.last_at = Some(now);
                        accumulator.travel += scroll;
                        if accumulator.travel.abs() < WHEEL_NOTCH_PX {
                            None
                        } else {
                            let travel_direction = accumulator.travel.signum();
                            accumulator.travel -= travel_direction * WHEEL_NOTCH_PX;
                            Some(if travel_direction > 0.0 { 1.0 } else { -1.0 })
                        }
                    }
                }
                egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                    accumulator.travel = 0.0;
                    accumulator.last_at = None;
                    Some(if scroll > 0.0 { 1.0 } else { -1.0 })
                }
            };
            if let Some(direction) = direction {
                notches.push((ctrl, direction));
            }
        }
        accumulator.frame_owned = consumed;
        data.insert_temp(id, accumulator);
        (consumed, notches)
    })
}

/// Construct the exact local ray and visible interval the kernel will use.
/// Clip coefficients are transformed as a plane covector, so the keep side
/// remains unchanged under the layer's rigid or uniform-scale transform.
pub(super) struct LocalBrushRayInput<'a> {
    pub(super) camera: &'a Camera,
    pub(super) viewport_rect: egui::Rect,
    pub(super) pointer: egui::Pos2,
    pub(super) world_to_local: Affine3A,
    pub(super) local_per_world: f32,
    pub(super) clip_plane: ClipPlane,
    pub(super) kind: SculptToolKind,
    pub(super) tip: SculptTip,
    pub(super) shift: bool,
    pub(super) command: bool,
    pub(super) radius_world_mm: f32,
    pub(super) strength: f32,
    pub(super) hold: bool,
}

pub(super) fn local_brush_ray_step(input: LocalBrushRayInput<'_>) -> Option<BrushRayStep> {
    let LocalBrushRayInput {
        camera,
        viewport_rect,
        pointer,
        world_to_local,
        local_per_world,
        clip_plane,
        kind,
        tip,
        shift,
        command,
        radius_world_mm,
        strength,
        hold,
    } = input;
    if !camera.near.is_finite()
        || !camera.far.is_finite()
        || camera.far < camera.near
        || !radius_world_mm.is_finite()
        || radius_world_mm <= 0.0
        || !strength.is_finite()
    {
        return None;
    }
    let (world_origin, world_direction) = viewport_ray(camera, viewport_rect, pointer)?;
    let localized = localize_visible_ray(
        world_origin,
        world_direction,
        VisibleRayTransform {
            camera_near: camera.near,
            camera_far: camera.far,
            world_to_local,
            local_per_world,
        },
    )?;
    let local_origin = localized.origin;
    let local_direction = localized.direction;
    if !local_origin.is_finite()
        || !local_direction.is_finite()
        || local_direction.length_squared() <= f32::EPSILON
    {
        return None;
    }
    let clip_plane = local_clip_plane(world_to_local, clip_plane);
    let (radius_min, radius_max) = tip.radius_range_mm();
    let strength = kind.dab_strength(strength, shift);
    let camera_right = camera
        .view_direction()
        .cross(camera.view_up())
        .normalize_or_zero();
    let local_axis = world_to_local
        .transform_vector3(camera_right)
        .normalize_or_zero();
    let mode = kind.brush_mode(shift, command);
    Some(BrushRayStep {
        origin: local_origin.to_array(),
        direction: local_direction.to_array(),
        near_mm: localized.near_mm,
        far_mm: localized.far_mm,
        clip_plane,
        radius_mm: (radius_world_mm * local_per_world)
            .clamp(radius_min * local_per_world, radius_max * local_per_world),
        strength,
        mode,
        tip,
        axis: (tip == SculptTip::Knife
            && local_axis.is_finite()
            && local_axis.length_squared() > f32::EPSILON)
            .then_some(local_axis.to_array()),
        hold,
        preserve_skirt: command && mode != crate::sculpt_kernel::BrushMode::Relax,
    })
}

pub(super) fn local_clip_plane(
    world_to_local: Affine3A,
    clip_plane: ClipPlane,
) -> Option<[f64; 4]> {
    (clip_plane.enabled != 0).then(|| {
        let normal = Vec3::from_array(clip_plane.normal);
        let local_to_world = world_to_local.inverse();
        let linear = local_to_world.matrix3;
        let local_normal = Vec3::new(
            normal.dot(linear.x_axis.into()),
            normal.dot(linear.y_axis.into()),
            normal.dot(linear.z_axis.into()),
        );
        let local_constant = normal.dot(local_to_world.translation.into()) - clip_plane.distance;
        [
            f64::from(local_normal.x),
            f64::from(local_normal.y),
            f64::from(local_normal.z),
            f64::from(local_constant),
        ]
    })
}

pub(super) fn local_ray_hit_is_visible(
    point: Vec3,
    origin: Vec3,
    direction: Vec3,
    far_mm: f32,
    clip_plane: Option<[f64; 4]>,
) -> bool {
    let distance = (point - origin).dot(direction);
    point.is_finite()
        && distance.is_finite()
        && distance >= -1e-5
        && distance <= far_mm + 1e-5
        && clip_plane.is_none_or(|[x, y, z, w]| {
            let visible_side =
                x * f64::from(point.x) + y * f64::from(point.y) + z * f64::from(point.z) + w;
            visible_side.is_finite() && visible_side >= -1e-5
        })
}

struct LocalizedVisibleRay {
    origin: Vec3,
    direction: Vec3,
    near_mm: f32,
    far_mm: f32,
}

/// Move the ray origin onto the camera's near plane. This preserves the exact
/// visible interval while satisfying the kernel's nonnegative near contract,
/// including cameras whose orthographic near plane is behind the eye.
struct VisibleRayTransform {
    camera_near: f32,
    camera_far: f32,
    world_to_local: Affine3A,
    local_per_world: f32,
}

fn localize_visible_ray(
    world_origin: Vec3,
    world_direction: Vec3,
    transform: VisibleRayTransform,
) -> Option<LocalizedVisibleRay> {
    let VisibleRayTransform {
        camera_near,
        camera_far,
        world_to_local,
        local_per_world,
    } = transform;
    if !camera_near.is_finite()
        || !camera_far.is_finite()
        || camera_far < camera_near
        || !local_per_world.is_finite()
        || local_per_world <= 0.0
    {
        return None;
    }
    let world_direction = world_direction.normalize_or_zero();
    if !world_origin.is_finite()
        || !world_direction.is_finite()
        || world_direction.length_squared() <= f32::EPSILON
    {
        return None;
    }
    let near_origin = world_origin + world_direction * camera_near;
    let origin = world_to_local.transform_point3(near_origin);
    let direction = world_to_local
        .transform_vector3(world_direction)
        .normalize_or_zero();
    let far_mm = (camera_far - camera_near) * local_per_world;
    (origin.is_finite()
        && direction.is_finite()
        && direction.length_squared() > f32::EPSILON
        && far_mm.is_finite()
        && far_mm >= 0.0)
        .then_some(LocalizedVisibleRay {
            origin,
            direction,
            near_mm: 0.0,
            far_mm,
        })
}

#[cfg(test)]
mod tests {
    // Fixtures and exact binary-representable ray values must fail closed.
    #![allow(clippy::expect_used, clippy::float_cmp)]
    use super::*;
    use glam::Quat;
    use occluview_core::{Mesh, Vertex};

    fn transformed_camera() -> Camera {
        Camera {
            target: Vec3::ZERO,
            distance: 10.0,
            orientation: Some(Quat::IDENTITY),
            near: -5.0,
            far: 15.0,
            orthographic_height: 20.0,
            ..Camera::default()
        }
    }

    #[test]
    fn negative_near_ray_keeps_the_visible_interval_after_layer_transform() {
        let camera = transformed_camera();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        let local_to_world = Affine3A::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::new(3.0, -4.0, 2.0),
        );
        let step = local_brush_ray_step(LocalBrushRayInput {
            camera: &camera,
            viewport_rect: viewport,
            pointer: viewport.center(),
            world_to_local: local_to_world.inverse(),
            local_per_world: 0.5,
            clip_plane: ClipPlane::disabled(),
            kind: SculptToolKind::AddRemove,
            tip: SculptTip::Ball,
            shift: false,
            command: false,
            radius_world_mm: 0.75,
            strength: 0.35,
            hold: false,
        })
        .expect("the camera's visible ray spans behind and ahead of its eye");

        assert_eq!(step.near_mm, 0.0);
        assert_eq!(step.far_mm, 10.0);
        assert!((step.origin[0] + 1.5).abs() < 1e-5);
        assert!((step.origin[1] - 2.0).abs() < 1e-5);
        assert!((step.origin[2] - 6.5).abs() < 1e-5);
        assert_eq!(step.direction, [0.0, 0.0, -1.0]);
        assert_eq!(step.radius_mm, 0.375);
        let clip = local_clip_plane(
            local_to_world.inverse(),
            ClipPlane::new([0.0, 0.0, 1.0], 9.0),
        )
        .expect("an enabled plane must localize");
        assert_eq!(clip, [0.0, 0.0, 2.0, -7.0]);
        assert!(local_ray_hit_is_visible(
            Vec3::new(-1.5, 2.0, 3.5),
            Vec3::from_array(step.origin),
            Vec3::from_array(step.direction),
            step.far_mm,
            Some(clip),
        ));
        assert!(!local_ray_hit_is_visible(
            Vec3::new(-1.5, 2.0, 2.5),
            Vec3::from_array(step.origin),
            Vec3::from_array(step.direction),
            step.far_mm,
            Some(clip),
        ));
        assert!(!local_ray_hit_is_visible(
            Vec3::new(-1.5, 2.0, -4.0),
            Vec3::from_array(step.origin),
            Vec3::from_array(step.direction),
            step.far_mm,
            None,
        ));
    }

    #[test]
    fn clipped_front_triangle_does_not_hide_the_visible_back_hit() {
        let mesh = Mesh::new(
            None,
            vec![
                Vertex::at(Vec3::new(-1.0, -1.0, 2.0)),
                Vertex::at(Vec3::new(1.0, -1.0, 2.0)),
                Vertex::at(Vec3::new(1.0, 1.0, 2.0)),
                Vertex::at(Vec3::new(-1.0, 1.0, 2.0)),
                Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
                Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
                Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        )
        .expect("two stacked quads form a pickable mesh");
        let origin = Vec3::new(0.0, 0.0, 4.0);
        let direction = -Vec3::Z;
        let clip = local_clip_plane(Affine3A::IDENTITY, ClipPlane::new([0.0, 0.0, -1.0], -1.0));
        let keep = |point| local_ray_hit_is_visible(point, origin, direction, 10.0, clip);

        let (triangle, point) = mesh
            .pick_ray_local(origin, direction, keep)
            .expect("BVH traversal must continue after rejecting the clipped front");
        assert!(triangle >= 2);
        assert_eq!(point, Vec3::ZERO);
    }

    #[test]
    fn ctrl_preserves_skirt_except_when_add_remove_resolves_to_relax() {
        let camera = transformed_camera();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        let step = |shift, command| {
            local_brush_ray_step(LocalBrushRayInput {
                camera: &camera,
                viewport_rect: viewport,
                pointer: viewport.center(),
                world_to_local: Affine3A::IDENTITY,
                local_per_world: 1.0,
                clip_plane: ClipPlane::default(),
                kind: SculptToolKind::AddRemove,
                tip: SculptTip::Ball,
                shift,
                command,
                radius_world_mm: 0.75,
                strength: 0.35,
                hold: false,
            })
            .expect("the pointer ray is valid")
        };
        assert!(step(false, true).preserve_skirt);
        let relax = step(true, true);
        assert_eq!(relax.mode, crate::sculpt_kernel::BrushMode::Relax);
        assert!(!relax.preserve_skirt);
    }
}

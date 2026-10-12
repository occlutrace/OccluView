//! Painting the sculpt cursor.

use super::super::{egui, live_viewport, mesh_editor_overlay, SceneContext};
use super::geometry::{orient_tool_axis, sculpt_face_normal};
use crate::app::workspace::id::SceneKey;
use crate::sculpt::sculpt_kernel::BrushMode;
use crate::sculpt::sculpt_tool::SculptTip;
use crate::sculpt::sculpt_worker::SculptSurfaceQueryError;
use glam::{DVec3, Mat4, Quat, Vec3};
use occluview_render::{
    sculpt_tool_length, PreparedSceneTopology, SculptToolShape, SculptToolUniform,
};

#[derive(Clone, Copy)]
struct CursorSurface {
    hit: occluview_core::ScenePickHit,
    normal: Vec3,
    topology: u64,
}

#[derive(Clone, Copy)]
struct CursorAxis {
    direction: Vec3,
    layer: occluview_core::SceneMeshId,
    at: f64,
}

impl SceneContext<'_> {
    /// Paint one atomic live-surface pose. Publication contention retains the
    /// latest pose; a real miss or a failed surface clears it immediately.
    #[expect(clippy::too_many_lines)]
    pub(in crate::app) fn paint_sculpt_cursor_impl(
        &self,
        ui: &egui::Ui,
        viewport_response: &egui::Response,
    ) {
        let Some(kind) = self.tools.sculpt.armed else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if !self.document.edit_mode.has_active_session() {
            self.publish_sculpt_cursor(None);
            return;
        }
        // Match cursor ownership to the drag and wait for preparation to finish.
        if !viewport_response.contains_pointer() {
            self.publish_sculpt_cursor(None);
            return;
        }
        let viewport_rect = viewport_response.rect;
        let Some(camera) = self.render.camera.as_ref() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let Some(pointer) = ui.ctx().pointer_hover_pos() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if !viewport_rect.contains(pointer) {
            self.publish_sculpt_cursor(None);
            return;
        }
        let Some(scene) = self.document.scene.as_ref() else {
            self.publish_sculpt_cursor(None);
            return;
        };
        let pose_id = egui::Id::new(("sculpt-cursor-surface", self.scene_key));
        let (hit, normal) = match self.sculpt_surface_query(viewport_rect, pointer) {
            Ok(Some((hit, local_normal))) => {
                let Some(normal) = sculpt_face_normal(scene, &hit, camera, local_normal) else {
                    self.publish_sculpt_cursor(None);
                    return;
                };
                let Some(entry) = scene.meshes().get(hit.layer_index) else {
                    self.publish_sculpt_cursor(None);
                    return;
                };
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        pose_id,
                        CursorSurface {
                            hit,
                            normal,
                            topology: entry.mesh.topology_id(),
                        },
                    )
                });
                (hit, normal)
            }
            Err(SculptSurfaceQueryError::Busy) => {
                let cached = ui
                    .ctx()
                    .data(|data| data.get_temp::<CursorSurface>(pose_id));
                let cached = cached.filter(|pose| {
                    self.sculpt_target_layer_id(scene) == Some(pose.hit.layer_id)
                        && scene
                            .meshes()
                            .get(pose.hit.layer_index)
                            .is_some_and(|entry| {
                                entry.visible
                                    && entry.id() == pose.hit.layer_id
                                    && entry.mesh.topology_id() == pose.topology
                            })
                });
                let Some(pose) = cached else {
                    self.publish_sculpt_cursor(None);
                    ui.ctx().request_repaint();
                    return;
                };
                // Display only: dabs still use their captured rays and the
                // kernel's current surface. A busy publication is not a miss.
                ui.ctx().request_repaint();
                (pose.hit, pose.normal)
            }
            Ok(None) | Err(SculptSurfaceQueryError::Unavailable) => {
                self.publish_sculpt_cursor(None);
                return;
            }
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            self.publish_sculpt_cursor(None);
            return;
        };
        if entry.id() != hit.layer_id {
            self.publish_sculpt_cursor(None);
            return;
        }
        let (shift, command) = ui.ctx().input(|input| {
            (
                input.modifiers.shift,
                input.modifiers.ctrl || input.modifiers.command,
            )
        });
        let tip = mesh_editor_overlay::sculpt_tip(ui.ctx(), self.scene_key);
        let radius_world = mesh_editor_overlay::sculpt_radius_mm(ui.ctx(), self.scene_key, tip);
        let strength_setting = mesh_editor_overlay::sculpt_strength(ui.ctx(), self.scene_key, kind);
        let mode = kind.brush_mode(shift, command);
        let color =
            animate_sculpt_cursor_color(ui.ctx(), self.scene_key, sculpt_cursor_color(mode));
        let strength = kind.dab_strength(strength_setting, shift);
        if mode == BrushMode::Remove && self.tools.sculpt.stroke.is_none() {
            if let Some(worker) = self.tools.sculpt.worker.as_ref().filter(|worker| {
                worker.layer_id == hit.layer_id && worker.topology_id == entry.mesh.topology_id()
            }) {
                let local = worker.world_to_local.transform_point3(hit.point);
                let center = DVec3::new(f64::from(local.x), f64::from(local.y), f64::from(local.z));
                let radius_mm = f64::from(radius_world * worker.local_per_world);
                let _ = worker.try_prime_wall_region(center, radius_mm, 128);
            }
        }
        let action = animate_sculpt_cursor_action(ui.ctx(), self.scene_key, mode);
        let shape = match tip {
            SculptTip::Ball => SculptToolShape::Cone,
            SculptTip::Knife => SculptToolShape::Knife,
            SculptTip::Cylinder => SculptToolShape::Cylinder,
        };
        let axis = self
            .tools
            .sculpt
            .stroke
            .as_ref()
            .and_then(|stroke| stroke.last_ray.as_ref())
            .and_then(|step| step.axis);
        let axis_world = axis
            .map(|axis| entry.transform.transform_vector3(Vec3::from_array(axis)))
            .filter(|axis| axis.is_finite() && axis.length_squared() > f32::EPSILON)
            .map(Vec3::normalize);
        let color_rgba = sculpt_cursor_body_rgba(color, strength);
        let direction = animate_sculpt_cursor_axis(ui.ctx(), self.scene_key, hit.layer_id, normal);
        let base_rotation = Quat::from_rotation_arc(Vec3::Z, direction);
        let tool_rotation = if tip == SculptTip::Knife {
            let fallback_axis = camera
                .view_direction()
                .cross(camera.view_up())
                .normalize_or_zero();
            orient_tool_axis(
                base_rotation,
                direction,
                axis_world.unwrap_or(fallback_axis),
            )
        } else {
            base_rotation
        };
        let tool_width = radius_world;
        let target_height = sculpt_cursor_height(mode, strength, radius_world);
        let tool_length = animate_sculpt_cursor_height(ui.ctx(), self.scene_key, target_height);
        let tool_model = Mat4::from_scale_rotation_translation(
            Vec3::new(tool_width, radius_world, tool_length),
            tool_rotation,
            hit.point + direction * 0.02,
        );
        self.publish_sculpt_cursor(Some(live_viewport::SculptCursor {
            target_index: hit.layer_index,
            topology: PreparedSceneTopology::from_mesh(&entry.mesh),
            tool: SculptToolUniform {
                model: tool_model.to_cols_array(),
                color: color_rgba,
                // Strength changes the body's saturation. There is no second
                // surface pass, ring, or material tint over the scan.
                opacity: SCULPT_TOOL_OPACITY,
                shape: shape as u32,
                action,
            },
        }));
        // One crosshair marks the exact contact; the volume shows the reach.
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    pub(in crate::app) fn publish_sculpt_cursor(
        &self,
        cursor: Option<live_viewport::SculptCursor>,
    ) {
        if cursor.is_none() {
            self.ui.repaint_ctx.data_mut(|data| {
                data.remove::<CursorSurface>(egui::Id::new((
                    "sculpt-cursor-surface",
                    self.scene_key,
                )));
                data.remove::<CursorAxis>(egui::Id::new(("sculpt-cursor-axis", self.scene_key)));
            });
        }
        let Some(viewport) = self.render.live_viewport.as_ref() else {
            return;
        };
        if let Ok(mut viewport) = viewport.lock() {
            viewport.set_sculpt_cursor(cursor);
        }
    }
}

/// Keep the four surface operations visually distinct.
const SCULPT_CURSOR_TRANSITION_SEC: f32 = 0.07;
const SCULPT_IRON_HEIGHT_SHARE: f32 = 0.22;
/// Source opacity of the translucent tool body. The fragment shader shapes it
/// with the Fresnel rim and axial fade; strength changes the body's saturation.
const SCULPT_TOOL_OPACITY: f32 = 0.5;

pub(super) fn sculpt_cursor_color(mode: BrushMode) -> egui::Color32 {
    match mode {
        BrushMode::Add => egui::Color32::from_rgb(22, 136, 74),
        BrushMode::Remove => egui::Color32::from_rgb(194, 58, 46),
        BrushMode::Relax => egui::Color32::from_rgb(43, 102, 177),
        BrushMode::Smooth => egui::Color32::from_rgb(60, 67, 72),
    }
}

/// Brush colour for the surface footprint and the tool body, in linear space.
///
/// The UI ink colours are dark once converted to linear, and a surface light
/// needs a pale tint so the tool colour does not erase the strength signal.
/// Every brush colour is therefore washed a fixed share toward white, which is
/// what keeps the footprint a pale mark on a bright surface instead of a
/// saturated dark one that reads as damage.
pub(super) fn sculpt_cursor_linear_rgba(color: egui::Color32) -> [f32; 4] {
    /// Share of the way to white, in linear space.
    const WHITE_WASH: f32 = 0.75;
    let linear = egui::Rgba::from(color);
    let washed = |channel: f32| channel + (1.0 - channel) * WHITE_WASH;
    [
        washed(linear.r()),
        washed(linear.g()),
        washed(linear.b()),
        linear.a(),
    ]
}

pub(super) fn sculpt_cursor_height(mode: BrushMode, strength: f32, radius: f32) -> f32 {
    if matches!(mode, BrushMode::Add | BrushMode::Remove) {
        sculpt_tool_length(strength) * radius.max(0.05)
    } else {
        (radius * SCULPT_IRON_HEIGHT_SHARE).max(0.05)
    }
}

/// Keep the operation's hue while making the strength readable in the body.
fn sculpt_cursor_body_rgba(color: egui::Color32, strength: f32) -> [f32; 4] {
    let low = sculpt_cursor_linear_rgba(color);
    let high = egui::Rgba::from(color).to_array();
    let saturation = strength.clamp(0.0, 1.0).sqrt();
    [
        low[0] + (high[0] - low[0]) * saturation,
        low[1] + (high[1] - low[1]) * saturation,
        low[2] + (high[2] - low[2]) * saturation,
        1.0,
    ]
}

/// Ease only the display axis, as in the shared CAD cursor. The contact point
/// stays exact; a new layer or a turn beyond 50 degrees snaps to its surface.
fn animate_sculpt_cursor_axis(
    context: &egui::Context,
    scene_key: SceneKey,
    layer: occluview_core::SceneMeshId,
    target: Vec3,
) -> Vec3 {
    let id = egui::Id::new(("sculpt-cursor-axis", scene_key));
    let now = context.input(|input| input.time);
    let previous = context.data(|data| data.get_temp::<CursorAxis>(id));
    let direction = previous
        .filter(|pose| {
            pose.layer == layer && pose.direction.dot(target) >= 50.0_f32.to_radians().cos()
        })
        .map_or(target, |pose| {
            let dt = (now - pose.at).clamp(0.0, 0.064) as f32;
            pose.direction
                .lerp(target, 1.0 - (-dt / 0.04).exp())
                .normalize_or_zero()
        });
    if direction.dot(target) < 0.99995 {
        context.request_repaint_after(std::time::Duration::from_millis(16));
    }
    context.data_mut(|data| {
        data.insert_temp(
            id,
            CursorAxis {
                direction,
                layer,
                at: now,
            },
        )
    });
    direction
}

fn animate_sculpt_cursor_height(context: &egui::Context, scene_key: SceneKey, target: f32) -> f32 {
    context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-height", scene_key)),
        target,
        SCULPT_CURSOR_TRANSITION_SEC,
    )
}

fn animate_sculpt_cursor_color(
    context: &egui::Context,
    scene_key: SceneKey,
    target: egui::Color32,
) -> egui::Color32 {
    let rgba = egui::Rgba::from(target).to_array();
    let red = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-red", scene_key)),
        rgba[0],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    let green = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-green", scene_key)),
        rgba[1],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    let blue = context.animate_value_with_time(
        egui::Id::new(("sculpt-cursor-blue", scene_key)),
        rgba[2],
        SCULPT_CURSOR_TRANSITION_SEC,
    );
    egui::Rgba::from_rgba_unmultiplied(red, green, blue, 1.0).into()
}

fn animate_sculpt_cursor_action(
    context: &egui::Context,
    scene_key: SceneKey,
    mode: BrushMode,
) -> [f32; 2] {
    let target = sculpt_cursor_action(mode);
    [
        context.animate_value_with_time(
            egui::Id::new(("sculpt-cursor-invert", scene_key)),
            target[0],
            SCULPT_CURSOR_TRANSITION_SEC,
        ),
        context.animate_value_with_time(
            egui::Id::new(("sculpt-cursor-flat", scene_key)),
            target[1],
            SCULPT_CURSOR_TRANSITION_SEC,
        ),
    ]
}

pub(super) fn sculpt_cursor_action(mode: BrushMode) -> [f32; 2] {
    match mode {
        BrushMode::Add => [0.0, 0.0],
        BrushMode::Remove => [1.0, 0.0],
        BrushMode::Relax | BrushMode::Smooth => [0.0, 1.0],
    }
}

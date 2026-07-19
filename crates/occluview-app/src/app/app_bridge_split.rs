//! Viewport orchestration for the interactive Bridge Split separator disc.

use super::{egui, layers_overlay, OccluViewApp, Scene};
use crate::bridge_split::{apply_preview_to_scene, BridgeSplitMode, BridgeSplitTarget};
use crate::bridge_split_overlay::{
    paint_separator_disc, show_panel, BridgeSplitPanelAction, BridgeSplitPanelState, SeparatorDisc,
};
use crate::cut_manipulator::{CutCursor, CutFrameInput, SurfaceSample};
use crate::edit_mode::state::{BusyFinish, EditModeCommand};
use crate::section_view::SectionViewFrame;
use crate::viewer::{project_world_to_viewport, viewport_ray};
use occluview_core::{Camera, SceneMesh, SceneMeshId};

struct BridgeFrameContext<'a> {
    camera: &'a Camera,
    scene: &'a Scene,
    entry: &'a SceneMesh,
    viewport_rect: egui::Rect,
}

struct BridgeSectionInput<'a> {
    ui: &'a mut egui::Ui,
    ctx: &'a egui::Context,
    frame_context: &'a BridgeFrameContext<'a>,
    frame: &'a CutFrameInput,
    panel_zoom_notches: f32,
}

impl OccluViewApp {
    pub(super) fn begin_bridge_split_from_layer(&mut self, scene: &Scene, layer_id: SceneMeshId) {
        if self.edit_mode.has_active_session() {
            self.status_message = Some("Finish or cancel mesh editing first".to_string());
            return;
        }
        if self.bridge_split.session().mode() != BridgeSplitMode::Off {
            self.status_message = Some("Bridge split is already active".to_string());
            return;
        }
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            self.status_message = Some("Bridge split target is no longer available".to_string());
            return;
        };
        if !entry.visible || entry.mesh.is_point_cloud() || entry.mesh.triangle_count() == 0 {
            self.status_message = Some("Bridge split requires a visible triangle mesh".to_string());
            return;
        }

        self.cut_view.disable();
        self.measure.disarm();
        self.mesh_selection_drag = None;
        self.bridge_split.start(entry);
        self.bridge_split_disc.arm();
        self.bridge_split_section.reset();
        self.needs_render = true;
        self.status_message = Some("Bridge split: place separator disc".to_string());
        self.repaint_ctx.request_repaint();
    }

    pub(super) fn bridge_split_active(&self) -> bool {
        self.bridge_split.session().mode() != BridgeSplitMode::Off
    }

    pub(super) fn show_bridge_split_overlay_impl(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.bridge_split_active() {
            return false;
        }
        let Some(scene) = self.scene.clone() else {
            self.cancel_bridge_split("Bridge split canceled: scene closed");
            return false;
        };
        let Some(camera) = self.camera else {
            self.cancel_bridge_split("Bridge split canceled: camera unavailable");
            return false;
        };
        let Some(entry) = live_bridge_entry(&scene, self.bridge_split.session().target()) else {
            self.cancel_bridge_split("Bridge split canceled: source mesh changed");
            return false;
        };

        self.poll_bridge_split_result(entry, ctx);
        if self.consume_bridge_split_escape(ctx) {
            self.cancel_bridge_split("Bridge split canceled");
            return true;
        }

        let frame_context = BridgeFrameContext {
            camera: &camera,
            scene: &scene,
            entry,
            viewport_rect: response.rect,
        };
        let (frame, panel_zoom_notches) = self.build_bridge_split_frame(ctx, &frame_context);
        let update = self.update_bridge_split_disc(&frame, entry, ctx);
        let section_consumed = self.show_bridge_split_section(BridgeSectionInput {
            ui,
            ctx,
            frame_context: &frame_context,
            frame: &frame,
            panel_zoom_notches,
        });
        let panel_action = self.show_bridge_split_panel(ctx, response.rect);
        if self.apply_bridge_split_panel_action(panel_action, &scene, entry, ctx) {
            return true;
        }
        if matches!(
            self.bridge_split.session().mode(),
            BridgeSplitMode::PlantedPending
        ) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        update.consumed_pointer || panel_action.is_some() || section_consumed
    }

    fn update_bridge_split_disc(
        &mut self,
        frame: &CutFrameInput,
        entry: &SceneMesh,
        ctx: &egui::Context,
    ) -> crate::cut_manipulator::CutUpdate {
        let update = self.bridge_split_disc.update(frame);
        match update.cursor {
            CutCursor::Grab => ctx.set_cursor_icon(egui::CursorIcon::Grab),
            CutCursor::Grabbing => ctx.set_cursor_icon(egui::CursorIcon::Grabbing),
            CutCursor::Default => {}
        }
        if update.planted {
            if let Some(pose) = self.bridge_split_disc.pose() {
                if self
                    .bridge_split
                    .session_mut()
                    .plant(to_bridge_pose(pose))
                    .is_some()
                {
                    self.submit_bridge_preview(entry);
                }
            }
        } else if update.pose_changed {
            self.sync_bridge_split_pose(entry);
        }
        update
    }

    fn show_bridge_split_section(&mut self, input: BridgeSectionInput<'_>) -> bool {
        let BridgeSectionInput {
            ui,
            ctx,
            frame_context,
            frame,
            panel_zoom_notches,
        } = input;
        let section_frame = self
            .bridge_split_disc
            .pose()
            .and_then(|pose| SectionViewFrame::new(pose, pose.plane_normal));
        if self.bridge_split_section.sync(section_frame) {
            self.needs_render = true;
            ctx.request_repaint();
        }
        if panel_zoom_notches != 0.0
            && self.bridge_split_section.zoom_at_cursor(
                frame_context.viewport_rect,
                frame.pointer,
                panel_zoom_notches,
            )
        {
            self.needs_render = true;
            ctx.request_repaint();
        }
        if let Some(pose) = self.bridge_split_disc.pose() {
            paint_separator_disc(
                ui.painter(),
                frame_context.camera,
                frame_context.viewport_rect,
                SeparatorDisc {
                    pose,
                    kerf_mm: self.bridge_split.session().kerf_mm(),
                    mode: self.bridge_split.session().mode(),
                },
            );
        }
        let section = self.section_for_plane(
            frame_context.scene,
            self.bridge_split_section.section_plane(),
        );
        let color_for = super::app_cut_measure::contour_tint(frame_context.scene);
        if let Some(section) = section.as_deref() {
            crate::cut_overlay::paint_section_contour(
                ui.painter(),
                frame_context.camera,
                frame_context.viewport_rect,
                section,
                &color_for,
            );
        }
        self.maybe_render_bridge_split_section(ctx);
        let panel = self.bridge_split_section.show(
            ui,
            frame_context.viewport_rect,
            section.as_deref(),
            &color_for,
        );
        if panel.viewport_needs_render {
            self.needs_render = true;
            ctx.request_repaint();
        }
        panel.consumed_pointer
    }

    fn show_bridge_split_panel(
        &self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
    ) -> Option<BridgeSplitPanelAction> {
        show_panel(
            ctx,
            viewport_rect,
            BridgeSplitPanelState {
                mode: self.bridge_split.session().mode(),
                kerf_mm: self.bridge_split.session().kerf_mm(),
                disc_radius_mm: self
                    .bridge_split_disc
                    .pose()
                    .map_or(crate::cut_manipulator::DEFAULT_DISC_RADIUS_MM, |pose| {
                        pose.radius_mm
                    }),
                can_apply: self.bridge_split.session().can_apply(),
                failure: self.bridge_split.session().failure(),
            },
        )
    }

    fn apply_bridge_split_panel_action(
        &mut self,
        action: Option<BridgeSplitPanelAction>,
        scene: &Scene,
        entry: &SceneMesh,
        ctx: &egui::Context,
    ) -> bool {
        match action {
            Some(BridgeSplitPanelAction::SetKerfMm(kerf_mm)) => {
                if self
                    .bridge_split
                    .session_mut()
                    .set_kerf_mm(kerf_mm)
                    .is_some()
                {
                    self.submit_bridge_preview(entry);
                }
            }
            Some(BridgeSplitPanelAction::SetDiscRadiusMm(radius_mm)) => {
                if self.bridge_split_disc.set_radius_mm(radius_mm) {
                    self.sync_bridge_split_pose(entry);
                    self.needs_render = true;
                    ctx.request_repaint();
                }
            }
            Some(BridgeSplitPanelAction::Apply) => self.apply_bridge_split_preview(scene, ctx),
            Some(BridgeSplitPanelAction::Cancel) => {
                self.cancel_bridge_split("Bridge split canceled");
                return true;
            }
            None => {}
        }
        false
    }

    fn poll_bridge_split_result(&mut self, entry: &SceneMesh, ctx: &egui::Context) {
        if self
            .bridge_split
            .poll(Some(BridgeSplitTarget::capture(entry)))
        {
            self.needs_render = true;
            ctx.request_repaint();
        }
    }

    fn submit_bridge_preview(&mut self, entry: &SceneMesh) {
        if self.bridge_split.submit_current_request(entry) {
            self.status_message = Some("Bridge split: calculating".to_string());
            self.repaint_ctx.request_repaint();
        }
    }

    fn sync_bridge_split_pose(&mut self, entry: &SceneMesh) {
        let pose = self.bridge_split_disc.pose().map(to_bridge_pose);
        if self.bridge_split_disc.is_planted() {
            if let Some(pose) = pose {
                if self.bridge_split.session_mut().update_pose(pose).is_some() {
                    self.submit_bridge_preview(entry);
                }
            }
        } else {
            self.bridge_split.session_mut().set_follow_pose(pose);
        }
    }

    fn consume_bridge_split_escape(&self, ctx: &egui::Context) -> bool {
        let dialogs_open = self.close_guard_open
            || self.pending_replace_open.is_some()
            || self.app_error.is_some()
            || self.about_window == super::AboutWindowState::Open;
        !dialogs_open
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    }

    fn build_bridge_split_frame(
        &mut self,
        ctx: &egui::Context,
        frame_context: &BridgeFrameContext<'_>,
    ) -> (CutFrameInput, f32) {
        let BridgeFrameContext {
            camera,
            scene,
            entry,
            viewport_rect,
        } = frame_context;
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let over_rect = pointer.is_some_and(|point| viewport_rect.contains(point));
        let layers_rect = layers_overlay::layer_overlay_rect(*viewport_rect, scene.meshes().len());
        let over_section_panel = self.bridge_split_section.slice_visible()
            && pointer.is_some_and(|point| {
                crate::cut_ruler::section_panel_contains(*viewport_rect, point)
            });
        let gizmo_avoid = self
            .bridge_split_section
            .slice_visible()
            .then(|| crate::cut_ruler::section_panel_rect(*viewport_rect))
            .flatten();
        let over_gizmo = pointer.is_some_and(|point| {
            crate::viewer::axis_gizmo::axis_gizmo_footprint(*viewport_rect, gizmo_avoid)
                .contains(point)
        });
        let over_egui = pointer.is_some_and(|point| {
            layers_rect.contains(point)
                || ctx
                    .layer_id_at(point)
                    .is_some_and(|layer| layer.order != egui::Order::Background)
        }) || over_section_panel
            || over_gizmo;
        let over_viewport = over_rect && !over_egui;
        let ctrl = ctx.input(|input| input.modifiers.command);
        let raw_scroll = ctx.input(|input| input.raw_scroll_delta.y);
        let (wheel_notches, panel_zoom_notches) = if over_section_panel && raw_scroll != 0.0 {
            ctx.input_mut(|input| {
                input.raw_scroll_delta = egui::Vec2::ZERO;
                input.smooth_scroll_delta = egui::Vec2::ZERO;
            });
            let notches = raw_scroll / super::app_cut_measure::CUT_WHEEL_PX_PER_NOTCH;
            if ctrl {
                (notches, 0.0)
            } else {
                (0.0, notches)
            }
        } else {
            (0.0, 0.0)
        };
        let eye = camera.eye();
        let view_dir = camera.view_direction();
        let camera_up = camera.view_up();
        let camera_right = view_dir.cross(camera_up).normalize_or_zero();
        let ray_origin = pointer
            .and_then(|point| viewport_ray(camera, *viewport_rect, point))
            .map_or(eye, |(origin, _)| origin);
        let surface_hit = (!self.bridge_split_disc.is_planted() && over_viewport)
            .then(|| {
                pointer.and_then(|point| {
                    bridge_surface_sample(camera, *viewport_rect, point, scene, entry.id())
                })
            })
            .flatten();
        let pose = self.bridge_split_disc.pose();
        let disc_center_screen = pose.and_then(|disc| {
            project_world_to_viewport(camera, *viewport_rect, disc.center).map(|(screen, _)| screen)
        });
        let disc_radius_screen = pose.map_or(0.0, |disc| {
            disc.radius_mm * viewport_rect.height().max(1.0)
                / camera.orthographic_height.max(1.0e-3)
        });
        let frame = CutFrameInput {
            pointer,
            over_viewport,
            primary_pressed: ctx
                .input(|input| input.pointer.button_pressed(egui::PointerButton::Primary)),
            primary_down: ctx
                .input(|input| input.pointer.button_down(egui::PointerButton::Primary)),
            ctrl,
            escape: false,
            flip: false,
            wheel_notches,
            eye,
            view_dir,
            camera_right,
            camera_up,
            ray_origin,
            surface_hit,
            disc_center_screen,
            disc_radius_screen,
        };
        (frame, panel_zoom_notches)
    }

    fn apply_bridge_split_preview(&mut self, scene: &Scene, ctx: &egui::Context) {
        let Some(preview) = self.bridge_split.session().preview().cloned() else {
            return;
        };
        let surface_result = !preview.result.report.parts_closed;
        let Some(entry) = live_bridge_entry(scene, Some(preview.guard.target)) else {
            self.cancel_bridge_split("Bridge split canceled: source mesh changed");
            return;
        };
        let Some(token) =
            self.edit_mode
                .begin_scene_edit(scene, entry.id(), EditModeCommand::BridgeSplit)
        else {
            self.status_message = Some("Bridge split is temporarily unavailable".to_string());
            return;
        };
        let undoable = self.edit_mode.last_edit_undoable();
        let applied = apply_preview_to_scene(scene, preview.guard.target, &preview.result);
        let Ok(applied) = applied else {
            let _ = self.edit_mode.finish_layer_edit_noop(token);
            self.cancel_bridge_split("Bridge split preview is no longer valid");
            return;
        };
        if self
            .edit_mode
            .finish_scene_edit_success(token, &applied.scene)
            != BusyFinish::Applied
        {
            self.status_message = Some("Bridge split was not applied".to_string());
            return;
        }
        let source_layer_id = applied.source_layer_id;
        let part_b_layer_id = applied.part_b_layer_id;
        self.commit_structural_scene(Some(scene), applied.scene, ctx);
        self.mark_mesh_edits_unsaved(source_layer_id);
        self.mark_mesh_edits_unsaved(part_b_layer_id);
        self.bridge_split.cancel();
        self.bridge_split_disc.disarm();
        self.bridge_split_section.reset();
        self.status_message = Some(if surface_result {
            "Bridge split complete (surface result; natural borders preserved)".to_string()
        } else if undoable {
            "Bridge split complete".to_string()
        } else {
            "Bridge split complete (not undoable: snapshot too large)".to_string()
        });
        ctx.request_repaint();
    }

    fn cancel_bridge_split(&mut self, message: &str) {
        self.bridge_split.cancel();
        self.bridge_split_disc.disarm();
        self.bridge_split_section.reset();
        self.mesh_selection_drag = None;
        self.status_message = Some(message.to_string());
        self.needs_render = true;
        self.repaint_ctx.request_repaint();
    }
}

fn live_bridge_entry(scene: &Scene, target: Option<BridgeSplitTarget>) -> Option<&SceneMesh> {
    let target = target?;
    let entry = scene
        .meshes()
        .iter()
        .find(|entry| entry.id() == target.layer_id)?;
    (entry.visible
        && !entry.mesh.is_point_cloud()
        && entry.mesh.triangle_count() > 0
        && BridgeSplitTarget::capture(entry) == target)
        .then_some(entry)
}

fn bridge_surface_sample(
    camera: &Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    scene: &Scene,
    layer_id: SceneMeshId,
) -> Option<SurfaceSample> {
    let (origin, direction) = viewport_ray(camera, viewport_rect, pointer)?;
    let hit = scene.pick_layer_ray_hit(origin, direction, layer_id)?;
    let entry = scene.meshes().get(hit.layer_index)?;
    let normal = super::app_cut_measure::triangle_world_normal(entry, hit.triangle_index)?;
    Some(SurfaceSample {
        point: hit.point,
        normal,
        arch_frame: super::app_cut_measure::world_arch_frame(entry),
    })
}

fn to_bridge_pose(pose: crate::cut_manipulator::DiscPose) -> crate::bridge_split::BridgeSplitPose {
    crate::bridge_split::BridgeSplitPose {
        center: pose.center,
        normal: pose.plane_normal,
        radius_mm: pose.radius_mm,
    }
}

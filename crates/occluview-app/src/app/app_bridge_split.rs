//! Viewport orchestration for the interactive Bridge Split separator disc.

use super::{egui, OccluViewApp, Scene};
use crate::bridge_split::{apply_preview_to_scene, BridgeSplitMode, BridgeSplitTarget};
use crate::bridge_split_overlay::{
    paint_separator_disc, show_panel, BridgeSplitPanelAction, BridgeSplitPanelState, SeparatorDisc,
};
use crate::cut_manipulator::{CutCursor, CutFrameInput, SurfaceSample};
use crate::edit_mode::state::{BusyFinish, EditModeCommand};
use crate::section_view::{SectionMainView, SectionViewFrame};
use crate::viewer::viewport_ray;
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
        if self.document.edit_mode.has_active_session() {
            self.ui.status_message = Some(self.ui.locale.tr("edit-session-busy"));
            return;
        }
        if self.tools.bridge_split.session().mode() != BridgeSplitMode::Off {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-active"));
            return;
        }
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-target-gone"));
            return;
        };
        if !entry.visible || entry.mesh.is_point_cloud() || entry.mesh.triangle_count() == 0 {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-needs-mesh"));
            return;
        }

        // Size the starting disc to the object: a fraction of its world bounding
        // diagonal, floored at the historical default, so a big arch gets a disc
        // that usually already covers the connector instead of the tiny minimum.
        let object_radius = {
            let world_diagonal = entry.mesh.bbox_cached().size().length()
                * crate::sculpt_tool::mean_uniform_scale(&entry.transform);
            (0.22 * world_diagonal).max(crate::cut_manipulator::DEFAULT_DISC_RADIUS_MM)
        };

        self.tools.cut_view.disable();
        self.tools.measure.disarm();
        self.document.mesh_selection_drag = None;
        self.tools.bridge_split.start(entry);
        self.tools.bridge_split_disc.arm_with_radius(object_radius);
        // Build the picking BVH off-thread now (shared via Arc<OnceLock>) so the
        // first hover/plant doesn't freeze the UI building it on a big scan.
        //
        // The thread takes the one mesh it warms, not the case it came from:
        // an `Arc<Scene>` held here made every scene edit on the UI thread copy
        // the whole case for as long as the warm ran, which is most of a
        // second on a full arch. The mesh is shared, so this costs a pointer
        // and the thread warms the very cell the scene will read.
        let target_mesh = self
            .document
            .scene
            .as_ref()
            .and_then(|scene| scene.meshes().iter().find(|e| e.id() == layer_id))
            .map(|entry| entry.mesh.clone());
        if let Some(mesh) = target_mesh {
            std::thread::spawn(move || mesh.warm_bvh());
        }
        self.tools.bridge_split_section.reset();
        self.render.invalidation.overlay_tools_changed();
        self.ui.status_message = Some(self.ui.locale.tr("bridge-place-disc"));
        self.ui.repaint_ctx.request_repaint();
    }

    pub(super) fn show_bridge_split_overlay(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.tools.bridge_split_active() {
            return false;
        }
        let Some(scene) = self.document.scene.clone() else {
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled-scene"));
            return false;
        };
        let Some(camera) = self.render.camera else {
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled-camera"));
            return false;
        };
        let Some(entry) = live_bridge_entry(&scene, self.tools.bridge_split.session().target())
        else {
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled-changed"));
            return false;
        };

        self.poll_bridge_split_result(entry, ctx);
        if self.consume_bridge_split_escape(ctx) {
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled"));
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
            self.tools.bridge_split.session().mode(),
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
        let update = self.tools.bridge_split_disc.update(frame);
        match update.cursor {
            CutCursor::Grab => ctx.set_cursor_icon(egui::CursorIcon::Grab),
            CutCursor::Grabbing => ctx.set_cursor_icon(egui::CursorIcon::Grabbing),
            CutCursor::Default => {}
        }
        if update.planted {
            if let Some(pose) = self.tools.bridge_split_disc.pose() {
                if self
                    .tools
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
            .tools
            .bridge_split_disc
            .pose()
            .and_then(|pose| SectionViewFrame::new(pose, pose.plane_normal));
        let frame_changed = self.tools.bridge_split_section.sync(section_frame);
        let orientation_changed = self
            .tools
            .bridge_split_section
            .sync_main_view(SectionMainView::from_camera(*frame_context.camera));
        if frame_changed || orientation_changed {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
        if panel_zoom_notches != 0.0
            && self.tools.bridge_split_section.zoom_at_cursor(
                frame_context.viewport_rect,
                frame.pointer,
                panel_zoom_notches,
            )
        {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
        if let Some(pose) = self.tools.bridge_split_disc.pose() {
            paint_separator_disc(
                ui.painter(),
                frame_context.camera,
                frame_context.viewport_rect,
                SeparatorDisc {
                    pose,
                    kerf_mm: self.tools.bridge_split.session().kerf_mm(),
                    mode: self.tools.bridge_split.session().mode(),
                },
            );
        }
        let section = self.section_for_plane(
            frame_context.scene,
            self.tools.bridge_split_section.section_plane(),
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
        let panel = self.tools.bridge_split_section.show(
            ui,
            frame_context.viewport_rect,
            section.as_deref(),
            &color_for,
            &self.ui.locale,
        );
        if panel.viewport_needs_render {
            self.render.invalidation.overlay_tools_changed();
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
                mode: self.tools.bridge_split.session().mode(),
                kerf_mm: self.tools.bridge_split.session().kerf_mm(),
                disc_radius_mm: self
                    .tools
                    .bridge_split_disc
                    .pose()
                    .map_or(crate::cut_manipulator::DEFAULT_DISC_RADIUS_MM, |pose| {
                        pose.radius_mm
                    }),
                can_apply: self.tools.bridge_split.session().can_apply(),
                failure: self.tools.bridge_split.session().failure(),
            },
            &self.ui.locale,
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
                    .tools
                    .bridge_split
                    .session_mut()
                    .set_kerf_mm(kerf_mm)
                    .is_some()
                {
                    self.submit_bridge_preview(entry);
                }
            }
            Some(BridgeSplitPanelAction::SetDiscRadiusMm(radius_mm)) => {
                if self.tools.bridge_split_disc.set_radius_mm(radius_mm) {
                    self.sync_bridge_split_pose(entry);
                    self.render.invalidation.overlay_tools_changed();
                    ctx.request_repaint();
                }
            }
            Some(BridgeSplitPanelAction::Apply) => self.apply_bridge_split_preview(scene, ctx),
            Some(BridgeSplitPanelAction::Cancel) => {
                self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled"));
                return true;
            }
            None => {}
        }
        false
    }

    fn poll_bridge_split_result(&mut self, entry: &SceneMesh, ctx: &egui::Context) {
        if self
            .tools
            .bridge_split
            .poll(Some(BridgeSplitTarget::capture(entry)))
        {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
    }

    fn submit_bridge_preview(&mut self, entry: &SceneMesh) {
        if self.tools.bridge_split.submit_current_request(entry) {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-calculating"));
            self.ui.repaint_ctx.request_repaint();
        }
    }

    fn sync_bridge_split_pose(&mut self, entry: &SceneMesh) {
        let pose = self.tools.bridge_split_disc.pose().map(to_bridge_pose);
        if self.tools.bridge_split_disc.is_planted() {
            if let Some(pose) = pose {
                if self
                    .tools
                    .bridge_split
                    .session_mut()
                    .update_pose(pose)
                    .is_some()
                {
                    self.submit_bridge_preview(entry);
                }
            }
        } else {
            self.tools.bridge_split.session_mut().set_follow_pose(pose);
        }
    }

    fn consume_bridge_split_escape(&self, ctx: &egui::Context) -> bool {
        !self.ui.modal_dialog_open()
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
        // Same question, same answer: see `OccluViewApp::viewport_pointer`.
        // Not a second copy: the gizmo's avoid-rect has to come from the call
        // that painted the gizmo, as the cut tool's own comment warns.
        let super::app_cut_measure::ViewportPointer {
            pointer,
            over_section_panel,
            over_viewport,
        } = self.viewport_pointer(
            ctx,
            *viewport_rect,
            scene,
            self.tools.bridge_split_section.slice_visible(),
        );
        let ctrl = ctx.input(|input| input.modifiers.command);
        // The wheel scoping and the camera basis are the cut tool's, not a
        // second copy of them: an operator meets one wheel gesture over one
        // Section panel, whichever disc tool put it there.
        let (wheel_notches, panel_zoom_notches) =
            super::disc_frame::section_panel_wheel(ctx, over_section_panel, ctrl);
        let super::disc_frame::DiscViewGeometry {
            eye,
            view_dir,
            camera_up,
            camera_right,
            ray_origin,
        } = super::disc_frame::disc_view_geometry(camera, *viewport_rect, pointer);
        let surface_hit = (!self.tools.bridge_split_disc.is_planted() && over_viewport)
            .then(|| {
                pointer.and_then(|point| {
                    bridge_surface_sample(camera, *viewport_rect, point, scene, entry.id())
                })
            })
            .flatten();
        let (disc_center_screen, disc_radius_screen) = super::disc_frame::disc_screen_placement(
            camera,
            *viewport_rect,
            self.tools.bridge_split_disc.pose(),
        );
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
        let Some(preview) = self.tools.bridge_split.session().preview().cloned() else {
            return;
        };
        let surface_result = !preview.result.report.parts_closed;
        let Some(entry) = live_bridge_entry(scene, Some(preview.guard.target)) else {
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-canceled-changed"));
            return;
        };
        let Some(token) = self.document.edit_mode.begin_scene_edit(
            scene,
            entry.id(),
            EditModeCommand::BridgeSplit,
        ) else {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-unavailable"));
            return;
        };
        let undoable = self.document.edit_mode.last_edit_undoable();
        let applied = apply_preview_to_scene(scene, preview.guard.target, &preview.result);
        let Ok(applied) = applied else {
            let _ = self.document.edit_mode.finish_layer_edit_noop(token);
            self.cancel_bridge_split(&self.ui.locale.tr("bridge-preview-stale"));
            return;
        };
        if self
            .document
            .edit_mode
            .finish_scene_edit_success(token, &applied.scene)
            != BusyFinish::Applied
        {
            self.ui.status_message = Some(self.ui.locale.tr("bridge-not-applied"));
            return;
        }
        let source_layer_id = applied.source_layer_id;
        let part_b_layer_id = applied.part_b_layer_id;
        self.commit_structural_scene(Some(scene), applied.scene, ctx);
        self.document.mark_mesh_edits_unsaved(source_layer_id);
        self.document.mark_mesh_edits_unsaved(part_b_layer_id);
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.ui.status_message = Some(if surface_result {
            self.ui.locale.tr("bridge-complete-surface")
        } else if undoable {
            self.ui.locale.tr("bridge-complete")
        } else {
            self.ui.locale.tr("bridge-complete-locked")
        });
        ctx.request_repaint();
    }

    fn cancel_bridge_split(&mut self, message: &str) {
        self.tools.bridge_split.cancel();
        self.tools.bridge_split_disc.disarm();
        self.tools.bridge_split_section.reset();
        self.document.mesh_selection_drag = None;
        self.ui.status_message = Some(message.to_string());
        self.render.invalidation.overlay_tools_changed();
        self.ui.repaint_ctx.request_repaint();
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

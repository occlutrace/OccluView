//! Cut View ownership, pointer routing, and viewport overlay handling.
//!
//! The thickness tool can plant a Cut View, so both workflows share the
//! viewport ownership and section cache defined here.

use super::{egui, pick_scene_hit, CutTool, Scene, SceneContext};
use crate::cut::cut_manipulator::{ArchFrame, CutCursor, CutFrameInput, SurfaceSample};
use crate::cut::cut_overlay;
use crate::cut::section_view::SectionMainView;
use crate::measure::measure_tool::{self, ThicknessProbe, ThicknessReading};
use glam::{Vec3, Vec3A};
use occluview_mesh_edit::scene::SceneSection;
use std::sync::Arc;

/// Wheel travel (screen px) mapped to one disc-radius scale notch in cut mode.
pub(super) const CUT_WHEEL_PX_PER_NOTCH: f32 = 50.0;

/// Sample the surface under the cursor for the follow disc: the world hit
/// point, the averaged world normal of the hit triangle, and the hit mesh's
/// own principal-axis frame (the stable signal the disc orientation prefers).
fn surface_sample(
    camera: &occluview_core::Camera,
    viewport_rect: egui::Rect,
    pointer: egui::Pos2,
    scene: &Scene,
) -> Option<SurfaceSample> {
    let hit = pick_scene_hit(camera, viewport_rect, pointer, scene)?;
    let entry = scene.meshes().get(hit.layer_index)?;
    if entry.id() != hit.layer_id {
        return None;
    }
    let normal = triangle_world_normal(entry, hit.triangle_index).unwrap_or(Vec3::Y);
    Some(SurfaceSample {
        point: hit.point,
        normal,
        arch_frame: world_arch_frame(entry),
    })
}

/// A per-layer contour tint keyed by layer id, shared by the viewport and slice.
pub(super) fn contour_tint(
    scene: &Scene,
) -> impl Fn(occluview_core::SceneMeshId) -> egui::Color32 + '_ {
    move |id| {
        let tint = scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == id)
            .map_or([0.55, 0.6, 0.68, 1.0], |entry| entry.tint);
        cut_overlay::contour_color(tint)
    }
}

pub(super) fn triangle_world_normal(
    entry: &occluview_core::SceneMesh,
    triangle_index: usize,
) -> Option<Vec3> {
    let base = triangle_index.checked_mul(3)?;
    let tri = entry.mesh.indices().get(base..base + 3)?;
    let vertices = entry.mesh.vertices();
    let mut sum = Vec3::ZERO;
    for &raw in tri {
        let vertex = vertices.get(raw as usize)?;
        sum += Vec3::from_array(vertex.normal);
    }
    let normal_matrix = entry.transform.matrix3.inverse().transpose();
    let world = normal_matrix * Vec3A::from(sum);
    Some(Vec3::from(world).normalize_or(Vec3::Y))
}

/// The hit mesh's own principal-axis frame, transformed into world space.
/// `centroid` is a point (needs the transform's translation), `axis0`/`axis1`
/// are directions (only the linear part, no translation). Shared by Cut View
/// and Bridge Split, both of which drive the same [`crate::cut::cut_manipulator`]
/// follow orientation from it. `None` propagates through to the disc's
/// local-normal fallback (see [`crate::cut::cut_geometry::follow_plane_normal`]).
pub(super) fn world_arch_frame(entry: &occluview_core::SceneMesh) -> Option<ArchFrame> {
    let local = entry.mesh.principal_frame_cached()?;
    let centroid = entry.transform.transform_point3(local.centroid);
    let axis0 = entry
        .transform
        .transform_vector3(local.axes[0])
        .normalize_or_zero();
    let axis1 = entry
        .transform
        .transform_vector3(local.axes[1])
        .normalize_or_zero();
    if axis0.length_squared() <= f32::EPSILON || axis1.length_squared() <= f32::EPSILON {
        return None;
    }
    Some(ArchFrame {
        centroid,
        axis0,
        axis1,
    })
}

/// Who owns the pointer this frame: the bare 3D viewport, or egui chrome
/// sitting over it.
pub(super) struct ViewportPointer {
    /// The hover position, if the pointer is on screen at all.
    pub(super) pointer: Option<egui::Pos2>,
    /// Over the docked Section panel, which owns its own pointer: the wheel
    /// there sizes the disc or zooms the panel rather than reaching the scene.
    pub(super) over_section_panel: bool,
    /// Over bare scene: the only case a disc may be planted, sized or dragged.
    pub(super) over_viewport: bool,
}

impl SceneContext<'_> {
    /// Resolve pointer ownership shared by both disc tools.
    pub(super) fn viewport_pointer(
        &self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        scene: &Scene,
        slice_visible: bool,
    ) -> ViewportPointer {
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        self.viewport_pointer_at(ctx, viewport_rect, scene, slice_visible, pointer)
    }

    /// Resolve ownership at the event position, even when later events in the
    /// same input batch have already moved or released the pointer elsewhere.
    pub(super) fn viewport_press_owned(
        &self,
        ctx: &egui::Context,
        response: &egui::Response,
        point: egui::Pos2,
    ) -> bool {
        let Some(scene) = self.document.scene.as_ref() else {
            return false;
        };
        if ctx
            .layer_id_at(point)
            .is_some_and(|layer| layer != response.layer_id)
        {
            return false;
        }
        self.viewport_pointer_at(
            ctx,
            response.rect,
            scene,
            self.active_section_panel_rect(response.rect).is_some(),
            Some(point),
        )
        .over_viewport
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Event ownership needs the explicit viewport, scene, panel state and event position."
    )]
    fn viewport_pointer_at(
        &self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        _scene: &Scene,
        slice_visible: bool,
        pointer: Option<egui::Pos2>,
    ) -> ViewportPointer {
        let over_rect = pointer.is_some_and(|point| viewport_rect.contains(point));
        // The layers panel is a same-layer (Background) scope, so it needs an
        // explicit rect test; floating areas are caught by their non-Background
        // layer order.
        let layers_rect = self.layers_panel_rect(ctx, viewport_rect);
        let over_section_panel = slice_visible
            && pointer.is_some_and(|point| {
                crate::cut::cut_ruler::section_panel_contains(viewport_rect, point)
            });
        let gizmo_hidden = self.axis_gizmo_is_hidden();
        let gizmo_avoid = if gizmo_hidden {
            None
        } else {
            self.active_section_panel_rect(viewport_rect)
        };
        let over_gizmo = !gizmo_hidden
            && pointer.is_some_and(|point| {
                crate::viewer::axis_gizmo::axis_gizmo_footprint_for(viewport_rect, gizmo_avoid)
                    .contains(point)
            });
        let over_egui = pointer.is_some_and(|point| {
            layers_rect.contains(point)
                // The contact bar draws on the Background layer, so the layer
                // test above cannot see it: without this the cut disc takes the
                // wheel through the bar and the slider never moves.
                || (self.tools.contacts.is_open()
                    && self.scene_contact_bar_rect(ctx, viewport_rect)
                    .contains(point))
                || super::contact::bar::occupied_contact_bar_rect(ctx, self.scene_key)
                    .is_some_and(|rect| rect.contains(point))
                || super::contact::bar::contact_details_rect(ctx, self.scene_key)
                    .is_some_and(|rect| rect.contains(point))
                || ctx
                    .layer_id_at(point)
                    .is_some_and(|layer| layer.order != egui::Order::Background)
        }) || over_section_panel
            || over_gizmo;
        ViewportPointer {
            pointer,
            over_section_panel,
            over_viewport: over_rect && !over_egui,
        }
    }
}

impl SceneContext<'_> {
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_cut_tool_overlay(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) -> bool {
        // Invariant: a probe-linked cut is owned by the measure tool. If the
        // measure tool is gone (e.g. its toolbar toggle turned it off), the
        // passive section has no owner left to drive or close it, so it closes
        // with its tool — never orphaned.
        if self.tools.cut_view.is_probe_linked() && !self.tools.measure.is_active() {
            self.tools.cut_view.disable();
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
            return false;
        }
        let can_cut = self
            .document
            .scene
            .as_ref()
            .is_some_and(|scene| CutTool::can_render_bbox(scene.bbox()));
        if self.tools.cut_view.is_active() && !can_cut {
            self.tools.cut_view.disable();
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
            return false;
        }
        if !self.tools.cut_view.is_active() {
            return false;
        }
        let Some(camera) = self.render.camera else {
            return false;
        };
        let Some(scene) = self.document.scene.clone() else {
            return false;
        };

        let (update, hover_pos, panel_zoom_notches) = if self.input_allowed
            && !self.ui.modal_dialog_open()
        {
            let (frame, wheel) = self.build_cut_frame_input(ctx, &camera, &scene, viewport_rect);
            (
                self.tools.cut_view.update(&frame, frame.eye),
                frame.pointer,
                wheel,
            )
        } else {
            (crate::cut::cut_manipulator::CutUpdate::default(), None, 0.0)
        };
        let orientation_changed = self
            .tools
            .cut_view
            .sync_main_view(SectionMainView::from_camera(camera));
        if update.pose_changed
            || update.planted
            || update.unplanted
            || update.exited
            || orientation_changed
        {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
        // Plain wheel inside the Section panel: zoom the slice to the cursor.
        if panel_zoom_notches != 0.0
            && self.tools.cut_view.zoom_slice_at_cursor(
                viewport_rect,
                hover_pos,
                panel_zoom_notches,
            )
        {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
        match update.cursor {
            CutCursor::Grab => ctx.set_cursor_icon(egui::CursorIcon::Grab),
            CutCursor::Grabbing => ctx.set_cursor_icon(egui::CursorIcon::Grabbing),
            CutCursor::Default => {}
        }
        if update.exited {
            return false;
        }

        // Section contour (camera-independent, cached).
        let section = self.cut_section(&scene);
        // One per-layer contour tint, shared by the 3D overlay and the panel's
        // Lines mode so their colors match exactly.
        let color_for = contour_tint(&scene);
        {
            let painter = ui.painter();
            if let Some(section) = section.as_deref() {
                cut_overlay::paint_section_contour(
                    painter,
                    &camera,
                    viewport_rect,
                    section,
                    &color_for,
                );
            }
            if let Some(pose) = self.tools.cut_view.pose() {
                cut_overlay::paint_disc(
                    painter,
                    &camera,
                    viewport_rect,
                    pose,
                    self.tools.cut_view.is_planted(),
                );
            }
        }

        // Responsiveness: render this frame's slice before painting the panel, so
        // a plant/drag/orbit/zoom shows its fresh section with no frame of lag.
        // `maybe_render_cut_view` consumes the dirty flag (`take_needs_render`),
        // so this stays one slice render per frame — the top-of-loop pass then
        // no-ops during an active cut. In Lines mode it no-ops (no GPU slice).
        self.maybe_render_cut_view(ctx);
        let panel = self.tools.cut_view.show_section_panel(
            ui,
            viewport_rect,
            section.as_deref(),
            &color_for,
            &self.ui.locale,
        );
        let panel_consumed = panel.consumed_pointer;
        self.apply_cut_section_outcome(panel, ctx);
        update.consumed_pointer || panel_consumed
    }

    fn apply_cut_section_outcome(
        &mut self,
        panel: crate::cut::cut_tool::CutToolUiOutcome,
        ctx: &egui::Context,
    ) {
        let measure_owned = self.tools.cut_view.is_probe_linked();
        if matches!(
            panel.command,
            crate::cut::cut_ruler::SectionPanelCommand::Close
        ) {
            if measure_owned {
                self.disarm_measure_and_probe_cut();
            } else {
                self.tools.cut_view.disable();
                self.render.invalidation.overlay_tools_changed();
            }
            ctx.request_repaint();
            return;
        }
        if panel.thickness_changed && measure_owned {
            if let Some(probe) = panel.thickness_probe {
                self.tools.measure.set_probe(ThicknessProbe {
                    entry: probe.entry,
                    reading: ThicknessReading::Wall {
                        exit: probe.exit,
                        thickness_mm: probe.thickness_mm,
                    },
                });
                self.scene_ui.status_message = Some(
                    self.ui.locale.tr_with(
                        crate::i18n::message_id!("measure-thickness"),
                        &[(
                            "len",
                            measure_tool::format_length(
                                f64::from(probe.thickness_mm),
                                self.persistence.settings.unit_display,
                                self.ui.locale.number_format(),
                            )
                            .as_str(),
                        )],
                    ),
                );
            } else {
                self.tools.measure.clear_probe();
            }
            ctx.request_repaint();
        }
        if panel.viewport_needs_render {
            self.render.invalidation.overlay_tools_changed();
            ctx.request_repaint();
        }
    }

    fn cut_section(&mut self, scene: &Scene) -> Option<Arc<SceneSection>> {
        self.section_for_plane(scene, self.tools.cut_view.section_plane())
    }

    /// Compute one cached world-space section from any active viewport tool.
    pub(super) fn section_for_plane(
        &mut self,
        scene: &Scene,
        plane: Option<occluview_mesh_edit::scene::SectionPlane>,
    ) -> Option<Arc<SceneSection>> {
        plane.map(|plane| self.render.section_cache.get_or_compute(scene, plane))
    }

    /// Sample this frame's pointer/keyboard/camera facts into a cut frame input.
    /// Reads and consumes the scoped wheel and Esc/F keys. Returns the frame plus
    /// the plain-wheel notches to spend on the in-panel zoom-to-cursor (`0` when
    /// the wheel was not a plain scroll inside the Section panel).
    fn build_cut_frame_input(
        &self,
        ctx: &egui::Context,
        camera: &occluview_core::Camera,
        scene: &Scene,
        viewport_rect: egui::Rect,
    ) -> (CutFrameInput, f32) {
        let raw_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let ctrl = ctx.input(|i| i.modifiers.command);
        // The disc only owns the *bare* viewport: never plant, slice or size
        // it through an egui surface sitting over the scene.
        //
        // The docked Section panel owns its own pointer (measuring plus the
        // disc-radius wheel) while it is on screen, and the axis gizmo needs an
        // explicit footprint test because it paints on the Background layer --
        // otherwise a follow-disc click on an axis marker both snaps the camera
        // and plants a disc.
        let ViewportPointer {
            pointer,
            over_section_panel,
            over_viewport,
        } = self.viewport_pointer(
            ctx,
            viewport_rect,
            scene,
            self.tools.cut_view.slice_visible(),
        );

        // A probe-linked cut is passive: the measure tool owns the main-viewport
        // pointer and the Esc/F keys, so the disc is not draggable, does not
        // re-plant, and never consumes Escape here (Esc goes to the measure tool,
        // which closes both). This is what lets the two tools coexist.
        let probe_linked = self.tools.cut_view.is_probe_linked();

        // An armed lasso outline owns primary clicks (the same placement
        // convention dental CAD software uses); the follow-mode plant yields
        // to it. A *planted* disc still owns its handle presses, so only the
        // follow-mode plant is gated here.
        let lasso_owns_lmb =
            self.document.edit_mode.lasso_armed() && self.document.edit_mode.has_active_session();
        let plant_suppressed = lasso_owns_lmb && !self.tools.cut_view.is_planted();
        let primary_pressed = raw_pressed && !plant_suppressed && !probe_linked;

        // Never steal Escape from an open dialog: the cut ladder only consumes
        // it when the operator is actually looking at the viewport.
        let escape = !probe_linked
            && !self.ui.modal_dialog_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let flip = !probe_linked
            && self.tools.cut_view.is_planted()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F));

        // Wheel scoping: the wheel acts only inside the Section
        // panel; over the bare viewport it stays camera zoom. Inside the panel,
        // Ctrl+wheel resizes the disc (manipulator radius) and a plain wheel
        // zooms the slice to the cursor. Drain the scroll in both cases so it
        // never leaks to the camera in `handle_viewport_input`.
        let (wheel_notches, panel_zoom_notches) =
            super::disc_frame::section_panel_wheel(ctx, over_section_panel, ctrl);

        let super::disc_frame::DiscViewGeometry {
            eye,
            view_dir,
            camera_up,
            camera_right,
            ray_origin,
        } = super::disc_frame::disc_view_geometry(camera, viewport_rect, pointer);

        let surface_hit = if self.tools.cut_view.is_planted() || !over_viewport {
            None
        } else {
            pointer.and_then(|p| surface_sample(camera, viewport_rect, p, scene))
        };

        let (disc_center_screen, disc_radius_screen) = super::disc_frame::disc_screen_placement(
            camera,
            viewport_rect,
            self.tools.cut_view.pose(),
        );

        let frame = CutFrameInput {
            pointer,
            over_viewport,
            primary_pressed,
            primary_down,
            ctrl,
            escape,
            flip,
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
}

#[cfg(test)]
mod viewport_ownership_tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
    use crate::app::OccluViewApp;
    use crate::layers_overlay;

    #[test]
    fn raw_press_ownership_uses_event_position_instead_of_final_hover() {
        let mut app = test_app("viewport-press-owner");
        app.workspace.scenes[0].document.scene = Some(Arc::new(named_scene("scan", 0.0)));
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![egui::Event::PointerMoved(screen.center())],
                ..Default::default()
            },
            |ui| {
                let response = ui.allocate_response(ui.available_size(), egui::Sense::drag());
                let layers = layers_overlay::layer_overlay_rect(response.rect, 1);
                assert!(!app
                    .active_context()
                    .expect("live test scene")
                    .viewport_press_owned(&ctx, &response, layers.center()));
                assert!(app
                    .active_context()
                    .expect("live test scene")
                    .viewport_press_owned(&ctx, &response, response.rect.center()));
                assert!(!app
                    .active_context()
                    .expect("live test scene")
                    .viewport_press_owned(
                        &ctx,
                        &response,
                        response.rect.right_bottom() + egui::vec2(10.0, 10.0)
                    ));
            },
        )
        .drop_without_applying_deltas();
    }

    #[test]
    fn contact_controls_own_the_frame_that_closes_them_and_release_the_next() {
        let mut app = test_app("viewport-contact-owner");
        let mut scene = named_scene("subject", 0.0);
        let subject = scene.meshes()[0].id();
        let antagonist = push_named_layer(&mut scene, "antagonist", 2.0);
        app.workspace.scenes[0].document.scene = Some(Arc::new(scene));
        app.workspace.scenes[0]
            .tools
            .contacts
            .open(crate::contact::state::ContactPair {
                subject,
                antagonist,
            });
        app.workspace.scenes[0].tools.contacts.toggle_details();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0));
        let mut details_point = None;
        for closing_frame in [true, false] {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    let response = ui.allocate_response(ui.available_size(), egui::Sense::drag());
                    let _ = app
                        .active_context()
                        .expect("live test scene")
                        .show_contact_bar(ui, response.rect, &ctx);
                    if closing_frame {
                        let bar = super::super::contact::bar::occupied_contact_bar_rect(
                            &ctx,
                            super::super::workspace::id::SceneKey::INITIAL,
                        )
                        .expect("bar drawn");
                        let details = super::super::contact::bar::contact_details_rect(
                            &ctx,
                            super::super::workspace::id::SceneKey::INITIAL,
                        )
                        .expect("details drawn");
                        assert!(details.height() > 0.0);
                        details_point = Some(details.center());
                        app.workspace.scenes[0].tools.contacts.close();
                        assert!(!app
                            .active_context()
                            .expect("live test scene")
                            .viewport_press_owned(&ctx, &response, bar.center()));
                        assert!(!app
                            .active_context()
                            .expect("live test scene")
                            .viewport_press_owned(&ctx, &response, details.center()));
                    } else {
                        assert!(super::super::contact::bar::contact_details_rect(
                            &ctx,
                            super::super::workspace::id::SceneKey::INITIAL
                        )
                        .is_none());
                        assert!(app
                            .active_context()
                            .expect("live test scene")
                            .viewport_press_owned(
                                &ctx,
                                &response,
                                details_point.expect("previous panel position")
                            ));
                    }
                },
            )
            .drop_without_applying_deltas();
        }
    }

    fn flat_sculpt_fixture() -> (OccluViewApp, Arc<occluview_core::Mesh>) {
        use crate::sculpt::sculpt_kernel::BrushSession;
        use crate::sculpt::sculpt_tool::{SculptSession, SculptToolKind};
        use crate::sculpt::sculpt_worker::SculptWorker;
        use glam::{Affine3A, Quat};
        use occluview_core::{Mesh, SceneMesh, Vertex};
        use occluview_mesh_edit::mesh_edit_buffers_from_mesh;
        use occluview_render::PreparedSceneTopology;
        use std::sync::RwLock;

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for row in 0..17_u32 {
            for column in 0..17_u32 {
                let x = f32::from(u16::try_from(column).expect("small column")) * 0.25 - 2.0;
                let y = f32::from(u16::try_from(row).expect("small row")) * 0.25 - 2.0;
                vertices.push(Vertex::at(Vec3::new(x, y, 0.0)));
                if row < 16 && column < 16 {
                    let corner = row * 17 + column;
                    indices.extend_from_slice(&[
                        corner,
                        corner + 1,
                        corner + 18,
                        corner,
                        corner + 18,
                        corner + 17,
                    ]);
                }
            }
        }
        let mesh = Mesh::new(Some("sheet".into()), vertices, indices).expect("grid mesh");
        mesh.warm_bvh();
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        let scene = Arc::new(scene);
        let entry = &scene.meshes()[0];
        let layer_id = entry.id();
        let base = Arc::clone(&entry.mesh);
        let session = BrushSession::prepare(&mesh_edit_buffers_from_mesh(&base)).expect("prepare");
        let mut app = test_app("sculpt-overlay-gap-replay");
        assert!(app.workspace.scenes[0]
            .document
            .edit_mode
            .begin_face_selection(entry, &scene));
        app.workspace.scenes[0].document.scene = Some(Arc::clone(&scene));
        app.workspace.scenes[0].tools.sculpt.armed = Some(SculptToolKind::AddRemove);
        app.workspace.scenes[0].render.camera = Some(occluview_core::Camera {
            target: Vec3::ZERO,
            distance: 100.0,
            orientation: Some(Quat::IDENTITY),
            orthographic_height: 8.0,
            near: 0.1,
            far: 200.0,
            ..Default::default()
        });
        app.workspace.scenes[0].tools.sculpt.worker = Some(SculptWorker::spawn(SculptSession {
            layer_id,
            topology_id: base.topology_id(),
            session,
            base_mesh: Arc::clone(&base),
            shadow: Arc::new(RwLock::new(base.vertices().to_vec())),
            topology: PreparedSceneTopology::from_mesh(&base),
            world_to_local: Affine3A::IDENTITY,
            local_per_world: 1.0,
            dirty_stroke: false,
            topology_dirty_stroke: false,
            stroke_start_mesh: None,
        }));
        (app, base)
    }

    fn drain_sculpt_worker(app: &mut OccluViewApp, ctx: &egui::Context) {
        use crate::sculpt::sculpt_worker::SculptWorker;
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.active_context()
                .expect("live test scene")
                .poll_sculpt_worker(ctx);
            if app.workspace.scenes[0]
                .tools
                .sculpt
                .worker
                .as_ref()
                .is_some_and(SculptWorker::is_quiescent)
            {
                app.active_context()
                    .expect("live test scene")
                    .poll_sculpt_worker(ctx);
                break;
            }
            assert!(Instant::now() < deadline, "worker finishes the ray path");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn show_occluding_panel(ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("occluding-test-panel"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(780.0, 480.0))
            .show(ctx, |panel| {
                panel.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
            });
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "Keep the complete gesture and its state assertions in one regression scenario."
    )]
    fn sculpt_reentry_after_an_overlay_leaves_the_hidden_surface_untouched() {
        use crate::sculpt::sculpt_tool::SculptTip;

        let (mut app, base) = flat_sculpt_fixture();
        let ctx = egui::Context::default();
        crate::mesh_editor::mesh_editor_overlay::set_sculpt_radius_mm(
            &ctx,
            super::super::workspace::id::SceneKey::INITIAL,
            SculptTip::Ball,
            0.5,
        );
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let left = egui::pos2(700.0, 500.0);
        let hidden = egui::pos2(800.0, 500.0);
        let right = egui::pos2(900.0, 500.0);
        let button = |point, pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // Areas need their initial sizing frame before they are visible and
        // participate in hit testing. Replay input against the drawn panel.
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| {
                ui.allocate_rect(viewport, egui::Sense::click_and_drag());
                show_occluding_panel(&ctx);
            },
        )
        .drop_without_applying_deltas();
        for events in [
            vec![egui::Event::PointerMoved(left), button(left, true)],
            vec![
                egui::Event::PointerMoved(hidden),
                egui::Event::PointerMoved(right),
            ],
            vec![button(right, false)],
        ] {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = ui.allocate_rect(viewport, egui::Sense::click_and_drag());
                    show_occluding_panel(&ctx);
                    assert!(!app
                        .active_context()
                        .expect("live test scene")
                        .viewport_press_owned(&ctx, &response, hidden));
                    let _ = app
                        .active_context()
                        .expect("live test scene")
                        .handle_sculpt_drag(&ctx, &response, false);
                },
            )
            .drop_without_applying_deltas();
            drain_sculpt_worker(&mut app, &ctx);
        }
        assert!(app.workspace.scenes[0].tools.sculpt.stroke.is_none());
        let committed = Arc::clone(
            &app.workspace.scenes[0]
                .document
                .scene
                .as_ref()
                .expect("scene")
                .meshes()[0]
                .mesh,
        );
        assert!(committed
            .vertices()
            .iter()
            .any(|vertex| vertex.position[2] > 1.0e-4));
        let middle: Vec<_> = committed
            .vertices()
            .iter()
            .filter(|vertex| vertex.position[0].abs() < 0.1 && vertex.position[1].abs() < 0.1)
            .collect();
        assert!(!middle.is_empty(), "the hidden centre remains in the mesh");
        assert!(
            middle
                .iter()
                .all(|vertex| vertex.position[2].abs() < 1.0e-6),
            "re-entry starts a new path instead of carving across the panel"
        );
        app.active_context()
            .expect("live test scene")
            .apply_history_navigation_now(false, &ctx);
        let undone = &app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("undo scene")
            .meshes()[0]
            .mesh;
        assert_eq!(undone.vertices(), base.vertices());
        assert_eq!(undone.indices(), base.indices());
        app.active_context()
            .expect("live test scene")
            .apply_history_navigation_now(true, &ctx);
        let redone = &app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .expect("redo scene")
            .meshes()[0]
            .mesh;
        assert_eq!(redone.vertices(), committed.vertices());
        assert_eq!(redone.indices(), committed.indices());
    }
}

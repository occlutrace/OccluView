//! Cut View ownership, pointer routing, and viewport overlay handling.
//!
//! The thickness tool can plant a Cut View, so both workflows share the
//! viewport ownership and section cache defined here.

use super::{egui, layers_overlay, pick_scene_hit, CutTool, OccluViewApp, Scene};
use crate::cut_manipulator::{ArchFrame, CutCursor, CutFrameInput, SurfaceSample};
use crate::cut_overlay;
use crate::measure_tool::{self, ThicknessProbe, ThicknessReading};
use crate::section_view::SectionMainView;
use glam::{Vec3, Vec3A};
use occluview_core::scene::SceneSection;
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
/// and Bridge Split, both of which drive the same [`crate::cut_manipulator`]
/// follow orientation from it. `None` propagates through to the disc's
/// local-normal fallback (see [`crate::cut_geometry::follow_plane_normal`]).
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

impl OccluViewApp {
    /// Resolve pointer ownership shared by both disc tools.
    pub(super) fn viewport_pointer(
        &self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        scene: &Scene,
        slice_visible: bool,
    ) -> ViewportPointer {
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let over_rect = pointer.is_some_and(|point| viewport_rect.contains(point));
        // The layers panel is a same-layer (Background) scope, so it needs an
        // explicit rect test; floating areas are caught by their non-Background
        // layer order.
        let layers_rect = layers_overlay::layer_overlay_rect(viewport_rect, scene.meshes().len());
        let over_section_panel = slice_visible
            && pointer.is_some_and(|point| {
                crate::cut_ruler::section_panel_contains(viewport_rect, point)
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
                    && crate::app::app_contact_bar::contact_bar_rect(
                        viewport_rect,
                        scene.meshes().len(),
                    )
                    .contains(point))
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

impl OccluViewApp {
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

        let (frame, panel_zoom_notches) =
            self.build_cut_frame_input(ctx, &camera, &scene, viewport_rect);
        let eye = frame.eye;
        let hover_pos = frame.pointer;
        let update = self.tools.cut_view.update(&frame, eye);
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
        panel: crate::cut_tool::CutToolUiOutcome,
        ctx: &egui::Context,
    ) {
        let measure_owned = self.tools.cut_view.is_probe_linked();
        if matches!(panel.command, crate::cut_ruler::SectionPanelCommand::Close) {
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
                self.ui.status_message = Some(
                    self.ui.locale.tr_with(
                        "measure-thickness",
                        &[(
                            "len",
                            measure_tool::format_length(
                                f64::from(probe.thickness_mm),
                                self.persistence.settings.unit_display,
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
        plane: Option<occluview_core::scene::SectionPlane>,
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

//! Viewport interaction for the cut plane and the measuring tools.
//!
//! The pointer means different things depending on which tool owns it, and
//! this module is where that is decided: placing and dragging the clip plane,
//! the follow disc that rides the surface under the cursor, the caliper and
//! thickness probes, and the section view's own picking.
//!
//! Everything here works in world space obtained by unprojecting the cursor
//! through the shared viewport helpers, so a hit means the same thing to the
//! overlay that draws it and to the geometry that answers it. Drawing lives in
//! `cut_overlay` and `measure_overlay`; the numbers live in `measure_tool` and
//! `probe_section`.

use super::{egui, layers_overlay, pick_scene_hit, CutTool, OccluViewApp, Scene};
use crate::cut_manipulator::{ArchFrame, CutCursor, CutFrameInput, SurfaceSample};
use crate::cut_overlay;
use crate::measure_overlay;
use crate::measure_tool::{self, MeasureMode, ThicknessProbe, ThicknessReading};
use crate::probe_section;
use crate::section_view::SectionMainView;
use glam::{Vec3, Vec3A};
use occluview_core::scene::SceneSection;
use occluview_core::ScenePickHit;
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
/// `centroid` is a POINT (needs the transform's translation), `axis0`/`axis1`
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
        if self.cut_view.is_probe_linked() && !self.measure.is_active() {
            self.cut_view.disable();
            self.needs_render = true;
            ctx.request_repaint();
            return false;
        }
        let can_cut = self
            .scene
            .as_ref()
            .is_some_and(|scene| CutTool::can_render_bbox(scene.bbox()));
        if self.cut_view.is_active() && !can_cut {
            self.cut_view.disable();
            self.needs_render = true;
            ctx.request_repaint();
            return false;
        }
        if !self.cut_view.is_active() {
            return false;
        }
        let Some(camera) = self.camera else {
            return false;
        };
        let Some(scene) = self.scene.clone() else {
            return false;
        };

        let (frame, panel_zoom_notches) =
            self.build_cut_frame_input(ctx, &camera, &scene, viewport_rect);
        let eye = frame.eye;
        let hover_pos = frame.pointer;
        let update = self.cut_view.update(&frame, eye);
        let orientation_changed = self
            .cut_view
            .sync_main_view(SectionMainView::from_camera(camera));
        if update.pose_changed
            || update.planted
            || update.unplanted
            || update.exited
            || orientation_changed
        {
            self.needs_render = true;
            ctx.request_repaint();
        }
        // Plain wheel inside the Section panel: zoom the slice to the cursor.
        if panel_zoom_notches != 0.0
            && self
                .cut_view
                .zoom_slice_at_cursor(viewport_rect, hover_pos, panel_zoom_notches)
        {
            self.needs_render = true;
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
            if let Some(pose) = self.cut_view.pose() {
                cut_overlay::paint_disc(
                    painter,
                    &camera,
                    viewport_rect,
                    pose,
                    self.cut_view.is_planted(),
                );
            }
        }

        // Responsiveness: render THIS frame's slice before painting the panel, so
        // a plant/drag/orbit/zoom shows its fresh section with no frame of lag.
        // `maybe_render_cut_view` consumes the dirty flag (`take_needs_render`),
        // so this stays one slice render per frame — the top-of-loop pass then
        // no-ops during an active cut. In Lines mode it no-ops (no GPU slice).
        self.maybe_render_cut_view(ctx);
        let panel =
            self.cut_view
                .show_section_panel(ui, viewport_rect, section.as_deref(), &color_for);
        let panel_consumed = panel.consumed_pointer;
        self.apply_cut_section_outcome(panel, ctx);
        update.consumed_pointer || panel_consumed
    }

    fn apply_cut_section_outcome(
        &mut self,
        panel: crate::cut_tool::CutToolUiOutcome,
        ctx: &egui::Context,
    ) {
        let measure_owned = self.cut_view.is_probe_linked();
        if matches!(panel.command, crate::cut_ruler::SectionPanelCommand::Close) {
            if measure_owned {
                self.disarm_measure_and_probe_cut();
            } else {
                self.cut_view.disable();
                self.needs_render = true;
            }
            ctx.request_repaint();
            return;
        }
        if panel.thickness_changed && measure_owned {
            if let Some(probe) = panel.thickness_probe {
                self.measure.set_probe(ThicknessProbe {
                    entry: probe.entry,
                    reading: ThicknessReading::Wall {
                        exit: probe.exit,
                        thickness_mm: probe.thickness_mm,
                    },
                });
                self.status_message = Some(format!(
                    "Wall thickness: {}",
                    measure_tool::format_length(
                        f64::from(probe.thickness_mm),
                        self.settings.unit_display
                    )
                ));
            } else {
                self.measure.clear_probe();
            }
            ctx.request_repaint();
        }
        if panel.viewport_needs_render {
            self.needs_render = true;
            ctx.request_repaint();
        }
    }

    fn cut_section(&mut self, scene: &Scene) -> Option<Arc<SceneSection>> {
        self.section_for_plane(scene, self.cut_view.section_plane())
    }

    /// Compute one cached world-space section from any active viewport tool.
    pub(super) fn section_for_plane(
        &mut self,
        scene: &Scene,
        plane: Option<occluview_core::scene::SectionPlane>,
    ) -> Option<Arc<SceneSection>> {
        plane.map(|plane| self.section_cache.get_or_compute(scene, plane))
    }

    /// Advance the armed measurement tool one frame: keep the tool-exclusivity
    /// invariants, route Esc and stationary clicks, and paint the overlays.
    /// Returns whether the pointer was consumed (mirrors the cut overlay's
    /// contract in `show_central_panel`); drags always fall through so the
    /// camera keeps orbit/pan/zoom while a measure tool is armed.
    pub(super) fn show_measure_tool_overlay(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        suppress_click: bool,
        ctx: &egui::Context,
    ) -> bool {
        if !self.measure.is_active() {
            return false;
        }
        // Invariants: an edit session owns LMB (marquee/lasso), an INTERACTIVE
        // cut owns the viewport, and a closed scene has nothing to measure. A
        // PROBE-LINKED cut is the exception: it was opened by this very tool and
        // is passive, so the marker and the section coexist. The tool stands down
        // instead of fighting the others.
        if self.edit_mode.has_active_session()
            || (self.cut_view.is_active() && !self.cut_view.is_probe_linked())
            || self.scene.is_none()
            || self.camera.is_none()
        {
            self.measure.disarm();
            ctx.request_repaint();
            return false;
        }
        // Esc exits the tool (and drops its overlays, incl. the probe-linked cut
        // view it opened) — but never steal Escape from an open dialog (same rule
        // as the cut ladder).
        if !self.modal_dialog_open()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.disarm_measure_and_probe_cut();
            ctx.request_repaint();
            return false;
        }
        // Align Scans owns the primary click while it is armed. Without this
        // one click would place an align point AND a ruler anchor.
        if self.align_active() {
            self.measure.disarm();
            return false;
        }
        let viewport_rect = response.rect;
        let consumed = self.handle_measure_pointer(response, suppress_click, ctx);
        let hover = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| self.pointer_on_bare_viewport(ctx, viewport_rect, *pos));
        if let Some(pointer) = hover {
            let over_anchor = self.measure.mode() == Some(MeasureMode::Ruler)
                && self.camera.is_some_and(|camera| {
                    measure_overlay::ruler_anchor_at(&camera, viewport_rect, &self.measure, pointer)
                        .is_some()
                });
            ctx.set_cursor_icon(if over_anchor {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        if let Some(camera) = self.camera {
            measure_overlay::paint_measurements(
                ui.painter(),
                &camera,
                viewport_rect,
                &self.measure,
                self.settings.unit_display,
                hover,
            );
        }
        consumed
    }

    /// Disarm the measure tool and, if the cut view was opened by its probe,
    /// close that too — one gesture (Esc / the strip Close) dismisses everything
    /// the thickness probe put on screen.
    fn disarm_measure_and_probe_cut(&mut self) {
        self.measure.disarm();
        if self.cut_view.is_probe_linked() {
            self.cut_view.disable();
            self.needs_render = true;
        }
    }

    /// Whether `pos` is over the bare 3D viewport: inside the rect and not over
    /// the measure strip, the layers panel, or any floating egui surface (same
    /// chrome test the cut tool uses).
    pub(super) fn pointer_on_bare_viewport(
        &self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
        pos: egui::Pos2,
    ) -> bool {
        if !viewport_rect.contains(pos) {
            return false;
        }
        // A probe-linked cut view coexists with the measure tool: its docked
        // Section panel owns its own pointer, so treat it as chrome, never as bare
        // viewport (no crosshair/re-probe bleeding into the panel).
        if self.cut_view.is_active() && crate::cut_ruler::section_panel_contains(viewport_rect, pos)
        {
            return false;
        }
        let layer_count = self.scene.as_ref().map_or(0, |scene| scene.meshes().len());
        if layers_overlay::layer_overlay_rect(viewport_rect, layer_count).contains(pos) {
            return false;
        }
        ctx.layer_id_at(pos)
            .is_none_or(|layer| layer.order == egui::Order::Background)
    }

    /// Route the frame's stationary clicks: LMB on the model places a ruler
    /// anchor or probes thickness (off-mesh clicks do nothing — no floating
    /// air-points); RMB clears every measurement, and a stationary RMB with
    /// nothing left to clear falls through to the shared scene menu, so
    /// saving stays reachable while the tool is up. Click detection is egui's
    /// press+release-without-drag, so a drag still orbits.
    fn handle_measure_pointer(
        &mut self,
        response: &egui::Response,
        suppress_click: bool,
        ctx: &egui::Context,
    ) -> bool {
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|input| input.pointer.hover_pos()))
        else {
            return false;
        };
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if self.measure.dragged_ruler_anchor().is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if primary_down && response.rect.contains(pointer) {
                if let Some((camera, scene)) = self.camera.zip(self.scene.clone()) {
                    if let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) {
                        if let Some(distance_mm) = self.measure.update_ruler_drag(hit.point) {
                            self.status_message = Some(format!(
                                "Distance: {}",
                                measure_tool::format_length(
                                    distance_mm,
                                    self.settings.unit_display
                                )
                            ));
                            ctx.request_repaint();
                        }
                    }
                }
            } else if !primary_down {
                self.measure.end_ruler_drag();
                ctx.request_repaint();
            }
            return true;
        }
        if !self.pointer_on_bare_viewport(ctx, response.rect, pointer) {
            return false;
        }
        if self.measure.mode() == Some(MeasureMode::Ruler)
            && ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        {
            if let Some(camera) = self.camera {
                if let Some(anchor) =
                    measure_overlay::ruler_anchor_at(&camera, response.rect, &self.measure, pointer)
                {
                    if self.measure.begin_ruler_drag(anchor) {
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        return true;
                    }
                }
            }
        }
        if response.secondary_clicked() {
            // RMB is also the orbit button. Only a truly stationary right-click
            // clears: `viewport_secondary_gesture_moved_since_press` is armed by
            // ANY pointer motion during the press (including sub-threshold
            // motion the platform may classify as a click), which is exactly
            // the "Thickness exits on rotation" guard. Note `press_origin()`
            // cannot be used here — egui wipes it on every release, so on the
            // click frame it is always None.
            if self.viewport_secondary_gesture_moved_since_press {
                return false;
            }
            let cleared_anything = self.measure.clear_measurements();
            if cleared_anything {
                self.status_message = Some("Measurements cleared".to_string());
            }
            // Clearing the measurement also closes the cut view it drove — the
            // section reflects the current probe or nothing at all.
            let probe_linked = self.cut_view.is_probe_linked();
            if probe_linked {
                self.cut_view.disable();
                self.needs_render = true;
            }
            ctx.request_repaint();
            // Nothing was cleared: the stationary RMB was not a tool gesture,
            // so let the shared layer/scene menu open instead of eating it.
            return cleared_anything || probe_linked;
        }
        if suppress_click || !response.clicked_by(egui::PointerButton::Primary) {
            return false;
        }
        let Some((camera, scene)) = self.camera.zip(self.scene.clone()) else {
            return false;
        };
        if let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) {
            self.apply_measure_click(&scene, hit);
            ctx.request_repaint();
        }
        // Even an off-mesh click belongs to the armed tool: nothing behind it
        // (face pick, camera retarget) may act on it.
        true
    }

    /// Apply one on-mesh measure click for the armed mode.
    fn apply_measure_click(&mut self, scene: &Scene, hit: ScenePickHit) {
        match self.measure.mode() {
            Some(MeasureMode::Ruler) => {
                if let Some(distance_mm) = self.measure.place_ruler_point(hit.point) {
                    self.status_message = Some(format!(
                        "Distance: {}",
                        measure_tool::format_length(distance_mm, self.settings.unit_display)
                    ));
                }
            }
            Some(MeasureMode::Thickness) => self.apply_thickness_probe(scene, hit),
            None => {}
        }
    }

    /// Probe the wall of the hit layer and report the reading honestly.
    fn apply_thickness_probe(&mut self, scene: &Scene, hit: ScenePickHit) {
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return;
        };
        if entry.id() != hit.layer_id {
            return;
        }
        match measure_tool::probe_wall_thickness(entry, hit.triangle_index, hit.point) {
            Some(probe) => {
                self.status_message = Some(match probe.reading {
                    ThicknessReading::Wall { thickness_mm, .. } => format!(
                        "Wall thickness: {}",
                        measure_tool::format_length(
                            f64::from(thickness_mm),
                            self.settings.unit_display
                        )
                    ),
                    ThicknessReading::Open => {
                        "Open surface: no opposite wall along the inward normal".to_string()
                    }
                });
                self.measure.set_probe(probe);
                // Feature D: the same click ALSO opens the Cut View at this
                // cross-section (Wall readings only), showing the same chord.
                self.drive_probe_cut_view(scene, &probe);
            }
            None => {
                self.status_message =
                    Some("Cannot probe here: degenerate surface geometry".to_string());
            }
        }
    }

    /// Open (or re-aim) the probe-linked Cut View from a thickness reading.
    ///
    /// A `Wall` reading with a buildable cross-section plane plants a world-fixed
    /// disc whose plane contains the entry->exit chord, so the wall reads edge-on
    /// in the Section panel with the same measurement. An `Open` reading (or a
    /// degenerate chord we cannot section) plants nothing and closes any cut view
    /// this probe flow had opened — honest: there is nothing to section.
    fn drive_probe_cut_view(&mut self, scene: &Scene, probe: &ThicknessProbe) {
        let planned = if let ThicknessReading::Wall { exit, thickness_mm } = probe.reading {
            let scale_hint = scene.bbox().half_diagonal();
            probe_section::disc_pose_through_chord(probe.entry, exit, scale_hint)
                .map(|pose| (pose, exit, thickness_mm))
        } else {
            None
        };
        match planned {
            Some((pose, exit, thickness_mm)) => {
                let eye = self
                    .camera
                    .map_or(pose.center + pose.plane_normal, occluview_core::Camera::eye);
                let keep_positive = crate::cut_geometry::camera_keep_side(&pose, eye);
                let seed = probe_section::SliceProbe {
                    entry: probe.entry,
                    exit,
                    thickness_mm,
                };
                self.cut_view.plant_from_probe(pose, keep_positive, seed);
                self.needs_render = true;
            }
            None => {
                if self.cut_view.is_probe_linked() {
                    self.cut_view.disable();
                    self.needs_render = true;
                }
            }
        }
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
        } = self.viewport_pointer(ctx, viewport_rect, scene, self.cut_view.slice_visible());

        // A probe-linked cut is PASSIVE: the measure tool owns the main-viewport
        // pointer and the Esc/F keys, so the disc is not draggable, does not
        // re-plant, and never consumes Escape here (Esc goes to the measure tool,
        // which closes both). This is what lets the two tools coexist.
        let probe_linked = self.cut_view.is_probe_linked();

        // An armed lasso outline owns primary clicks (the same placement
        // convention dental CAD software uses); the follow-mode plant yields
        // to it. A *planted* disc still owns its handle presses, so only the
        // follow-mode plant is gated here.
        let lasso_owns_lmb = self.edit_mode.lasso_armed() && self.edit_mode.has_active_session();
        let plant_suppressed = lasso_owns_lmb && !self.cut_view.is_planted();
        let primary_pressed = raw_pressed && !plant_suppressed && !probe_linked;

        // Never steal Escape from an open dialog: the cut ladder only consumes
        // it when the operator is actually looking at the viewport.
        let escape = !probe_linked
            && !self.modal_dialog_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let flip = !probe_linked
            && self.cut_view.is_planted()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F));

        // Wheel scoping: the wheel acts ONLY inside the Section
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

        let surface_hit = if self.cut_view.is_planted() || !over_viewport {
            None
        } else {
            pointer.and_then(|p| surface_sample(camera, viewport_rect, p, scene))
        };

        let (disc_center_screen, disc_radius_screen) =
            super::disc_frame::disc_screen_placement(camera, viewport_rect, self.cut_view.pose());

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

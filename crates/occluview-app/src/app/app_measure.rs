//! Ruler and thickness measurement input and overlay handling.
//!
//! A thickness probe may drive the passive Cut View owned by the measure tool.

use super::{egui, pick_scene_hit, Scene, SceneContext};
use crate::app_settings::RulerLineAngle;
use crate::cut::probe_section;
use crate::measure_overlay;
use crate::measure_tool::{self, MeasureMode, ThicknessProbe, ThicknessReading};
use occluview_core::ScenePickHit;

impl SceneContext<'_> {
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
        if !self.tools.measure.is_active() {
            return false;
        }
        // Invariants: an edit session owns LMB (marquee/lasso), an interactive
        // cut owns the viewport, and a closed scene has nothing to measure. A
        // probe-linked cut is the exception: it was opened by this very tool and
        // is passive, so the marker and the section coexist. The tool stands down
        // instead of fighting the others.
        if self.document.edit_mode.has_active_session()
            || (self.tools.cut_view.is_active() && !self.tools.cut_view.is_probe_linked())
            || self.document.scene.is_none()
            || self.render.camera.is_none()
        {
            self.tools.measure.disarm();
            ctx.request_repaint();
            return false;
        }
        // Esc exits the tool (and drops its overlays, incl. the probe-linked cut
        // view it opened) — but never steal Escape from an open dialog (same rule
        // as the cut ladder).
        if !self.ui.modal_dialog_open()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.disarm_measure_and_probe_cut();
            ctx.request_repaint();
            return false;
        }
        // Align Scans owns the primary click while it is armed. Without this
        // one click would place an align point and a ruler anchor.
        if self.align_active() {
            self.tools.measure.disarm();
            return false;
        }
        let viewport_rect = response.rect;
        self.show_ruler_options(ctx, viewport_rect);
        let consumed = self.handle_measure_pointer(response, suppress_click, ctx);
        let hover = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| self.pointer_on_bare_viewport(ctx, viewport_rect, *pos));
        if let Some(pointer) = hover {
            let over_anchor = self.tools.measure.mode() == Some(MeasureMode::Ruler)
                && self.render.camera.is_some_and(|camera| {
                    measure_overlay::ruler_anchor_at(
                        &camera,
                        viewport_rect,
                        &self.tools.measure,
                        pointer,
                    )
                    .is_some()
                });
            ctx.set_cursor_icon(if over_anchor {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        if let Some(camera) = self.render.camera {
            measure_overlay::paint_measurements(
                ui.painter(),
                &camera,
                viewport_rect,
                &self.tools.measure,
                self.persistence.settings.unit_display,
                hover,
                self.ruler_line_angle(ctx),
                &self.ui.locale,
            );
        }
        consumed
    }

    /// Disarm the measure tool and, if the cut view was opened by its probe,
    /// close that too — one gesture (Esc / the strip Close) dismisses everything
    /// the thickness probe put on screen.
    pub(super) fn disarm_measure_and_probe_cut(&mut self) {
        self.tools.measure.disarm();
        if self.tools.cut_view.is_probe_linked() {
            self.tools.cut_view.disable();
            self.render.invalidation.overlay_tools_changed();
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
        if self.tools.cut_view.is_active()
            && crate::cut::cut_ruler::section_panel_contains(viewport_rect, pos)
        {
            return false;
        }
        if self.layers_panel_rect(ctx, viewport_rect).contains(pos) {
            return false;
        }
        ctx.layer_id_at(pos)
            .is_none_or(|layer| layer.order == egui::Order::Background)
    }

    /// Route the frame's stationary clicks: LMB on the model places a ruler
    /// anchor or probes thickness (off-mesh clicks do nothing — no floating
    /// air-points, except an end on another ruler's line, which is in the air
    /// by design); RMB clears every measurement, and a stationary RMB with
    /// nothing left to clear falls through to the shared scene menu, so
    /// saving stays reachable while the tool is up. Click detection is egui's
    /// press+release-without-drag, so a drag still orbits.
    fn handle_measure_pointer(
        &mut self,
        response: &egui::Response,
        suppress_click: bool,
        ctx: &egui::Context,
    ) -> bool {
        if !self.input_allowed {
            return false;
        }
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|input| input.pointer.hover_pos()))
        else {
            return false;
        };
        if self.tools.measure.dragged_ruler_anchor().is_some() {
            self.continue_ruler_drag(response, pointer, ctx);
            return true;
        }
        if !self.pointer_on_bare_viewport(ctx, response.rect, pointer) {
            return false;
        }
        if self.tools.measure.mode() == Some(MeasureMode::Ruler)
            && ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        {
            if let Some(camera) = self.render.camera {
                if let Some(anchor) = measure_overlay::ruler_anchor_at(
                    &camera,
                    response.rect,
                    &self.tools.measure,
                    pointer,
                ) {
                    if self.tools.measure.begin_ruler_drag(anchor) {
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        return true;
                    }
                }
            }
        }
        if response.secondary_clicked() {
            // RMB is also the orbit button. Only a truly stationary right-click
            // clears: `viewport_secondary_gesture_moved_since_press` is armed by
            // any pointer motion during the press (including sub-threshold
            // motion the platform may classify as a click), which is the
            // "Thickness exits on rotation" guard. Note `press_origin()`
            // cannot be used here — egui wipes it on every release, so on the
            // click frame it is always None.
            if self.scene_ui.viewport_secondary_gesture_moved_since_press {
                return false;
            }
            let cleared_anything = self.tools.measure.clear_measurements();
            if cleared_anything {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("measure-cleared")),
                );
            }
            // Clearing the measurement also closes the cut view it drove — the
            // section reflects the current probe or nothing at all.
            let probe_linked = self.tools.cut_view.is_probe_linked();
            if probe_linked {
                self.tools.cut_view.disable();
                self.render.invalidation.overlay_tools_changed();
            }
            ctx.request_repaint();
            // Nothing was cleared: the stationary RMB was not a tool gesture,
            // so let the shared layer/scene menu open instead of eating it.
            return cleared_anything || probe_linked;
        }
        if suppress_click || !response.clicked_by(egui::PointerButton::Primary) {
            return false;
        }
        let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) else {
            return false;
        };
        // A click on a drawn ruler line, with an anchor pending, ends the
        // ruler on that line; the end is not on the surface, so it takes
        // precedence over whatever mesh lies under the line.
        if self.tools.measure.mode() == Some(MeasureMode::Ruler) {
            if let Some((base, placement)) = measure_overlay::line_target(
                &camera,
                response.rect,
                &self.tools.measure,
                pointer,
                self.ruler_line_angle(ctx),
            ) {
                if self.tools.measure.place_on_line(base, placement).is_some() {
                    self.report_ruler(self.tools.measure.ruler_count() - 1);
                    ctx.request_repaint();
                    return true;
                }
            }
        }
        if let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) {
            self.apply_measure_click(&scene, hit);
            ctx.request_repaint();
        }
        // Even an off-mesh click belongs to the armed tool: nothing behind it
        // (face pick, camera retarget) may act on it.
        true
    }

    /// One frame of a ruler-end drag: once the pointer has moved past the click
    /// tolerance, the end follows the surface under the pointer, or, for an
    /// end on another ruler's line, the place on that line under the pointer
    /// (the foot of the perpendicular while Shift is held). The drag ends on
    /// release.
    fn continue_ruler_drag(
        &mut self,
        response: &egui::Response,
        pointer: egui::Pos2,
        ctx: &egui::Context,
    ) {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if !primary_down {
            self.tools.measure.end_ruler_drag();
            ctx.request_repaint();
            return;
        }
        let click_tolerance = ctx.options(|options| options.input_options.max_click_dist);
        let moved_past_click = ctx.input(|input| {
            input
                .pointer
                .press_origin()
                .is_some_and(|origin| origin.distance(pointer) > click_tolerance)
        });
        if !response.rect.contains(pointer)
            || !self.tools.measure.ruler_drag_follows(moved_past_click)
        {
            return;
        }
        let Some(camera) = self.render.camera else {
            return;
        };
        let Some(anchor) = self.tools.measure.dragged_ruler_anchor() else {
            return;
        };
        let moved = if let Some(base) = self.tools.measure.dragged_line_end_base() {
            // Sliding an end along its line is free; Shift squares it.
            let angle = if ctx.input(|input| input.modifiers.shift) {
                RulerLineAngle::Perpendicular
            } else {
                RulerLineAngle::Free
            };
            self.tools
                .measure
                .ruler_segment(base)
                .and_then(|base| {
                    measure_overlay::line_placement(&camera, response.rect, &base, pointer, angle)
                })
                .and_then(|placement| self.tools.measure.update_line_end_drag(placement))
        } else {
            let Some(scene) = self.document.scene.clone() else {
                return;
            };
            pick_scene_hit(&camera, response.rect, pointer, &scene)
                .and_then(|hit| self.tools.measure.update_ruler_drag(hit.point))
        };
        if moved.is_some() {
            self.report_ruler(anchor.ruler_index);
            ctx.request_repaint();
        }
    }

    /// Apply one on-mesh measure click for the armed mode.
    fn apply_measure_click(&mut self, scene: &Scene, hit: ScenePickHit) {
        match self.tools.measure.mode() {
            Some(MeasureMode::Ruler) => {
                if self.tools.measure.place_ruler_point(hit.point).is_some() {
                    self.report_ruler(self.tools.measure.ruler_count() - 1);
                }
            }
            Some(MeasureMode::Thickness) => self.apply_thickness_probe(scene, hit),
            None => {}
        }
    }

    /// Put the reading of ruler `ruler_index` on the status line in the
    /// operator's unit: its length, and for a ruler on another ruler's line
    /// the angle between the two.
    fn report_ruler(&mut self, ruler_index: usize) {
        let Some(ruler) = self.tools.measure.ruler_segment(ruler_index) else {
            return;
        };
        let number_format = self.ui.locale.number_format();
        let length = measure_tool::format_length(
            ruler.distance_mm(),
            self.persistence.settings.unit_display,
            number_format,
        );
        let message = match ruler.foot {
            Some(foot) if foot.perpendicular => self.ui.locale.tr_with(
                crate::i18n::message_id!("measure-perpendicular"),
                &[("len", length.as_str())],
            ),
            Some(foot) if foot.angle_deg.is_some() => {
                let angle = foot
                    .angle_deg
                    .map_or_else(String::new, |degrees| {
                        measure_tool::format_angle(degrees, number_format)
                    });
                self.ui.locale.tr_with(
                    crate::i18n::message_id!("measure-to-line"),
                    &[("len", length.as_str()), ("angle", angle.as_str())],
                )
            }
            _ => self.ui.locale.tr_with(
                crate::i18n::message_id!("measure-distance"),
                &[("len", length.as_str())],
            ),
        };
        self.scene_ui.status_message = Some(message);
    }

    /// Probe the wall of the hit layer and report the reading.
    fn apply_thickness_probe(&mut self, scene: &Scene, hit: ScenePickHit) {
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return;
        };
        if entry.id() != hit.layer_id {
            return;
        }
        match measure_tool::probe_wall_thickness(entry, hit.triangle_index, hit.point) {
            Some(probe) => {
                self.scene_ui.status_message = Some(match probe.reading {
                    ThicknessReading::Wall { thickness_mm, .. } => self.ui.locale.tr_with(
                        crate::i18n::message_id!("measure-thickness"),
                        &[(
                            "len",
                            measure_tool::format_length(
                                f64::from(thickness_mm),
                                self.persistence.settings.unit_display,
                                self.ui.locale.number_format(),
                            )
                            .as_str(),
                        )],
                    ),
                    ThicknessReading::Open => self
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("measure-open-wall")),
                });
                self.tools.measure.set_probe(probe);
                // The same click also opens the Cut View at this cross-section
                // (Wall readings only), showing the same chord.
                self.drive_probe_cut_view(scene, &probe);
            }
            None => {
                self.scene_ui.status_message = Some(
                    self.ui
                        .locale
                        .tr(crate::i18n::message_id!("measure-cannot-probe")),
                );
            }
        }
    }

    /// Open (or re-aim) the probe-linked Cut View from a thickness reading.
    ///
    /// A `Wall` reading with a buildable cross-section plane plants a world-fixed
    /// disc whose plane contains the entry->exit chord, so the wall reads edge-on
    /// in the Section panel with the same measurement. An `Open` reading (or a
    /// degenerate chord that cannot be sectioned) plants nothing and closes any
    /// cut view this probe flow had opened: there is nothing to section.
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
                    .render
                    .camera
                    .map_or(pose.center + pose.plane_normal, occluview_core::Camera::eye);
                let keep_positive = crate::cut::cut_geometry::camera_keep_side(&pose, eye);
                let seed = probe_section::SliceProbe {
                    entry: probe.entry,
                    exit,
                    thickness_mm,
                };
                self.tools
                    .cut_view
                    .plant_from_probe(pose, keep_positive, seed);
                self.render.invalidation.overlay_tools_changed();
            }
            None => {
                if self.tools.cut_view.is_probe_linked() {
                    self.tools.cut_view.disable();
                    self.render.invalidation.overlay_tools_changed();
                }
            }
        }
    }
}

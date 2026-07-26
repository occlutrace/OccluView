//! Painting the match region while Align Scans is armed.
//!
//! A dab is a plain pass over the moving layer's vertices with a distance
//! test — no acceleration structure, no worker. That is deliberate: it costs a
//! few milliseconds even on a dense scan, and routing it through the worker
//! would mean the paint lagged the cursor. The expensive half — re-measuring
//! the deviation map — still goes to the worker, and only once the stroke ends
//! rather than on every dab.
//!
//! The markings are also *shown*. A brush whose only evidence is a number in
//! the statistics is a brush an operator cannot aim: marked surface goes blue,
//! exactly as it does in exocad, and the tint comes off with the window.

use std::sync::Arc;

use eframe::egui;
use glam::DVec3;
use occluview_align::{MaskEdit, Rigid};

use super::app_align_display::AlignOverlay;
use super::OccluViewApp;
use crate::align_brush::MaskCommand;
use crate::viewer::pick_scene_hit;

/// Tint for surface that still takes part in the match — a neutral stone, so
/// the marked surface is the only thing that draws the eye.
const REGION_IN_COLOR: [u8; 4] = [228, 216, 196, 255];
/// Tint for surface marked out of the match. Blue, because that is the colour
/// exocad paints an excluded region, and an operator who works in that dialog
/// should not have to learn a second convention here.
const REGION_OUT_COLOR: [u8; 4] = [58, 108, 196, 255];

impl OccluViewApp {
    /// Paint or erase under the pointer. Returns whether the brush owns this
    /// frame's pointer.
    pub(super) fn handle_align_brush(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.align_brush.is_armed() {
            return false;
        }
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if !primary_down {
            // The stroke ended. The region changed what would be measured, so a
            // map drawn before it is stale — drop it rather than silently
            // recomputing behind the operator's hand.
            if self.align_mask_stroke_open {
                self.align_mask_stroke_open = false;
                self.invalidate_deviation_map("Region changed");
                // The release frame still reads as a click. An armed brush owns
                // it, or one dab would also drop an alignment point.
                return true;
            }
            return false;
        }
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|input| input.pointer.hover_pos()))
        else {
            return false;
        };
        // The brush's own size slider sits inches from the cursor, and the
        // window floats over the scan. Without this, dragging that slider
        // paints a dab per frame on whatever is behind the window — silently.
        if !self.pointer_on_bare_viewport(ctx, response.rect, pointer) {
            return false;
        }
        let Some((camera, scene)) = self.camera.zip(self.scene.clone()) else {
            return false;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return true;
        };
        if Some(hit.layer_id) != self.align.moving_layer() {
            self.align_status = Some("The region is painted on the scan that moves".into());
            return true;
        }
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return true;
        };
        let Some(pose) = Rigid::from_affine(&entry.transform) else {
            return true;
        };

        let vertex_count = entry.mesh.vertices().len();
        let mut mask = match self.align_mask.as_ref() {
            Some(existing) if existing.len() == vertex_count => existing.as_ref().clone(),
            _ => vec![occluview_align::INCLUDED; vertex_count],
        };
        let positions: Vec<f32> = entry
            .mesh
            .vertices()
            .iter()
            .flat_map(|vertex| vertex.position)
            .collect();

        // exocad's rule: a plain drag marks, Shift inverses the brush, and the
        // Brush inverse toggle inverses it standing. Held together they cancel.
        let shift = ctx.input(|input| input.modifiers.shift);
        let changed = occluview_align::apply_brush(
            &mut mask,
            &positions,
            pose,
            &MaskEdit {
                center: DVec3::new(
                    f64::from(hit.point.x),
                    f64::from(hit.point.y),
                    f64::from(hit.point.z),
                ),
                radius_mm: f64::from(self.align_brush.radius_mm()),
                erase: self.align_brush.erases(shift),
            },
        );

        self.align_mask_stroke_open = true;
        if changed > 0 {
            self.set_align_mask(Some(mask));
            self.refresh_align_region_preview();
            ctx.request_repaint();
        }
        true
    }

    /// Shift+wheel resizes the brush instead of zooming the camera — the same
    /// gesture the sculpt brush uses, so there is one size gesture in the whole
    /// application rather than one per tool.
    pub(super) fn handle_align_brush_wheel(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.align_brush.is_armed() {
            return false;
        }
        // Over the viewport itself, not over a window floating on it: the
        // brush's own size slider takes a plain wheel, and a shifted wheel
        // there must not also resize the brush behind it.
        let over_viewport = ctx
            .pointer_hover_pos()
            .is_some_and(|pointer| self.pointer_on_bare_viewport(ctx, response.rect, pointer));
        if !over_viewport {
            return false;
        }
        let (scroll, shift) = ctx.input(|input| {
            // Some platforms turn a shifted wheel into HORIZONTAL scroll, so
            // read whichever axis actually moved.
            let raw = input.raw_scroll_delta;
            let axis = if raw.y.abs() >= raw.x.abs() {
                raw.y
            } else {
                raw.x
            };
            (axis, input.modifiers.shift)
        });
        if !shift || scroll.abs() < f32::EPSILON {
            return false;
        }
        self.align_brush.nudge_radius(scroll.signum());
        self.align_status = Some(format!("Brush {:.1} mm", self.align_brush.radius_mm()));
        ctx.request_repaint();
        true
    }

    /// Draw the brush footprint under the cursor.
    ///
    /// Without it the operator is aiming a millimetre-sized tool with no idea
    /// how much of the scan it covers, which on an arch is the difference
    /// between excluding a bubble and excluding a quadrant.
    pub(super) fn paint_align_brush_cursor(
        &self,
        ui: &egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) {
        if !self.align_brush.is_armed() {
            return;
        }
        let (Some(camera), Some(pointer)) = (self.camera.as_ref(), ctx.pointer_hover_pos()) else {
            return;
        };
        if !self.pointer_on_bare_viewport(ctx, viewport_rect, pointer) {
            return;
        }
        // The viewport camera is orthographic, so a millimetre maps to a fixed
        // number of pixels regardless of depth.
        let ortho_height = camera.orthographic_height.max(f32::EPSILON);
        let radius_px = self.align_brush.radius_mm() * viewport_rect.height() / ortho_height;
        if !radius_px.is_finite() || radius_px < 2.0 {
            return;
        }
        // exocad paints with a green tool and clears with a red one; the ring
        // says which of the two this drag will be, Shift included.
        let shift = ctx.input(|input| input.modifiers.shift);
        let ink = if self.align_brush.erases(shift) {
            egui::Color32::from_rgb(196, 82, 72)
        } else {
            egui::Color32::from_rgb(72, 158, 108)
        };
        let canvas = ui.painter();
        canvas.circle_filled(pointer, radius_px, ink.gamma_multiply(0.10));
        canvas.circle_stroke(pointer, radius_px, egui::Stroke::new(1.2, ink));
        canvas.circle_filled(pointer, 1.5, ink.gamma_multiply(0.7));
    }

    /// Install a mask, stamping it with a fresh revision.
    ///
    /// The mask decides which vertices are measured, so the worker's cached
    /// deviation map is only reusable while the mask is the same one. Every
    /// write goes through here so a future edit cannot forget to say the mask
    /// moved and leave a stale map on screen — see
    /// `every_mask_write_stamps_a_new_revision`.
    fn set_align_mask(&mut self, mask: Option<Vec<u8>>) {
        self.align_mask = mask.map(Arc::new);
        self.align_mask_revision = self.align_mask_revision.wrapping_add(1);
    }

    /// Apply one whole-region command from the panel.
    pub(super) fn apply_align_mask_command(&mut self, command: MaskCommand) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(moving) = self
            .align
            .moving_layer()
            .and_then(|id| super::app_align::layer_of(&scene, id))
        else {
            return;
        };
        let vertex_count = moving.mesh.vertices().len();
        let mut mask = match self.align_mask.as_ref() {
            Some(existing) if existing.len() == vertex_count => existing.as_ref().clone(),
            _ => vec![occluview_align::INCLUDED; vertex_count],
        };
        match command {
            MaskCommand::FitEverywhere => occluview_align::set_all(&mut mask, false),
            MaskCommand::FitNowhere => occluview_align::set_all(&mut mask, true),
            MaskCommand::InvertMarkings => occluview_align::invert(&mut mask),
            MaskCommand::MarkAutomatic => {
                // exocad "Mark automatic": keep only a disc of surface at each
                // arrow end, and mark everything else out. Written as
                // mark-everything then clear-the-discs, because the discs are
                // what the operator wants MATCHED and the mask stores what is
                // ignored.
                let Some(pose) = Rigid::from_affine(&moving.transform) else {
                    return;
                };
                if self.align.pairs().is_empty() {
                    self.align_status =
                        Some("Place at least one arrow before marking automatically".into());
                    return;
                }
                occluview_align::set_all(&mut mask, true);
                let positions: Vec<f32> = moving
                    .mesh
                    .vertices()
                    .iter()
                    .flat_map(|vertex| vertex.position)
                    .collect();
                let radius = f64::from(self.align_brush.auto_radius_mm());
                for pair in self.align.pairs() {
                    let world = moving.transform.transform_point3(pair.moving.local);
                    occluview_align::apply_brush(
                        &mut mask,
                        &positions,
                        pose,
                        &MaskEdit {
                            center: DVec3::new(
                                f64::from(world.x),
                                f64::from(world.y),
                                f64::from(world.z),
                            ),
                            radius_mm: radius,
                            erase: true,
                        },
                    );
                }
            }
        }
        self.set_align_mask(Some(mask));
        self.align_status = Some(command.report().into());
        self.invalidate_deviation_map(command.report());
        self.refresh_align_region_preview();
    }

    /// Drop the mask — done whenever the pair changes, since a mask is indexed
    /// by one layer's vertices.
    pub(super) fn clear_align_mask(&mut self) {
        self.set_align_mask(None);
        self.align_mask_stroke_open = false;
    }

    /// Put the region tint on the moving scan, take it off, or leave it alone.
    ///
    /// Called whenever paint mode or the mask changes. A brush that leaves no
    /// mark on the surface is one an operator cannot aim, and the operator's
    /// verdict on the previous build was exactly that.
    pub(super) fn refresh_align_region_preview(&mut self) {
        if !self.align_brush.is_armed() {
            if self.align_overlay == AlignOverlay::Region {
                self.clear_deviation_overlay();
                // The map was taken down to make room for the markings. Closing
                // the brush is the moment to put it back, or an operator who
                // opened the brush to fix one region loses the reading they
                // opened it because of.
                self.measure_if_shown();
            }
            return;
        }
        // A measured map and a region tint are both per-vertex colours on the
        // same layer, so only one of them can be up. Paint mode wins: the
        // operator is about to change what the map measured anyway.
        if self.align_overlay == AlignOverlay::Map {
            self.clear_deviation_overlay();
        }
        let Some(colors) = self.align_region_colors() else {
            // Nothing to paint on yet. Silence here reads as a broken brush;
            // the operator's actual problem is that no scan has been named.
            self.align_status =
                Some("Click a point on the scan that should move, then paint on it".into());
            return;
        };
        self.apply_region_colors(colors);
    }

    /// What share of the moving scan is marked out of the match, if there is a
    /// mask at all. `None` while there is nothing to report.
    pub(super) fn align_marked_fraction(&self) -> Option<f32> {
        let mask = self.align_mask.as_ref()?;
        // A mask left over from other geometry is not a reading about this
        // mesh. Reporting its fraction would be a number about nothing.
        let scene = self.scene.as_ref()?;
        let moving = self
            .align
            .moving_layer()
            .and_then(|id| super::app_align::layer_of(scene, id))?;
        if mask.is_empty() || mask.len() != moving.mesh.vertices().len() {
            return None;
        }
        // `bytecount` would be the crate for this, but it is one dependency for
        // one counter over an array the operator's own brush strokes bounded.
        #[allow(clippy::naive_bytecount)]
        let marked = mask
            .iter()
            .filter(|slot| **slot == occluview_align::EXCLUDED)
            .count();
        #[allow(clippy::cast_precision_loss)]
        Some(marked as f32 / mask.len() as f32)
    }

    /// One colour per vertex of the moving scan: stone where it takes part in
    /// the match, blue where it has been marked out.
    fn align_region_colors(&self) -> Option<Vec<[u8; 4]>> {
        let scene = self.scene.as_ref()?;
        let moving = self
            .align
            .moving_layer()
            .and_then(|id| super::app_align::layer_of(scene, id))?;
        let vertices = moving.mesh.vertices();
        let count = vertices.len();
        // A mask of the wrong length belongs to different geometry. Reading it
        // by index would paint arbitrary vertices blue and call it a region.
        let mask = self
            .align_mask
            .as_ref()
            .filter(|mask| mask.len() == count)
            .map(|mask| mask.as_ref().as_slice());
        // A coloured scan keeps its own colours where nothing is marked. The
        // operator is usually aiming AT something they can see — a stain, a
        // bubble, a bite block — and flattening the surface to one tint takes
        // away the very thing they were aiming at.
        let own_colors = moving.mesh.has_vertex_colors();
        Some(
            (0..count)
                .map(|vertex| {
                    if mask.is_some_and(|mask| mask[vertex] == occluview_align::EXCLUDED) {
                        return REGION_OUT_COLOR;
                    }
                    match vertices.get(vertex) {
                        Some(vertex) if own_colors => vertex.color,
                        _ => REGION_IN_COLOR,
                    }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::align_brush::MaskCommand;

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = include_str!("app_align_brush.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// Painting changes what would be measured, so a map drawn before the
    /// stroke describes a comparison that no longer exists. Dropping it is
    /// honest; silently recomputing behind the operator's hand is not, and
    /// recomputing per dab would also be slow.
    #[test]
    fn a_stroke_drops_the_map_instead_of_recomputing_it() {
        let production = production();
        assert!(
            production.contains("self.invalidate_deviation_map("),
            "a mask change must invalidate the map"
        );
        // Scoped to the stroke: CLOSING the brush does re-measure, because the
        // map was taken down to make room for the markings and the operator is
        // asking for it back. Measuring per dab is the thing that must not
        // happen — it is seconds of work behind a moving hand.
        let stroke = production
            .split_once("fn handle_align_brush(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("fn handle_align_brush_wheel("))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            !stroke.contains("measure_if_shown") && !stroke.contains("run_align_measure"),
            "a stroke must never kick off a measurement"
        );
    }

    /// The mask decides which vertices are measured, so the worker only reuses
    /// a cached measurement while the mask is the same one. A write that did
    /// not stamp a new revision would leave the pre-stroke map on screen and
    /// call it a measurement of the masked scan.
    #[test]
    fn every_mask_write_stamps_a_new_revision() {
        let production = production();
        assert_eq!(
            production.matches("self.align_mask =").count(),
            1,
            "the mask must only be written inside set_align_mask"
        );
        let setter = production
            .split_once("fn set_align_mask(")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        assert!(
            setter.contains("self.align_mask_revision = self.align_mask_revision.wrapping_add(1)"),
            "installing a mask must stamp a fresh revision"
        );
        assert_eq!(
            production.matches("self.set_align_mask(").count(),
            3,
            "the brush stroke, the whole-region commands, and Clear all write the mask"
        );
    }

    /// Every command must actually reach the mask, or a panel button is a lie.
    #[test]
    fn every_mask_command_is_handled() {
        let source = include_str!("app_align_brush.rs");
        for command in [
            MaskCommand::FitEverywhere,
            MaskCommand::FitNowhere,
            MaskCommand::InvertMarkings,
            MaskCommand::MarkAutomatic,
        ] {
            let name = format!("MaskCommand::{command:?}");
            assert!(
                source.contains(name.as_str()),
                "{name} is declared but never applied"
            );
        }
    }

    /// exocad's rule: a plain drag marks, Shift inverses the brush, and the
    /// Brush inverse toggle inverses it standing. Both have to reach the same
    /// decision or the toggle and the key would fight.
    #[test]
    fn a_stroke_takes_its_direction_from_the_toggle_and_shift_together() {
        let stroke = production()
            .split_once("fn handle_align_brush(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("fn handle_align_brush_wheel("))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            stroke.contains("erase: self.align_brush.erases(shift)"),
            "the stroke direction must come from the brush, Shift included"
        );
    }

    /// A brush that leaves no mark on the surface cannot be aimed. Every path
    /// that changes the region has to refresh what is on screen.
    #[test]
    fn every_region_change_refreshes_what_the_operator_sees() {
        let production = production();
        assert_eq!(
            production
                .matches("self.refresh_align_region_preview()")
                .count(),
            2,
            "a stroke and a whole-region command both change the surface"
        );
    }
}

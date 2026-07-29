//! Painting the markings that best-fit matching must ignore.
//!
//! Three things make this fast enough to feel like a brush rather than a
//! progress bar, and all three had to be true at once:
//!
//! 1. **Nothing is rebuilt per dab.** The flat position array comes from the
//!    geometry cache and the mask is edited in place. Rebuilding them cost
//!    seven milliseconds and a megabyte of churn per dab.
//! 2. **Only the marked vertices are re-coloured and re-uploaded.** A dab the
//!    size of a cusp touches a few hundred vertices out of a million; the old
//!    path repainted and re-uploaded all of them, thirty-four megabytes each
//!    way, which is exactly the three frames a second the operator reported.
//! 3. **The dab itself is parallel** — three milliseconds on a 942k-vertex arch.
//!
//! There is no mesh picker. exocad needs one because its brush is modal; here
//! the brush marks whichever mesh is under the cursor, so painting both sides
//! of a comparison is one continuous gesture and there is nothing to get wrong.

use eframe::egui;
use glam::DVec3;
use occluview_align::{MaskEdit, Rigid};
use occluview_core::{SceneMesh, SceneMeshId};

use super::app_align::layer_of;
use super::app_align_display::AlignOverlay;
use super::OccluViewApp;
use crate::align_markings::{AlignSide, AutoKeep, MarkedMesh, MaskCommand};
use crate::viewer::pick_scene_hit;

/// Tint for surface that still takes part in the match — a neutral stone, so
/// the marked surface is the only thing that draws the eye.
const REGION_IN_COLOR: [u8; 4] = [228, 216, 196, 255];
/// Tint for surface marked out of the match. Blue, because that is the colour
/// exocad paints an excluded region, and an operator who works in that dialog
/// should not have to learn a second convention here.
pub(super) const REGION_OUT_COLOR: [u8; 4] = [58, 108, 196, 255];

impl OccluViewApp {
    /// Paint or clear under the pointer. Returns whether the brush owns this
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
            // The stroke ended. The markings changed what would be matched and
            // measured, so a map drawn before them is stale — drop it rather
            // than silently recomputing behind the operator's hand.
            if self.align_markings.close_stroke() {
                self.invalidate_deviation_map("Markings changed");
                // The release frame still reads as a click. An armed brush owns
                // it, or one dab would also drop an alignment arrow.
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
        // The brush's own size slider sits inches from the cursor, and its
        // window floats over the mesh. Without this, dragging that slider
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
        // Whichever mesh is under the cursor. Both sides of a comparison can
        // carry markings, and asking which one to paint on would be a picker
        // this tool has spent its whole design avoiding.
        let painting = if Some(hit.layer_id) == self.align.moving_layer() {
            AlignSide::Moving
        } else if Some(hit.layer_id) == self.align.fixed_layer() {
            AlignSide::Fixed
        } else {
            self.align_status = Some("That mesh is not in this alignment".into());
            return true;
        };
        let Some(entry) = layer_of(&scene, hit.layer_id) else {
            return true;
        };
        let Some(pose) = Rigid::from_affine(&entry.transform) else {
            return true;
        };

        let erase = self
            .align_brush
            .erases(ctx.input(|input| input.modifiers.shift));
        let radius_mm = f64::from(self.align_brush.radius_mm());
        let center = DVec3::new(
            f64::from(hit.point.x),
            f64::from(hit.point.y),
            f64::from(hit.point.z),
        );
        // Cached: rebuilding this per dab was seven milliseconds of pure copy.
        let positions = self.align_geometry.local_positions(entry);
        let mesh = MarkedMesh {
            positions: &positions,
            pose,
            vertex_count: entry.mesh.vertices().len(),
        };
        let changed = self.align_markings.dab(
            painting,
            &mesh,
            &MaskEdit {
                center,
                radius_mm,
                erase,
            },
        );
        if changed > 0 {
            self.patch_region_preview(hit.layer_id, painting);
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
    /// how much of the mesh it covers, which on an arch is the difference
    /// between marking out a bubble and marking out a quadrant.
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
        let mm_per_pixel =
            crate::align_drag::mm_per_pixel(camera.orthographic_height, viewport_rect.height());
        let radius_px = self.align_brush.radius_mm() / mm_per_pixel;
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

    /// Apply one whole-mesh command from the Brush tool window.
    ///
    /// Both sides at once, because the buttons say "the mesh" and an operator
    /// who has marked a region on each does not expect Fit everywhere to clear
    /// only one of them.
    pub(super) fn apply_align_mask_command(&mut self, command: MaskCommand) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let mut reached = false;
        for side in AlignSide::BOTH {
            let Some(layer) = self.side_layer(side) else {
                continue;
            };
            let Some(entry) = layer_of(&scene, layer).cloned() else {
                continue;
            };
            if self.apply_mask_command_to(command, side, &entry) {
                reached = true;
                self.repaint_region_preview(layer, side);
            }
        }
        if !reached {
            // "Mark automatic" is the only command that can decline, and it
            // declines for one reason the operator can act on.
            if command == MaskCommand::MarkAutomatic {
                self.align_status =
                    Some("Place at least one arrow before marking automatically".into());
            }
            return;
        }
        self.align_status = Some(command.report().into());
        self.invalidate_deviation_map(command.report());
    }

    /// Run one command against one side. Returns whether it reached a mask.
    fn apply_mask_command_to(
        &mut self,
        command: MaskCommand,
        side: AlignSide,
        entry: &SceneMesh,
    ) -> bool {
        let Some(pose) = Rigid::from_affine(&entry.transform) else {
            return false;
        };
        // "Mark automatic" keeps a disc at each arrow end, and the arrows only
        // touch the surface they were clicked on.
        let keep: Vec<DVec3> = if command == MaskCommand::MarkAutomatic {
            self.side_arrow_points(side, entry)
        } else {
            Vec::new()
        };
        let positions = self.align_geometry.local_positions(entry);
        let mesh = MarkedMesh {
            positions: &positions,
            pose,
            vertex_count: entry.mesh.vertices().len(),
        };
        let keep = AutoKeep {
            centres: &keep,
            radius_mm: f64::from(self.align_brush.auto_radius_mm()),
        };
        self.align_markings.command(side, command, &mesh, &keep)
    }

    /// The world positions of the arrow ends that sit on one side's mesh.
    fn side_arrow_points(&self, side: AlignSide, entry: &SceneMesh) -> Vec<DVec3> {
        self.align
            .pairs()
            .iter()
            .map(|pair| match side {
                AlignSide::Moving => pair.moving.local,
                AlignSide::Fixed => pair.fixed.local,
            })
            .map(|local| {
                let world = entry.transform.transform_point3(local);
                DVec3::new(f64::from(world.x), f64::from(world.y), f64::from(world.z))
            })
            .collect()
    }

    /// Which layer one side of the alignment is.
    fn side_layer(&self, side: AlignSide) -> Option<SceneMeshId> {
        match side {
            AlignSide::Moving => self.align.moving_layer(),
            AlignSide::Fixed => self.align.fixed_layer(),
        }
    }

    /// Drop both masks — done whenever the pair changes, since a mask is
    /// indexed by one layer's vertices.
    pub(super) fn clear_align_mask(&mut self) {
        self.align_markings.clear();
    }

    /// What share of the two scans is marked, if either carries a mask that
    /// fits its mesh.
    ///
    /// The panel asks every frame the Brush window is open. Walking both masks
    /// to answer would be a two-million-byte scan per frame, so the counts are
    /// maintained where the masks are edited and this only reads them.
    pub(super) fn align_marked_fraction(&self) -> Option<f32> {
        let scene = self.scene.as_ref()?;
        let vertices = |side: AlignSide| -> usize {
            self.side_layer(side)
                .and_then(|id| layer_of(scene, id))
                .map_or(0, |entry| entry.mesh.vertices().len())
        };
        self.align_markings
            .marked_fraction(vertices(AlignSide::Moving), vertices(AlignSide::Fixed))
    }

    /// Put the markings on both meshes, take them off, or leave them alone.
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
        // A measured map and the markings are both per-vertex colours on the
        // same layers, so only one of them can be up. The brush wins: the
        // operator is about to change what the map measured anyway.
        if self.align_overlay == AlignOverlay::Map {
            self.clear_deviation_overlay();
        }
        // A measurement already running would land on top of the markings a
        // moment later. Retiring the generation drops it at the door instead of
        // letting it race the brush.
        if let Some(worker) = self.align_worker.as_ref() {
            worker.bump_generation();
        }
        let mut reached = false;
        for side in AlignSide::BOTH {
            if let Some(layer) = self.side_layer(side) {
                reached |= self.repaint_region_preview(layer, side);
            }
        }
        if !reached {
            // Silence here reads as a broken brush; the operator's actual
            // problem is that no mesh has been named.
            self.align_status =
                Some("Click a point on each mesh first, then paint on either".into());
        }
    }

    /// Rebuild one layer's markings in full.
    fn repaint_region_preview(&mut self, layer: SceneMeshId, side: AlignSide) -> bool {
        let Some(colors) = self.region_colors(layer, side) else {
            return false;
        };
        self.attach_overlay_colors(layer, colors, AlignOverlay::Region)
    }

    /// Rewrite only the vertices the last dab touched.
    ///
    /// Falls back to a full repaint when there is nothing to patch into yet —
    /// the first dab of a session, or the frame after the markings were
    /// dropped. Every dab after that costs a few hundred vertex writes.
    fn patch_region_preview(&mut self, layer: SceneMeshId, side: AlignSide) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(entry) = layer_of(&scene, layer) else {
            return;
        };
        let count = entry.mesh.vertices().len();
        let Some(mask) = self.align_markings.mask_for(side, count) else {
            self.repaint_region_preview(layer, side);
            return;
        };
        let own_colors = entry.mesh.has_vertex_colors();
        let vertices = entry.mesh.vertices();
        let recolor = |vertex: usize| region_color(vertices, own_colors, Some(&mask), vertex);
        if !self.patch_overlay_colors(layer, recolor) {
            self.repaint_region_preview(layer, side);
        }
    }

    /// One colour per vertex of one side: its own colour where nothing is
    /// marked, blue where it is.
    fn region_colors(&self, layer: SceneMeshId, side: AlignSide) -> Option<Vec<[u8; 4]>> {
        let scene = self.scene.as_ref()?;
        let entry = layer_of(scene, layer)?;
        let vertices = entry.mesh.vertices();
        let count = vertices.len();
        let mask = self.align_markings.mask_for(side, count);
        let mask = mask.as_ref().map(|mask| mask.as_slice());
        // A coloured scan keeps its own colours where nothing is marked. The
        // operator is usually aiming AT something they can see — a stain, a
        // bubble, a bite block — and flattening the surface to one tint takes
        // away the very thing they were aiming at.
        let own_colors = entry.mesh.has_vertex_colors();
        Some(
            (0..count)
                .map(|vertex| region_color(vertices, own_colors, mask, vertex))
                .collect(),
        )
    }
}

/// The colour one vertex takes while the markings are on screen.
///
/// One function, because two paths ask the question: the full repaint that
/// installs the markings and the per-dab patch that keeps them up to date. Two
/// copies of this rule would drift, and the drift would look like the brush
/// painting a colour the mask does not have.
fn region_color(
    vertices: &[occluview_core::Vertex],
    own_colors: bool,
    mask: Option<&[u8]>,
    vertex: usize,
) -> [u8; 4] {
    if mask.and_then(|mask| mask.get(vertex).copied()) == Some(occluview_align::EXCLUDED) {
        return REGION_OUT_COLOR;
    }
    match vertices.get(vertex) {
        Some(vertex) if own_colors => vertex.color,
        _ => REGION_IN_COLOR,
    }
}

#[cfg(test)]
mod tests {

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = include_str!("app_align_brush.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// The stroke handler's own body, which several contracts are about.
    fn stroke() -> &'static str {
        production()
            .split_once("fn handle_align_brush(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("fn handle_align_brush_wheel("))
            .map(|(body, _)| body)
            .unwrap_or_default()
    }

    /// Painting changes what would be matched, so a map drawn before the stroke
    /// describes a comparison that no longer exists. Dropping it is honest;
    /// silently recomputing behind the operator's hand is not, and recomputing
    /// per dab would also be slow.
    #[test]
    fn a_stroke_drops_the_map_instead_of_recomputing_it() {
        assert!(
            production().contains("self.invalidate_deviation_map("),
            "a mask change must invalidate the map"
        );
        // Scoped to the stroke: CLOSING the brush does re-measure, because the
        // map was taken down to make room for the markings and the operator is
        // asking for it back. Measuring per dab is the thing that must not
        // happen — it is most of a second behind a moving hand.
        let stroke = stroke();
        assert!(
            !stroke.contains("measure_if_shown") && !stroke.contains("run_align_measure"),
            "a stroke must never kick off a measurement"
        );
    }

    /// The two things this file, and only this file, is responsible for keeping
    /// cheap. What a dab does to the mask itself is covered by real tests over
    /// `AlignMarkings`; these are the wiring around it, which has no behaviour
    /// of its own to run.
    #[test]
    fn a_dab_reuses_the_cached_geometry_and_re_colours_only_what_it_touched() {
        let stroke = stroke();
        assert!(
            stroke.contains("self.align_geometry.local_positions(entry)"),
            "the positions must come from the cache, not a fresh copy per dab"
        );
        assert!(
            !stroke.contains("flat_map(|vertex| vertex.position)"),
            "a dab must not rebuild the position array"
        );
        assert!(
            stroke.contains("self.patch_region_preview("),
            "a dab must re-colour only what it touched"
        );
    }

    /// The operator's report: "on one surface it marks, on the other nothing".
    /// That both sides carry markings is tested for real over `AlignMarkings`;
    /// what this file owns is where the side comes from — the cursor, not a
    /// picker.
    #[test]
    fn the_side_a_dab_lands_on_comes_from_the_cursor() {
        let stroke = stroke();
        assert!(
            stroke.contains("Some(hit.layer_id) == self.align.moving_layer()")
                && stroke.contains("Some(hit.layer_id) == self.align.fixed_layer()"),
            "the side must come from what is under the cursor"
        );
    }

    /// exocad's rule: a plain drag marks, Shift inverses the brush, and the
    /// Brush inverse toggle inverses it standing. Both have to reach the same
    /// decision or the toggle and the key would fight.
    #[test]
    fn a_stroke_takes_its_direction_from_the_toggle_and_shift_together() {
        assert!(
            production().contains(".erases(ctx.input(|input| input.modifiers.shift))"),
            "the stroke direction must come from the brush, Shift included"
        );
    }
}

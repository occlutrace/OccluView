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
//! The Brush window has an explicit mesh selection. The two scans overlap by
//! design, so choosing the nearest ray hit would make a stroke change the
//! wrong mask whenever the other scan is slightly closer to the camera.

use eframe::egui;
use glam::DVec3;
use occluview_align::{MaskEdit, Rigid};
use occluview_core::{SceneMesh, SceneMeshId};

use super::app_align::layer_of;
use super::app_align_display::AlignOverlay;
use super::OccluViewApp;
use crate::align_markings::{AlignSide, AutoKeep, MarkedMesh, MarkedOn, MaskCommand};
use crate::viewer::pick_layer_hit;

/// The identity a mask painted on this layer has to match later.
fn marked_on(entry: &SceneMesh) -> MarkedOn {
    MarkedOn {
        geometry: entry.mesh.geometry_id(),
        vertex_count: entry.mesh.vertices().len(),
    }
}

fn resize_align_brush_from_wheel(
    brush: &mut crate::align_brush::AlignBrush,
    ctx: &egui::Context,
) -> bool {
    // Some platforms turn a shifted wheel into HORIZONTAL scroll, so read
    // whichever axis actually moved.
    let raw = super::app_input::raw_wheel_delta(ctx);
    let scroll = if raw.y.abs() >= raw.x.abs() {
        raw.y
    } else {
        raw.x
    };
    let shift = ctx.input(|input| input.modifiers.shift);
    if !shift || scroll.abs() < f32::EPSILON {
        return false;
    }
    brush.nudge_radius(scroll.signum());
    true
}

impl OccluViewApp {
    /// Paint or clear under the pointer. Returns whether the brush owns this
    /// frame's pointer.
    pub(super) fn handle_align_brush(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.tools.align.brush.is_armed() {
            return false;
        }
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if !primary_down {
            // The stroke ended. The markings changed what would be matched and
            // measured, so a map drawn before them is stale — drop it rather
            // than silently recomputing behind the operator's hand.
            if self.tools.align.markings.close_stroke() {
                self.invalidate_deviation_map(&self.ui.locale.tr("align-status-markings-changed"));
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
        // Everything that reads the scene happens inside this block, so the
        // handle it clones is gone before the dab is drawn into the scene
        // below. `Arc::make_mut` copies the whole case while a second handle
        // is alive, and this is a per-frame path.
        let (layer_id, painting, changed) = {
            let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) else {
                return false;
            };
            let painting = self.tools.align.brush.target_side();
            let Some(layer_id) = self.side_layer(painting) else {
                self.tools.align.status = Some(self.ui.locale.tr("brush-no-mesh"));
                return true;
            };
            let Some(hit) = pick_layer_hit(&camera, response.rect, pointer, &scene, layer_id)
            else {
                return true;
            };
            // The picker is deliberately scoped to the selected layer. A
            // nearest-hit picker would paint the other overlapping scan.
            let Some(entry) = layer_of(&scene, layer_id) else {
                return true;
            };
            let Some(pose) = Rigid::from_affine(&entry.transform) else {
                return true;
            };

            let erase = self
                .tools
                .align
                .brush
                .erases(ctx.input(|input| input.modifiers.shift));
            let radius_mm = f64::from(self.tools.align.brush.radius_mm());
            let center = DVec3::new(
                f64::from(hit.point.x),
                f64::from(hit.point.y),
                f64::from(hit.point.z),
            );
            // Cached: rebuilding this per dab was seven milliseconds of pure copy.
            let positions = self.tools.align.geometry.local_positions(entry);
            let mesh = MarkedMesh {
                positions: &positions,
                pose,
                vertex_count: entry.mesh.vertices().len(),
                geometry: entry.mesh.geometry_id(),
            };
            let changed = self.tools.align.markings.dab(
                painting,
                &mesh,
                &MaskEdit {
                    center,
                    radius_mm,
                    erase,
                },
            );
            (layer_id, painting, changed)
        };
        if changed > 0 {
            self.patch_region_preview(layer_id, painting);
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
        if !self.tools.align.brush.is_armed() {
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
        if !resize_align_brush_from_wheel(&mut self.tools.align.brush, ctx) {
            return false;
        }
        self.tools.align.status = Some(self.ui.locale.tr_with(
            "align-brush-size-status",
            &[(
                "size",
                &format!("{:.1}", self.tools.align.brush.radius_mm()),
            )],
        ));
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
        if !self.tools.align.brush.is_armed() {
            return;
        }
        let (Some(camera), Some(pointer)) = (self.render.camera.as_ref(), ctx.pointer_hover_pos())
        else {
            return;
        };
        if !self.pointer_on_bare_viewport(ctx, viewport_rect, pointer) {
            return;
        }
        // The viewport camera is orthographic, so a millimetre maps to a fixed
        // number of pixels regardless of depth.
        let mm_per_pixel =
            crate::align_drag::mm_per_pixel(camera.orthographic_height, viewport_rect.height());
        let radius_px = self.tools.align.brush.radius_mm() / mm_per_pixel;
        if !radius_px.is_finite() || radius_px < 2.0 {
            return;
        }
        // The operator's dental CAD software paints with a green tool and
        // clears with a red one; the ring says which of the two this drag
        // will be, Shift included.
        let shift = ctx.input(|input| input.modifiers.shift);
        let ink = if self.tools.align.brush.erases(shift) {
            egui::Color32::from_rgb(196, 82, 72)
        } else {
            egui::Color32::from_rgb(72, 158, 108)
        };
        let canvas = ui.painter();
        canvas.circle_filled(pointer, radius_px, ink.gamma_multiply(0.10));
        canvas.circle_stroke(pointer, radius_px, egui::Stroke::new(1.2_f32, ink));
        canvas.circle_filled(pointer, 1.5, ink.gamma_multiply(0.7));
    }

    /// Apply one whole-mesh command from the Brush tool window.
    ///
    /// Apply to the mesh selected in the Brush window.
    ///
    /// Exocad's Brush tool has an explicit Mesh selection, and commands such
    /// as Fit everywhere follow that selection. Applying them to both sides
    /// would silently destroy a mask the operator meant to keep.
    pub(super) fn apply_align_mask_command(&mut self, command: MaskCommand) {
        let side = self.tools.align.brush.target_side();
        let Some(layer) = self.side_layer(side) else {
            self.tools.align.status = Some(self.ui.locale.tr("brush-no-mesh"));
            return;
        };
        let taken = {
            let Some(scene) = self.document.scene.clone() else {
                return;
            };
            layer_of(&scene, layer).map(|entry| entry.clone())
        };
        let Some(entry) = taken else {
            self.tools.align.status = Some(self.ui.locale.tr("brush-no-mesh"));
            return;
        };
        if !self.apply_mask_command_to(command, side, &entry) {
            // "Mark automatic" is the only command that can decline, and it
            // declines for one reason the operator can act on.
            if command == MaskCommand::MarkAutomatic {
                self.tools.align.status = Some(self.ui.locale.tr("align-status-place-arrow-first"));
            }
            return;
        }
        self.repaint_region_preview(layer, side);
        self.tools.align.status = Some(self.ui.locale.tr(command.report_key()));
        self.invalidate_deviation_map(&self.ui.locale.tr(command.report_key()));
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
        let positions = self.tools.align.geometry.local_positions(entry);
        let mesh = MarkedMesh {
            positions: &positions,
            pose,
            vertex_count: entry.mesh.vertices().len(),
            geometry: entry.mesh.geometry_id(),
        };
        let keep = AutoKeep {
            centres: &keep,
            radius_mm: f64::from(self.tools.align.brush.auto_radius_mm()),
        };
        self.tools
            .align
            .markings
            .command(side, command, &mesh, &keep)
    }

    /// The world positions of the arrow ends that sit on one side's mesh.
    fn side_arrow_points(&self, side: AlignSide, entry: &SceneMesh) -> Vec<DVec3> {
        self.tools
            .align
            .tool
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
            AlignSide::Moving => self.tools.align.tool.moving_layer(),
            AlignSide::Fixed => self.tools.align.tool.fixed_layer(),
        }
    }

    /// Drop both masks — done whenever the pair changes, since a mask is
    /// indexed by one layer's vertices.
    pub(super) fn clear_align_mask(&mut self) {
        self.tools.align.markings.clear();
    }

    /// Put the markings on both meshes, take them off, or leave them alone.
    pub(super) fn refresh_align_region_preview(&mut self) {
        if !self.tools.align.brush.is_armed() {
            if self.tools.align.overlay == AlignOverlay::Region {
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
        if self.tools.align.overlay == AlignOverlay::Map {
            self.clear_deviation_overlay();
        }
        // A measurement already running would land on top of the markings a
        // moment later. Retiring the generation drops it at the door instead of
        // letting it race the brush.
        if let Some(worker) = self.tools.align.worker.as_ref() {
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
            self.tools.align.status = Some(self.ui.locale.tr("brush-no-mesh"));
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
        // The list belongs to the markings, which produced it. Copied rather
        // than stolen: a `mem::take` here left the markings holding an empty
        // list for the rest of the frame, so anything else that asked what the
        // last dab touched was told "nothing".
        let touched = self.tools.align.markings.touched().to_vec();
        let Some(patched) = self.region_colors_for(layer, side, &touched) else {
            self.repaint_region_preview(layer, side);
            return;
        };
        if !self.patch_overlay_colors(layer, &touched, &patched) {
            self.repaint_region_preview(layer, side);
        }
    }

    /// Compute colours without retaining a scene handle across the in-place edit.
    fn region_colors_for(
        &mut self,
        layer: SceneMeshId,
        side: AlignSide,
        touched: &[u32],
    ) -> Option<Vec<[u8; 4]>> {
        let scene = self.document.scene.clone()?;
        let entry = layer_of(&scene, layer)?;
        let mask = self.tools.align.markings.mask_for(side, marked_on(entry))?;
        let own_colors = entry.mesh.has_vertex_colors();
        let vertices = entry.mesh.vertices();
        Some(
            touched
                .iter()
                .map(|vertex| region_color(vertices, own_colors, Some(&mask), *vertex as usize))
                .collect(),
        )
    }

    /// One colour per vertex of one side: its own colour where nothing is
    /// marked, blue where it is.
    fn region_colors(&self, layer: SceneMeshId, side: AlignSide) -> Option<Vec<[u8; 4]>> {
        let scene = self.document.scene.as_ref()?;
        let entry = layer_of(scene, layer)?;
        let vertices = entry.mesh.vertices();
        let count = vertices.len();
        let mask = self.tools.align.markings.mask_for(side, marked_on(entry));
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
        return crate::align_markings::MARKED_OUT_COLOR;
    }
    match vertices.get(vertex) {
        Some(vertex) if own_colors => vertex.color,
        _ => crate::align_markings::MARKED_IN_COLOR,
    }
}

#[cfg(test)]
mod tests {
    use super::resize_align_brush_from_wheel;
    use crate::align_brush::AlignBrush;
    use eframe::egui;

    fn shift_input(mut events: Vec<egui::Event>) -> egui::RawInput {
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        events.insert(0, egui::Event::ModifiersChanged(shift));
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn consumer_wheel_align_shift_resizes_once_from_horizontal_raw_event() {
        let ctx = egui::Context::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let mut brush = AlignBrush::default();
        brush.set_radius_mm(2.0);
        let mut first_changed = false;
        ctx.run_ui(
            shift_input(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 50.0),
                phase: egui::TouchPhase::Move,
                modifiers: shift,
            }]),
            |ui| first_changed = resize_align_brush_from_wheel(&mut brush, ui.ctx()),
        )
        .drop_without_applying_deltas();

        assert!(first_changed);
        assert!((brush.radius_mm() - 2.25).abs() < f32::EPSILON);

        let mut replayed = true;
        ctx.run_ui(shift_input(Vec::new()), |ui| {
            replayed = resize_align_brush_from_wheel(&mut brush, ui.ctx());
        })
        .drop_without_applying_deltas();

        assert!(
            !replayed,
            "one physical notch must not replay from smoothing"
        );
        assert!((brush.radius_mm() - 2.25).abs() < f32::EPSILON);
    }

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = crate::primary_ui_tests::production_source(include_str!("app_align_brush.rs"));
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
            stroke.contains("self.tools.align.geometry.local_positions(entry)"),
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

    /// The brush must follow the explicit Mesh selection. The two surfaces
    /// overlap, so a nearest-hit picker would intermittently paint the wrong
    /// side; `pick_layer_hit` is the causal guard.
    #[test]
    fn a_dab_is_scoped_to_the_explicit_mesh_selection() {
        let stroke = stroke();
        assert!(
            stroke.contains("let painting = self.tools.align.brush.target_side()")
                && stroke.contains("pick_layer_hit")
                && !stroke.contains("pick_scene_hit"),
            "the stroke must pick only on the explicitly selected mesh"
        );
    }

    /// The operator's dental CAD software's rule: a plain drag marks, Shift
    /// inverses the brush, and the Brush inverse toggle inverses it standing.
    /// Both have to reach the same decision or the toggle and the key would
    /// fight.
    #[test]
    fn a_stroke_takes_its_direction_from_the_toggle_and_shift_together() {
        assert!(
            production().contains(".erases(ctx.input(|input| input.modifiers.shift))"),
            "the stroke direction must come from the brush, Shift included"
        );
    }
}

//! Align Scans: wiring the click model, the worker, and the scene together.
//!
//! Every heavy call goes to [`crate::align_worker`]. This module only routes
//! clicks, hands the worker the geometry it needs, and applies what comes back.

use eframe::egui;
use glam::{DVec3, Vec3};
use occluview_align::Rigid;
use occluview_core::{Scene, SceneMesh, SceneMeshId};

use super::OccluViewApp;
use crate::align_geometry::transform_key;
use crate::align_tool::{AlignPoint, ClickOutcome};
use crate::align_worker::{
    AlignCompletion, AlignJob, AlignJobKind, AlignOutcome, AlignWorker, MeasureKey, WorldPair,
};
use crate::edit_mode::EditModeCommand;
use crate::viewer::pick_scene_hit;

impl OccluViewApp {
    /// One frame of the tool: drain the worker, take the click, paint the
    /// pairs, and run whatever the panel asked for. Returns whether the tool
    /// consumed this frame's viewport input.
    pub(super) fn show_align_tool_overlay_impl(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        suppress_click: bool,
        ctx: &egui::Context,
    ) -> bool {
        self.drain_align_worker(ctx);
        if !self.align.is_armed() {
            return false;
        }
        self.forget_removed_align_layers();

        // Escape leaves the tool, but never steals the key from a dialog.
        let dialogs_open = self.close_guard_open
            || self.app_error.is_some()
            || self.about_window == super::AboutWindowState::Open;
        if !dialogs_open
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            // Escape is a close, and a close puts the scans back. Silently
            // keeping what a cancelled tool did is how work gets lost.
            self.cancel_align_session(ctx);
            return false;
        }

        let hover = ctx.input(|input| input.pointer.hover_pos());
        if let Some((camera, scene)) = self.camera.zip(self.scene.clone()) {
            crate::align_overlay::paint_pairs(
                ui.painter(),
                &crate::align_overlay::PairPaint {
                    camera: &camera,
                    viewport_rect: response.rect,
                    scene: &scene,
                    tool: &self.align,
                    rejected: &self.align_rejected,
                    hover: hover.filter(|pos| response.rect.contains(*pos)),
                },
            );
        }

        self.show_align_panel(ctx, response.rect);
        self.paint_align_brush_cursor(ui, response.rect, ctx);

        if self.handle_align_brush_wheel(response, ctx) {
            return true;
        }
        // Before the brush and the drag: a right-click is an undo whichever of
        // them happens to be live, and neither of them wants the button.
        if self.handle_align_undo_click(response, ctx) {
            return true;
        }
        if self.handle_align_brush(response, ctx) {
            return true;
        }
        if self.handle_align_drag(response, ctx) {
            return true;
        }
        if suppress_click {
            return true;
        }
        self.handle_align_click(response, ctx)
    }

    /// Drop a pair whose layer has left the scene. Half a pair is not a pair,
    /// and a stale layer id would send a fit at whatever inherited it.
    fn forget_removed_align_layers(&mut self) {
        let Some(scene) = self.scene.as_ref() else {
            self.align.clear();
            return;
        };
        let live: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
        let named: Vec<SceneMeshId> = [self.align.moving_layer(), self.align.fixed_layer()]
            .into_iter()
            .flatten()
            .collect();
        for layer in named {
            if !live.contains(&layer) {
                self.align.forget_layer(layer);
                // The mask indexes that layer's vertices. Left behind, it would
                // be handed to the next pair and exclude an arbitrary region of
                // a different scan, with nothing on screen to say so.
                self.clear_align_mask();
            }
        }
    }

    /// Arm the tool, standing every other tool down first.
    ///
    /// Two tools sharing the primary click would fight over every gesture, so
    /// arming one disarms the rest.
    pub(super) fn arm_align_tool(&mut self, ctx: &egui::Context) {
        self.sculpt.disarm();
        self.measure.disarm();
        self.cut_view.disable();
        self.align.arm();
        // Remember where every scan started. Cancel is only honest if there is
        // something to go back to.
        self.align_session_poses = self.scene.as_ref().map_or_else(Vec::new, |scene| {
            scene
                .meshes()
                .iter()
                .map(|entry| (entry.id(), entry.transform))
                .collect()
        });
        if self.align_worker.is_none() {
            self.align_worker = Some(AlignWorker::spawn());
        }
        self.imply_align_pair();
        self.align_status = Some(match self.align.moving_layer() {
            Some(_) => "Two scans in view — click a point on each to pair them".into(),
            None => "Click a point on the scan that should move".into(),
        });
        ctx.request_repaint();
    }

    /// Disarm the tool and drop everything it put on screen.
    pub(super) fn disarm_align_tool(&mut self, ctx: &egui::Context) {
        self.clear_deviation_overlay();
        self.clear_align_mask();
        // Tens of megabytes of cached arrays belong to a session the operator
        // has just left.
        self.align_geometry.clear();
        self.align.disarm();
        if let Some(worker) = self.align_worker.as_ref() {
            worker.bump_generation();
        }
        self.align_status = None;
        self.align_stats = None;
        self.align_rejected.clear();
        self.align_session_poses.clear();
        self.align_brush.set_armed(false);
        ctx.request_repaint();
    }

    /// Whether the tool owns the primary click this frame.
    pub(super) fn align_active(&self) -> bool {
        self.align.is_armed()
    }

    /// Adopt the pair a two-layer scene implies.
    fn imply_align_pair(&mut self) {
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let eligible: Vec<SceneMeshId> = scene
            .meshes()
            .iter()
            .filter(|entry| entry.visible && !entry.mesh.is_point_cloud())
            .map(SceneMesh::id)
            .collect();
        self.align.imply_pair(&eligible);
    }

    /// Route one primary click onto a surface.
    pub(super) fn handle_align_click(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !response.clicked_by(egui::PointerButton::Primary) {
            return false;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return false;
        };
        let Some((camera, scene)) = self.camera.zip(self.scene.clone()) else {
            return false;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            // An off-mesh click still belongs to the armed tool: nothing behind
            // it may act on it.
            return true;
        };
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return true;
        };
        if entry.mesh.is_point_cloud() {
            self.align_status = Some("A point cloud has no surface to pair".into());
            return true;
        }

        let inverse = entry.transform.inverse();
        let point = AlignPoint {
            layer: hit.layer_id,
            local: inverse.transform_point3(hit.point),
            normal: inverse
                .transform_vector3(triangle_normal(entry, hit.triangle_index))
                .normalize_or_zero(),
        };

        self.align_status = Some(match self.align.click(point) {
            ClickOutcome::Ignored => return true,
            ClickOutcome::StartedPair => "Now click the matching spot on the other scan".into(),
            ClickOutcome::CompletedPair(index) => {
                format!("Pair {} placed", index + 1)
            }
            ClickOutcome::MovedPending => "Point moved".into(),
            ClickOutcome::RefusedThirdLayer => {
                "That scan is not in this pair — press Clear to start over".into()
            }
        });
        ctx.request_repaint();
        true
    }

    /// Submit a fit over the clicked pairs.
    pub(super) fn run_align_fit(&mut self) {
        let pairs: Vec<WorldPair> = self.align_world_pairs();
        if pairs.is_empty() {
            return;
        }
        self.submit_align_job(AlignJobKind::Align, pairs);
    }

    /// Submit an ICP refine from the current pose.
    pub(super) fn run_align_refine(&mut self) {
        self.submit_align_job(AlignJobKind::Refine, Vec::new());
    }

    /// Submit a deviation measurement.
    pub(super) fn run_align_measure(&mut self) {
        self.submit_align_job(AlignJobKind::Measure, Vec::new());
    }

    /// The clicked pairs, with the moving half in its layer's local frame and
    /// the fixed half in world — the frames each stage expects.
    fn align_world_pairs(&self) -> Vec<WorldPair> {
        let Some(scene) = self.scene.as_ref() else {
            return Vec::new();
        };
        let Some(fixed_pose) = self
            .align
            .fixed_layer()
            .and_then(|id| layer_of(scene, id))
            .map(|entry| entry.transform)
        else {
            return Vec::new();
        };
        self.align
            .pairs()
            .iter()
            .map(|pair| WorldPair {
                moving: double(pair.moving.local),
                moving_normal: double(pair.moving.normal),
                fixed: double(fixed_pose.transform_point3(pair.fixed.local)),
                fixed_normal: double(
                    fixed_pose
                        .transform_vector3(pair.fixed.normal)
                        .normalize_or_zero(),
                ),
            })
            .collect()
    }

    /// Build and queue one job.
    fn submit_align_job(&mut self, kind: AlignJobKind, pairs: Vec<WorldPair>) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        if self.align_worker.is_none() {
            return;
        }
        let (Some(moving_id), Some(fixed_id)) =
            (self.align.moving_layer(), self.align.fixed_layer())
        else {
            self.align_status = Some("Place a point on each scan first".into());
            return;
        };
        let (Some(moving), Some(fixed)) = (layer_of(&scene, moving_id), layer_of(&scene, fixed_id))
        else {
            return;
        };

        let Some(pose) = Rigid::from_affine(&moving.transform) else {
            self.align_status =
                Some("That scan carries a scaled placement, which cannot be aligned".into());
            return;
        };

        // Geometry, not topology: a sculpt deliberately keeps the topology id
        // and mints a fresh geometry id precisely so geometry-derived caches
        // can tell that the surface changed under them.
        let moving_key = (moving.mesh.geometry_id(), transform_key(moving.transform));
        let fixed_key = (fixed.mesh.geometry_id(), transform_key(fixed.transform));
        // Handed over by `Arc`: the arrays are built once per geometry and pose,
        // not once per submit. Measure is re-submitted on every settings change,
        // and rebuilding them there cost eleven megabytes of copying a time.
        let moving_positions = self.align_geometry.local_positions(moving);
        let moving_indices = self.align_geometry.indices(moving);
        let fixed_world_positions = self.align_geometry.world_positions(fixed);
        let fixed_indices = self.align_geometry.indices(fixed);
        let mask_revision = self.align_mask_revision;
        let mask = self.align_mask.clone();
        let settings = self.align_settings;
        let Some(worker) = self.align_worker.as_ref() else {
            return;
        };
        worker.submit(AlignJob {
            generation: worker.generation(),
            kind,
            moving_positions,
            moving_indices,
            fixed_world_positions,
            fixed_indices,
            fixed_key,
            measure_key: MeasureKey {
                moving: moving_key,
                fixed: fixed_key,
                mask: mask_revision,
                influence_radius_bits: settings.influence_radius_mm.to_bits(),
                orientation: settings.orientation,
            },
            pose,
            pairs,
            mask,
            settings,
        });
        self.align_status = Some(
            match kind {
                AlignJobKind::Align => "Aligning…",
                AlignJobKind::Refine => "Refining…",
                AlignJobKind::Measure => "Measuring…",
            }
            .into(),
        );
    }

    /// Drain finished jobs and apply them.
    pub(super) fn drain_align_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.align_worker.as_ref() else {
            return;
        };
        let completions: Vec<AlignCompletion> = worker.drain();
        if completions.is_empty() {
            return;
        }
        for completion in completions {
            self.apply_align_outcome(completion, ctx);
        }
        ctx.request_repaint();
    }

    /// Apply one finished job.
    fn apply_align_outcome(&mut self, completion: AlignCompletion, ctx: &egui::Context) {
        match completion.outcome {
            AlignOutcome::Aligned {
                pose,
                rms,
                rejected,
            } => {
                self.commit_align_pose(pose);
                self.align_rejected = rejected;
                let dropped = if self.align_rejected.is_empty() {
                    String::new()
                } else {
                    let names: Vec<String> = self
                        .align_rejected
                        .iter()
                        .map(|index| (index + 1).to_string())
                        .collect();
                    format!(", pair {} ignored as an outlier", names.join(" and "))
                };
                // Deliberately no measurement here. The point fit only gets
                // the scan close; measuring it would put a map on screen that
                // the very next step invalidates.
                // The scan just moved, so a map drawn before this describes a
                // pose that no longer exists — and the viewport would happily
                // keep re-pushing it.
                self.invalidate_deviation_map("Aligned on points");
                self.rearm_auto_scale();
                self.align_status = Some(format!(
                    "Aligned — {rms:.3} mm on the points{dropped}. Refine to seat it."
                ));
            }
            AlignOutcome::Refined { pose, report } => {
                self.commit_align_pose(pose);
                self.rearm_auto_scale();
                let weak = weak_axis_note(report.weak_trans_axes, report.weak_rot_axes);
                self.align_status = Some(format!(
                    "Refined — {:.3} mm over {:.0}% of the surface{weak}",
                    report.rms,
                    report.coverage * 100.0
                ));
                self.measure_if_shown();
            }
            AlignOutcome::Measured {
                colors,
                stats,
                seen,
                scale_mm,
            } => {
                self.align_stats = Some(stats);
                // Auto-scale chose the range the colours were painted at. The
                // legend has to adopt it or it would describe a different one.
                if self.align_settings.auto_scale {
                    self.align_settings.scale_mm = scale_mm;
                }
                self.apply_deviation_colors(colors);
                self.align_status = Some(format!(
                    "{:.0}% within {:.2} mm, {} vertices had nothing to measure against{}",
                    stats.within_tolerance * 100.0,
                    self.align_settings.tolerance_mm,
                    stats.skipped,
                    blind_note(seen.as_ref(), stats.rms)
                ));
            }
            AlignOutcome::Failed { message } => {
                self.align_status = Some(message);
            }
        }
        ctx.request_repaint();
    }

    /// Measure again after a pose change, but only if the map is on screen.
    pub(super) fn measure_if_shown(&mut self) {
        // Not while the brush is open. The markings and the map are both
        // per-vertex colours on the same layer, so a measurement landing here
        // would take the operator's own paint off the surface mid-stroke.
        if self.align_brush.is_armed() {
            return;
        }
        if self.align_settings.show_deviation && self.align.can_measure() {
            self.run_align_measure();
        }
    }

    /// Give the display range back to the tool after a pose change.
    ///
    /// A range the operator picked belongs to the alignment they picked it at.
    /// Carried across a fit it becomes a lie in the operator's own hand: the
    /// screenshot behind this was a 0.20 mm range over a pair sitting 1.4 mm
    /// apart, where every vertex is pinned to an end stop and the arch reads as
    /// a red and blue mosaic with no structure in it at all.
    fn rearm_auto_scale(&mut self) {
        self.align_settings.auto_scale = true;
    }

    /// Drop a map that the scan just moved out from under.
    ///
    /// Showing a stale map is worse than showing none: the colours describe a
    /// pose that no longer exists. The operator re-measures when they are ready.
    pub(super) fn invalidate_deviation_map(&mut self, reason: &str) {
        // Only a MAP goes stale. The region tint is the operator's own paint,
        // and dropping it here would erase the brush stroke that called this.
        if self.align_overlay != super::app_align_display::AlignOverlay::Map {
            return;
        }
        self.clear_deviation_overlay();
        self.align_status = Some(format!("{reason} — run Best fit matching to measure again"));
    }

    /// Write a new pose onto the moving layer, as one undo step.
    fn commit_align_pose(&mut self, pose: Rigid) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(moving_id) = self.align.moving_layer() else {
            return;
        };
        let mut next = scene.as_ref().clone();
        if !next.meshes().iter().any(|entry| entry.id() == moving_id) {
            return;
        }
        let Some(token) =
            self.edit_mode
                .begin_scene_edit(&next, moving_id, EditModeCommand::MoveLayer)
        else {
            return;
        };
        if let Some(entry) = next
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == moving_id)
        {
            entry.transform = pose.to_affine();
        }
        self.edit_mode.finish_scene_edit_success(token, &next);
        self.set_scene(next, false);
    }
}

/// Find a layer by identity.
pub(super) fn layer_of(scene: &Scene, id: SceneMeshId) -> Option<&SceneMesh> {
    scene.meshes().iter().find(|entry| entry.id() == id)
}

/// The geometric normal of one triangle, in the layer's local frame.
fn triangle_normal(entry: &SceneMesh, triangle: usize) -> Vec3 {
    let indices = entry.mesh.indices();
    let vertices = entry.mesh.vertices();
    let Some(slice) = indices.get(triangle * 3..triangle * 3 + 3) else {
        return Vec3::Z;
    };
    let corner = |slot: usize| -> Option<Vec3> {
        let index = usize::try_from(slice[slot]).ok()?;
        vertices
            .get(index)
            .map(|vertex| Vec3::from_array(vertex.position))
    };
    let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
        return Vec3::Z;
    };
    let normal = (b - a).cross(c - a);
    if normal.length_squared() > 0.0 {
        normal.normalize()
    } else {
        Vec3::Z
    }
}

/// Promote a stored position to double precision.
fn double(value: Vec3) -> DVec3 {
    DVec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
}

/// Name the directions a refine could not determine, so the panel never shows
/// a confident number for a fit that is free to slide.
/// What the deviation map could not have seen, in a sentence.
///
/// The map measures the distance from each moving vertex to the nearest point
/// on the fixed surface. Slide the two surfaces past each other along a
/// direction the geometry is smooth in and that nearest point slides with them,
/// so a real displacement reads as a fraction of itself — on a full arch, about
/// half. This turns the reported RMS back into the displacement that could be
/// behind it, which is the number a clinician thinks they are already reading.
fn blind_note(seen: Option<&occluview_align::Observability>, rms_mm: f64) -> String {
    /// Below this the correction is not worth a sentence.
    const WORTH_SAYING: f64 = 1.15;

    let Some(seen) = seen else {
        return String::new();
    };
    if seen.has_blind_direction() {
        return " — these surfaces can slide freely, so a displacement of any size \
                could be hiding behind this"
            .into();
    }
    let hidden = seen.hidden_displacement_mm(rms_mm);
    if !hidden.is_finite() || hidden < rms_mm * WORTH_SAYING {
        return String::new();
    }
    format!(" — a rigid mismatch of up to {hidden:.2} mm could read as this")
}

/// Name the directions a refine could not determine, so the panel never shows
/// a confident number for a fit that is free to slide.
fn weak_axis_note(translation: [bool; 3], rotation: [bool; 3]) -> String {
    let names = |flags: [bool; 3]| -> String {
        ["X", "Y", "Z"]
            .into_iter()
            .zip(flags)
            .filter_map(|(name, flagged)| flagged.then_some(name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sliding = names(translation);
    let spinning = names(rotation);
    match (sliding.is_empty(), spinning.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" — the fit can still slide along {sliding}"),
        (true, false) => format!(" — the fit can still turn about {spinning}"),
        (false, false) => {
            format!(" — the fit can still slide along {sliding} and turn about {spinning}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// The whole reason the worker exists. A full arch is hundreds of
    /// thousands of triangles; calling a stage inline would freeze the window
    /// for seconds.
    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = include_str!("app_align.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    #[test]
    fn every_heavy_call_goes_through_the_worker() {
        let source = production();
        for inline in [
            "occluview_align::refine(",
            "occluview_align::deviation(",
            "occluview_align::fit_pairs(",
            "SurfaceIndex::build(",
        ] {
            assert!(
                !source.contains(inline),
                "{inline} must run on the worker, never on the UI thread"
            );
        }
        assert!(
            source.contains("worker.submit(AlignJob {"),
            "the tool must reach the maths by submitting a job"
        );
    }

    /// Two tools sharing the primary click would fight over every gesture.
    #[test]
    fn arming_align_stands_the_other_tools_down() {
        let source = production();
        let arm = source
            .split_once("fn arm_align_tool(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("\n    }"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        for other in [
            "self.sculpt.disarm()",
            "self.measure.disarm()",
            "self.cut_view.disable()",
        ] {
            assert!(arm.contains(other), "arming align must stand down {other}");
        }
    }

    /// A pose change has to reach the same history Ctrl+Z reads, or an
    /// alignment would be unundoable.
    #[test]
    fn a_pose_change_is_recorded_in_the_scene_history() {
        let source = production();
        assert!(source.contains("begin_scene_edit(&next, moving_id, EditModeCommand::MoveLayer)"));
        assert!(source.contains("finish_scene_edit_success(token, &next)"));
    }

    /// Measure is re-submitted on every settings change, so building the
    /// worker's arrays here would copy eleven megabytes of an arch each time
    /// the operator touched a slider. They are cached by the geometry and pose
    /// they were built from and handed over by `Arc`.
    #[test]
    fn a_job_never_re_copies_geometry_that_has_not_changed() {
        let source = production();
        for built_inline in ["flat_map(|vertex| vertex.position)", "indices().to_vec()"] {
            assert!(
                !source.contains(built_inline),
                "{built_inline} must come from the geometry cache, not a fresh copy per submit"
            );
        }
        for cached in [
            "self.align_geometry.local_positions(moving)",
            "self.align_geometry.world_positions(fixed)",
        ] {
            assert!(source.contains(cached), "a job must borrow {cached}");
        }
    }

    /// The reuse contract: a job carries the identity of what it measures, so
    /// the worker can tell a re-colour from a re-measurement. Without it every
    /// nudge of the display scale would re-derive distances that did not
    /// change.
    #[test]
    fn a_job_carries_the_identity_of_what_it_measures() {
        let source = production();
        assert!(source.contains("measure_key: MeasureKey {"));
        for input in [
            "moving: moving_key",
            "fixed: fixed_key",
            "mask: mask_revision",
            "influence_radius_bits",
            "orientation: settings.orientation",
        ] {
            assert!(
                source.contains(input),
                "the measurement key must cover {input}"
            );
        }
        for colour_only in ["scale_mm", "bands", "ramp_mode"] {
            assert!(
                !source.contains(&format!("{colour_only}:")),
                "{colour_only} only changes the colour, so it must not key the measurement"
            );
        }
    }
}

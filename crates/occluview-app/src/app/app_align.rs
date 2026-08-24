//! Align Scans: wiring the click model, the worker, and the scene together.
//!
//! Every heavy call goes to [`crate::align_worker`]. This module only routes
//! clicks and hands the worker the geometry it needs; what comes back is applied
//! in [`super::app_align_results`].

use eframe::egui;
use glam::{DVec3, Vec3};
use occluview_align::Rigid;
use occluview_core::{Scene, SceneMesh, SceneMeshId};

use super::OccluViewApp;
use crate::align_geometry::transform_key;
use crate::align_markings::AlignSide;
use crate::align_tool::{AlignPoint, ClickOutcome};
use crate::align_worker::{AlignJob, AlignJobKind, AlignWorker, MeasureKey, SurfaceKey, WorldPair};
use crate::viewer::pick_scene_hit;

impl OccluViewApp {
    /// One frame of the tool: drain the worker, take the click, paint the
    /// pairs, and run whatever the panel asked for. Returns whether the tool
    /// consumed this frame's viewport input.
    pub(super) fn show_align_tool_overlay(
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
        if !self.modal_dialog_open()
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
                // The rejection list indexes pairs by position. The pairs are
                // gone, so a freshly placed first pair would inherit the red of
                // whatever the last fit rejected, with no fit having run.
                self.align_rejected.clear();
                // Colours and a reading that described a pair this tool no
                // longer has. Left up, they keep rendering under a tool that
                // has forgotten what they were about.
                self.clear_deviation_overlay();
                self.align_stats = None;
                // And whatever the worker is still computing about that pair.
                // Without this the cleanup above was undone a beat later by the
                // abandoned job's own result, which repopulated the status and
                // the statistics for a scan that had left the scene.
                self.abandon_align_jobs();
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
        // A gesture can still be open: Escape is read before the drag handler,
        // and arming another tool disarms this one from the outside. Closing it
        // here records the movement as one undo step. Left dangling, the scan
        // kept a pose that no history step described and no save prompt knew
        // about — and the stale gesture was still live the next time the tool
        // opened.
        self.finish_align_drag();
        self.align_drag = None;
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
        // A session that ended on Manually used to re-open there, with the tab
        // the operator last left rather than the one the tool starts in. The
        // drag constraint is the same class of leak and worse to diagnose: an
        // axis lock set on one case survived into the next pair of scans, where
        // it reads as "the scan is stuck" rather than as a setting that is
        // still on.
        self.align_tab = crate::align_panel::AlignTab::default();
        self.align_constraint = crate::align_drag::DragConstraint::default();
        ctx.request_repaint();
    }

    /// A layer's name, the way the operator named the file.
    ///
    /// Every message about a scan uses this. An operator who is told "moved by
    /// hand" cannot tell which of two arches moved, and in this tool whichever
    /// one they grabbed is the one that moves — so the name is the whole message.
    pub(super) fn layer_display_name(&self, layer: SceneMeshId) -> Option<String> {
        let scene = self.scene.as_ref()?;
        let index = scene
            .meshes()
            .iter()
            .position(|entry| entry.id() == layer)?;
        Some(crate::layers_overlay::layer_label(
            &self.current_paths,
            &scene.meshes()[index],
            index,
        ))
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
        // Arrows belong to the Automatically tab. The drag handler already
        // refuses to run outside Manually; without the mirror of that here, a
        // press too short to become a drag fell through and started a pair on
        // the tab that has no arrows in it — and the panel only drops a
        // half-placed point on the way OUT of Automatically, so that arrow
        // survived every later switch.
        if self.align_tab != crate::align_panel::AlignTab::Automatically {
            return true;
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
        // A hidden scan is still geometry, so every stage below would happily fit
        // against it and measure it, and the panel would report a percentage for a
        // surface nobody can see. Worse for a map: the colours land on an
        // invisible layer while the visible one is faded to sixteen per cent, so
        // the viewport shows a ghost and nothing else.
        if !moving.visible || !fixed.visible {
            let hidden = if moving.visible { fixed_id } else { moving_id };
            let name = self
                .layer_display_name(hidden)
                .unwrap_or_else(|| "One of the scans".to_owned());
            self.align_status = Some(format!("{name} is hidden — show it to align against it"));
            return;
        }

        let Some(pose) = Rigid::from_affine(&moving.transform) else {
            self.align_status =
                Some("That scan carries a scaled placement, which cannot be aligned".into());
            return;
        };

        // Geometry, not topology: a sculpt deliberately keeps the topology id
        // and mints a fresh geometry id precisely so geometry-derived caches
        // can tell that the surface changed under them.
        let mask_revision = self.align_markings.revision();
        let moving_key = (moving.mesh.geometry_id(), transform_key(moving.transform));
        // The markings are part of the fixed surface's identity: masked
        // triangles are left out of the index entirely, so a different set of
        // markings is a different surface and must not reuse the built one.
        let fixed_key = SurfaceKey {
            geometry: fixed.mesh.geometry_id(),
            pose: transform_key(fixed.transform),
            markings: mask_revision,
        };
        // Handed over by `Arc`: the arrays are built once per geometry and pose,
        // not once per submit. Measure is re-submitted on every settings change,
        // and rebuilding them there cost eleven megabytes of copying a time.
        let moving_positions = self.align_geometry.local_positions(moving);
        let moving_indices = self.align_geometry.indices(moving);
        let fixed_world_positions = self.align_geometry.world_positions(fixed);
        let fixed_indices = self.align_geometry.indices(fixed);
        // Filtered by vertex count on the way out. A mask taken on geometry that
        // has since changed under the tool indexes vertices that no longer mean
        // what it thinks, and handing it to a job would exclude an arbitrary
        // region of the current scan with nothing on screen to say so.
        let moving_marked = crate::align_markings::MarkedOn {
            geometry: moving.mesh.geometry_id(),
            vertex_count: moving.mesh.vertices().len(),
        };
        let fixed_marked = crate::align_markings::MarkedOn {
            geometry: fixed.mesh.geometry_id(),
            vertex_count: fixed.mesh.vertices().len(),
        };
        // Marks that no longer describe the scan in front of the operator are
        // dropped, and SAID. They used to be dropped in silence, so a region
        // painted out before a repair or a sculpt quietly re-entered the match and
        // the fit changed for no visible reason.
        let stale = [
            (AlignSide::Moving, moving_marked),
            (AlignSide::Fixed, fixed_marked),
        ]
        .into_iter()
        .any(|(side, mesh)| self.align_markings.stale_for(side, mesh));
        let mask = self
            .align_markings
            .mask_for(AlignSide::Moving, moving_marked);
        let fixed_mask = self.align_markings.mask_for(AlignSide::Fixed, fixed_marked);
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
            fixed_mask,
            settings,
        });
        if stale {
            self.align_status = Some(
                "Markings dropped — the scan's surface changed since they were painted".into(),
            );
            return;
        }
        self.align_status = Some(
            match kind {
                AlignJobKind::Align => "Aligning…",
                AlignJobKind::Refine => "Refining…",
                AlignJobKind::Measure => "Measuring…",
            }
            .into(),
        );
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    /// The whole reason the worker exists. A full arch is hundreds of
    /// thousands of triangles; calling a stage inline would freeze the window
    /// for seconds.
    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = crate::primary_ui_tests::production_source(include_str!("app_align.rs"));
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

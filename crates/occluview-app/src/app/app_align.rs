//! Align Scans: wiring the click model, the worker, and the scene together.
//!
//! Every heavy call goes to [`crate::align_worker`]. This module only routes
//! clicks and hands the worker the geometry it needs; what comes back is applied
//! in [`super::app_align_results`].

use eframe::egui;
use glam::{Affine3A, DVec3, Vec3, Vec3A};
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
        if !self.tools.align.tool.is_armed() {
            return false;
        }
        self.forget_removed_align_layers();

        // Escape leaves the tool, but never steals the key from a dialog.
        if !self.ui.modal_dialog_open()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            // Escape is a close, and a close puts the scans back. Silently
            // keeping what a cancelled tool did is how work gets lost.
            self.cancel_align_session(ctx);
            return false;
        }

        let hover = ctx.input(|input| input.pointer.hover_pos());
        if let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) {
            crate::align_overlay::paint_pairs(
                ui.painter(),
                &crate::align_overlay::PairPaint {
                    camera: &camera,
                    viewport_rect: response.rect,
                    scene: &scene,
                    tool: &self.tools.align.tool,
                    rejected: &self.tools.align.rejected,
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
        let Some(scene) = self.document.scene.as_ref() else {
            self.tools.align.tool.clear();
            return;
        };
        let live: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
        let named: Vec<SceneMeshId> = [
            self.tools.align.tool.moving_layer(),
            self.tools.align.tool.fixed_layer(),
        ]
        .into_iter()
        .flatten()
        .collect();
        for layer in named {
            if !live.contains(&layer) {
                self.tools.align.tool.forget_layer(layer);
                self.tools.align.refined_match_ready = false;
                self.tools.align.settings.show_deviation = false;
                // The mask indexes that layer's vertices. Left behind, it would
                // be handed to the next pair and exclude an arbitrary region of
                // a different scan, with nothing on screen to say so.
                self.clear_align_mask();
                // The rejection list indexes pairs by position. The pairs are
                // gone, so a freshly placed first pair would inherit the red of
                // whatever the last fit rejected, with no fit having run.
                self.tools.align.rejected.clear();
                // Colours and a reading that described a pair this tool no
                // longer has. Left up, they keep rendering under a tool that
                // has forgotten what they were about.
                self.clear_deviation_overlay();
                self.tools.align.stats = None;
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
        self.abort_sculpt_stroke();
        self.tools.sculpt.disarm();
        self.tools.measure.disarm();
        self.tools.cut_view.disable();
        self.tools.align.tool.arm();
        // Remember where every scan started. Cancel is only honest if there is
        // something to go back to.
        self.tools.align.session_poses =
            self.document.scene.as_ref().map_or_else(Vec::new, |scene| {
                scene
                    .meshes()
                    .iter()
                    .map(|entry| (entry.id(), entry.transform))
                    .collect()
            });
        if self.tools.align.worker.is_none() {
            self.tools.align.worker = Some(AlignWorker::spawn());
        }
        self.imply_align_pair();
        self.tools.align.status = Some(match self.tools.align.tool.moving_layer() {
            Some(_) => self.ui.locale.tr("align-status-two-scans"),
            None => self.ui.locale.tr("align-status-click-moving"),
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
        self.reset_align_state_for_scene_clear();
        ctx.request_repaint();
    }

    /// Revoke every alignment claim before the scene it describes disappears.
    ///
    /// This is shared by normal tool teardown and the last-layer scene clear.
    /// The latter has no UI context to pass to `disarm_align_tool`, but it still
    /// must cancel jobs and remove overlays before a new scene can reuse a layer
    /// id.
    pub(super) fn reset_align_state_for_scene_clear(&mut self) {
        self.tools.align.drag = None;
        self.clear_deviation_overlay();
        self.clear_align_mask();
        self.tools.align.refined_match_ready = false;
        // Tens of megabytes of cached arrays belong to a session the operator
        // has just left.
        self.tools.align.geometry.clear();
        self.tools.align.tool.disarm();
        if let Some(worker) = self.tools.align.worker.as_ref() {
            worker.bump_generation();
        }
        self.tools.align.status = None;
        self.tools.align.stats = None;
        self.tools.align.rejected.clear();
        self.tools.align.session_poses.clear();
        self.tools.align.brush.set_armed(false);
        self.tools.align.brush.reset_target_side();
        // A session that ended on Manually used to re-open there, with the tab
        // the operator last left rather than the one the tool starts in. The
        // drag constraint is the same class of leak and worse to diagnose: an
        // axis lock set on one case survived into the next pair of scans, where
        // it reads as "the scan is stuck" rather than as a setting that is
        // still on.
        self.tools.align.tab = crate::align_panel::AlignTab::default();
        self.tools.align.constraint = crate::align_drag::DragConstraint::default();
    }

    /// A layer's name, the way the operator named the file.
    ///
    /// Every message about a scan uses this. An operator who is told "moved by
    /// hand" cannot tell which of two arches moved, and in this tool whichever
    /// one they grabbed is the one that moves — so the name is the whole message.
    pub(super) fn layer_display_name(&self, layer: SceneMeshId) -> Option<String> {
        let scene = self.document.scene.as_ref()?;
        let index = scene
            .meshes()
            .iter()
            .position(|entry| entry.id() == layer)?;
        Some(crate::layers_overlay::layer_label(
            &self.persistence.current_paths,
            &scene.meshes()[index],
            index,
            &self.ui.locale,
        ))
    }

    /// Whether the tool owns the primary click this frame.
    pub(super) fn align_active(&self) -> bool {
        self.tools.align.tool.is_armed()
    }

    /// Adopt the pair a two-layer scene implies.
    fn imply_align_pair(&mut self) {
        let Some(scene) = self.document.scene.as_ref() else {
            return;
        };
        let eligible: Vec<SceneMeshId> = scene
            .meshes()
            .iter()
            .filter(|entry| entry.visible && !entry.mesh.is_point_cloud())
            .map(SceneMesh::id)
            .collect();
        self.tools.align.tool.imply_pair(&eligible);
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
        if self.tools.align.tab != crate::align_panel::AlignTab::Automatically {
            return true;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return false;
        };
        let Some((camera, scene)) = self.render.camera.zip(self.document.scene.clone()) else {
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
            self.tools.align.status = Some(self.ui.locale.tr("align-status-no-surface"));
            return true;
        }

        let inverse = entry.transform.inverse();
        let point = AlignPoint {
            layer: hit.layer_id,
            local: inverse.transform_point3(hit.point),
            // `triangle_normal` is calculated from the mesh's local vertices.
            // Do not apply the layer inverse a second time: on a rotated layer
            // that would put the normal in the wrong frame while the point
            // remains local, poisoning the two-pair frame fit.
            normal: triangle_normal(entry, hit.triangle_index),
        };

        let outcome = self.tools.align.tool.click(point);
        // The first point can contradict the arm-time role guess and swap the
        // two scans. That is the same role change the panel button performs, so
        // it owes the same invalidation: without it the map kept describing the
        // direction the panel no longer showed.
        if self.tools.align.tool.take_role_swap() {
            self.adopt_swapped_roles(self.ui.locale.tr("align-status-turned"));
        }
        self.tools.align.status = Some(match outcome {
            ClickOutcome::Ignored => return true,
            ClickOutcome::StartedPair => self.ui.locale.tr("align-status-now-other"),
            ClickOutcome::CompletedPair(index) => self
                .ui
                .locale
                .tr_with("align-pair-placed", &[("n", &(index + 1).to_string())]),
            ClickOutcome::MovedPending => self.ui.locale.tr("align-status-moved"),
            ClickOutcome::RefusedThirdLayer => self.ui.locale.tr("align-status-wrong-scan"),
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
        if !self.tools.align.refined_match_ready || !self.tools.align.settings.show_deviation {
            return;
        }
        self.submit_align_job(AlignJobKind::Measure, Vec::new());
    }

    /// Keep a queued measurement tied to the visible, currently authorized map.
    fn align_measure_allowed(&self, kind: AlignJobKind) -> bool {
        kind != AlignJobKind::Measure
            || (self.tools.align.refined_match_ready && self.tools.align.settings.show_deviation)
    }

    /// The clicked pairs, with the moving half in its layer's local frame and
    /// the fixed half in world — the frames each stage expects.
    fn align_world_pairs(&self) -> Vec<WorldPair> {
        let Some(scene) = self.document.scene.as_ref() else {
            return Vec::new();
        };
        let Some(fixed_pose) = self
            .tools
            .align
            .tool
            .fixed_layer()
            .and_then(|id| layer_of(scene, id))
            .map(|entry| entry.transform)
        else {
            return Vec::new();
        };
        self.tools
            .align
            .tool
            .pairs()
            .iter()
            .map(|pair| WorldPair {
                moving: double(pair.moving.local),
                moving_normal: double(pair.moving.normal),
                fixed: double(fixed_pose.transform_point3(pair.fixed.local)),
                fixed_normal: double(transform_world_normal(fixed_pose, pair.fixed.normal)),
            })
            .collect()
    }

    /// Build and queue one job.
    // The validation and snapshot assembly form one transaction: splitting it
    // between helpers would make it easier to submit mixed-generation inputs.
    #[expect(clippy::too_many_lines)]
    fn submit_align_job(&mut self, kind: AlignJobKind, pairs: Vec<WorldPair>) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        if !self.align_measure_allowed(kind) {
            return;
        }
        if self.tools.align.worker.is_none() {
            return;
        }
        let (Some(moving_id), Some(fixed_id)) = (
            self.tools.align.tool.moving_layer(),
            self.tools.align.tool.fixed_layer(),
        ) else {
            self.tools.align.status = Some(self.ui.locale.tr("align-status-place-first"));
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
                .unwrap_or_else(|| self.ui.locale.tr("align-status-one-scan"));
            self.tools.align.status = Some(
                self.ui
                    .locale
                    .tr_with("align-status-hidden", &[("name", &name)]),
            );
            return;
        }

        let Some(pose) = Rigid::from_affine(&moving.transform) else {
            self.tools.align.status = Some(self.ui.locale.tr("align-status-scaled"));
            return;
        };

        // Geometry, not topology: a sculpt deliberately keeps the topology id
        // and mints a fresh geometry id precisely so geometry-derived caches
        // can tell that the surface changed under them.
        let markings = &self.tools.align.markings;
        let geometry = &mut self.tools.align.geometry;
        let mask_revision = markings.revision();
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
        let moving_positions = geometry.local_positions(moving);
        let moving_indices = geometry.indices(moving);
        let fixed_world_positions = geometry.world_positions(fixed);
        let fixed_indices = geometry.indices(fixed);
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
        .any(|(side, mesh)| markings.stale_for(side, mesh));
        let mask = markings.mask_for(AlignSide::Moving, moving_marked);
        let fixed_mask = markings.mask_for(AlignSide::Fixed, fixed_marked);
        let settings = self.tools.align.settings;
        let Some(worker) = self.tools.align.worker.as_ref() else {
            return;
        };
        let accepted = worker.submit(AlignJob {
            generation: worker.generation(),
            request_id: 0,
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
        if !accepted {
            self.tools.align.status = Some(self.ui.locale.tr("align-status-worker-unavailable"));
            return;
        }
        if stale {
            self.tools.align.status = Some(self.ui.locale.tr("align-markings-dropped"));
            return;
        }
        self.tools.align.status = Some(match kind {
            AlignJobKind::Align => self.ui.locale.tr("align-job-align"),
            AlignJobKind::Refine => self.ui.locale.tr("align-job-refine"),
            AlignJobKind::Measure => self.ui.locale.tr("align-job-measure"),
        });
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

/// Transform a surface normal through a possibly scaled scene instance. A
/// normal is a covector: direct vector transformation is only correct for a
/// rigid transform, while inverse-transpose preserves perpendicularity under
/// non-uniform scale or shear. A singular transform yields zero and is then
/// refused by the two-point fit instead of inventing a direction.
fn transform_world_normal(transform: Affine3A, local: Vec3) -> Vec3 {
    let world = Vec3::from(transform.matrix3.inverse().transpose() * Vec3A::from(local));
    world.normalize_or_zero()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::transform_world_normal;
    use glam::{Affine3A, Vec3};

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

    #[test]
    fn a_click_that_turns_the_pair_around_invalidates_the_fit() {
        let source = production();
        assert!(
            source.contains("if self.tools.align.tool.take_role_swap() {"),
            "the click path must consume the swap the tool reported"
        );
        assert!(
            source.contains(
                "self.adopt_swapped_roles(self.ui.locale.tr(\"align-status-turned\"))"
            ),
            "a click-driven swap owes the same invalidation as the panel button"
        );
    }

    #[test]
    fn clicked_triangle_normals_stay_in_the_mesh_local_frame() {
        let source = production();
        assert!(
            source.contains("normal: triangle_normal(entry, hit.triangle_index)"),
            "a triangle normal is already local and must not be inverse-transformed twice"
        );
        assert!(
            !source.contains("transform_vector3(triangle_normal(entry, hit.triangle_index))"),
            "pair refinement must not receive a normal in a second inverse-transformed frame"
        );
    }

    #[test]
    fn fixed_pair_normals_use_the_inverse_transpose_for_scaled_instances() {
        let source = production();
        assert!(
            source.contains("transform.matrix3.inverse().transpose()"),
            "fixed surface normals need the inverse-transpose normal matrix"
        );
        assert!(
            !source.contains("fixed_pose.transform_vector3(pair.fixed.normal)"),
            "a non-rigid fixed instance must not transform normals as vectors"
        );

        let transform = Affine3A::from_scale(Vec3::new(2.0, 1.0, 1.0));
        let actual = transform_world_normal(transform, Vec3::new(1.0, 1.0, 0.0));
        let expected = Vec3::new(0.5, 1.0, 0.0).normalize();
        assert!(
            (actual - expected).length() < 1.0e-6,
            "{actual:?} != {expected:?}"
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
            "self.tools.sculpt.disarm()",
            "self.tools.measure.disarm()",
            "self.tools.cut_view.disable()",
        ] {
            assert!(arm.contains(other), "arming align must stand down {other}");
        }
    }

    #[test]
    fn removing_a_named_layer_revokes_refined_authority() {
        let source = production();
        let cleanup = source
            .split_once("fn forget_removed_align_layers(")
            .and_then(|(_, rest)| rest.split_once("    /// Arm the tool"))
            .map(|(body, _)| body)
            .unwrap_or_default();
        assert!(
            cleanup.contains("self.tools.align.refined_match_ready = false")
                && cleanup.contains("self.tools.align.settings.show_deviation = false"),
            "a removed scan must not leave a heatmap authority behind"
        );
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
            "geometry.local_positions(moving)",
            "geometry.world_positions(fixed)",
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

//! Align Scans: wiring the click model, the worker, and the scene together.
//!
//! Every heavy call goes to [`crate::align_worker`]. This module only routes
//! clicks, hands the worker the geometry it needs, and applies what comes back.

use std::sync::Arc;

use eframe::egui;
use glam::{Affine3A, DVec3, Vec3};
use occluview_align::Rigid;
use occluview_core::{Scene, SceneMesh, SceneMeshId, Vertex};
use occluview_render::PreparedSceneTopology;

use super::OccluViewApp;
use crate::align_tool::{AlignPoint, ClickOutcome};
use crate::align_worker::{
    AlignCompletion, AlignJob, AlignJobKind, AlignOutcome, AlignWorker, WorldPair,
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
            self.disarm_align_tool(ctx);
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

        self.show_align_panel(ctx);

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
            }
        }
    }

    /// Draw the panel and run what it asked for.
    fn show_align_panel(&mut self, ctx: &egui::Context) {
        let busy = self.align_worker.as_ref().is_some_and(AlignWorker::is_busy);
        let mut settings = self.align_settings;
        let action = crate::align_panel::show(
            ctx,
            crate::align_panel::AlignPanelView {
                tool: &self.align,
                settings: &mut settings,
                status: self.align_status.as_deref(),
                stats: self.align_stats,
                busy,
            },
        );
        self.align_settings = settings;

        match action {
            Some(crate::align_panel::AlignPanelAction::Align) => self.run_align_fit(),
            Some(crate::align_panel::AlignPanelAction::Refine) => self.run_align_refine(),
            Some(crate::align_panel::AlignPanelAction::Measure) => self.run_align_measure(),
            Some(crate::align_panel::AlignPanelAction::HideMap) => self.clear_deviation_overlay(),
            Some(crate::align_panel::AlignPanelAction::Back) => {
                if self.align.back() {
                    self.align_rejected.clear();
                    self.align_status = Some("Point removed".into());
                }
            }
            Some(crate::align_panel::AlignPanelAction::Clear) => {
                self.align.clear();
                self.align_rejected.clear();
                self.align_status = Some("Click a point on the scan that should move".into());
            }
            Some(crate::align_panel::AlignPanelAction::Close) => self.disarm_align_tool(ctx),
            None => {}
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
        self.align.disarm();
        if let Some(worker) = self.align_worker.as_ref() {
            worker.bump_generation();
        }
        self.align_status = None;
        self.align_stats = None;
        self.align_rejected.clear();
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
        let Some(worker) = self.align_worker.as_ref() else {
            return;
        };
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

        worker.submit(AlignJob {
            generation: worker.generation(),
            kind,
            moving_positions: Arc::new(local_positions(moving)),
            moving_indices: Arc::new(moving.mesh.indices().to_vec()),
            fixed_world_positions: Arc::new(world_positions(fixed)),
            fixed_indices: Arc::new(fixed.mesh.indices().to_vec()),
            fixed_key: (fixed.mesh.topology_id(), transform_key(fixed.transform)),
            pose,
            pairs,
            mask: self.align_mask.clone(),
            settings: self.align_settings,
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
                self.align_status = Some(format!("Aligned — {rms:.3} mm on the points{dropped}"));
                self.measure_if_shown();
            }
            AlignOutcome::Refined { pose, report } => {
                self.commit_align_pose(pose);
                let weak = weak_axis_note(report.weak_trans_axes, report.weak_rot_axes);
                self.align_status = Some(format!(
                    "Refined — {:.3} mm over {:.0}% of the surface{weak}",
                    report.rms,
                    report.coverage * 100.0
                ));
                self.measure_if_shown();
            }
            AlignOutcome::Measured { colors, stats } => {
                self.align_stats = Some(stats);
                self.apply_deviation_colors(colors);
                self.align_status = Some(format!(
                    "{:.0}% within {:.2} mm, {} vertices had nothing to measure against",
                    stats.within_tolerance * 100.0,
                    self.align_settings.tolerance_mm,
                    stats.skipped
                ));
            }
            AlignOutcome::Failed { message } => {
                self.align_status = Some(message);
            }
        }
        ctx.request_repaint();
    }

    /// Measure again after a pose change, but only if the map is on screen.
    fn measure_if_shown(&mut self) {
        if self.align_settings.show_deviation && self.align.can_measure() {
            self.run_align_measure();
        }
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

    /// Attach the measured colours and push them to the GPU.
    fn apply_deviation_colors(&mut self, colors: Vec<[u8; 4]>) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(moving_id) = self.align.moving_layer() else {
            return;
        };
        let shared = Arc::new(colors);
        let mut next = scene.as_ref().clone();
        if let Some(entry) = next
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == moving_id)
        {
            if entry.mesh.vertices().len() != shared.len() {
                return;
            }
            *entry = entry.clone().with_deviation(Some(Arc::clone(&shared)));
        }
        self.align_deviation = Some(shared);
        self.set_scene(next, false);
        self.push_deviation_colors();
    }

    /// Replace the moving layer's uploaded vertex colours with the measured
    /// map. The CPU mesh is never touched, so the scan keeps its own colours
    /// and an export is unaffected by what happens to be on screen.
    pub(super) fn push_deviation_colors(&mut self) {
        let (Some(scene), Some(colors), Some(moving_id)) = (
            self.scene.clone(),
            self.align_deviation.clone(),
            self.align.moving_layer(),
        ) else {
            return;
        };
        let Some(entry) = layer_of(&scene, moving_id) else {
            return;
        };
        if entry.mesh.vertices().len() != colors.len() {
            return;
        }
        let painted: Vec<Vertex> = entry
            .mesh
            .vertices()
            .iter()
            .zip(colors.iter())
            .map(|(vertex, color)| {
                let mut painted = *vertex;
                painted.color = *color;
                painted
            })
            .collect();
        let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
        if let Some(live_viewport) = self.live_viewport.as_ref() {
            if let Ok(viewport) = live_viewport.lock() {
                let _ = viewport.write_scene_vertices(&topology, &painted);
            }
        }
        self.needs_render = true;
    }

    /// Drop the map and restore the scan's own colours.
    pub(super) fn clear_deviation_overlay(&mut self) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        if self.align_deviation.take().is_none() {
            return;
        }
        let mut next = scene.as_ref().clone();
        for entry in next.meshes_mut() {
            if entry.deviation_colors().is_some() {
                *entry = entry.clone().with_deviation(None);
            }
        }
        let restore: Vec<(PreparedSceneTopology, Vec<Vertex>)> = next
            .meshes()
            .iter()
            .map(|entry| {
                (
                    PreparedSceneTopology::from_mesh(&entry.mesh),
                    entry.mesh.vertices().to_vec(),
                )
            })
            .collect();
        self.set_scene(next, false);
        if let Some(live_viewport) = self.live_viewport.as_ref() {
            if let Ok(viewport) = live_viewport.lock() {
                for (topology, vertices) in &restore {
                    let _ = viewport.write_scene_vertices(topology, vertices);
                }
            }
        }
        self.align_stats = None;
        self.needs_render = true;
    }
}

/// Find a layer by identity.
fn layer_of(scene: &Scene, id: SceneMeshId) -> Option<&SceneMesh> {
    scene.meshes().iter().find(|entry| entry.id() == id)
}

/// A layer's vertex positions in its own local frame.
fn local_positions(entry: &SceneMesh) -> Vec<f32> {
    entry
        .mesh
        .vertices()
        .iter()
        .flat_map(|vertex| vertex.position)
        .collect()
}

/// A layer's vertex positions posed into world.
fn world_positions(entry: &SceneMesh) -> Vec<f32> {
    entry
        .mesh
        .vertices()
        .iter()
        .flat_map(|vertex| {
            entry
                .transform
                .transform_point3(Vec3::from_array(vertex.position))
                .to_array()
        })
        .collect()
}

/// A cheap identity for a transform, so a cached surface index is reused only
/// while the fixed layer really has not moved.
fn transform_key(transform: Affine3A) -> u64 {
    let mut key = 0u64;
    for value in transform.to_cols_array() {
        key = key
            .rotate_left(7)
            .wrapping_add(u64::from(value.to_bits()))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    key
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

    /// The map is a display overlay. Writing it into the mesh would corrupt
    /// the scan's own colours and leak into every export.
    #[test]
    fn the_deviation_map_never_touches_the_cpu_mesh() {
        let source = production();
        assert!(
            !source.contains("mesh.vertices_mut"),
            "the map must not be written into mesh data"
        );
        assert!(
            source.contains("write_scene_vertices(&topology, &painted)"),
            "the map reaches the GPU through the vertex upload path"
        );
    }
}

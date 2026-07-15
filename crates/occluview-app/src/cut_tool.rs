//! Viewport-integrated Cut View tool: placement, clipping, and section view.
//!
//! The reusable section panel lives in [`crate::section_view`]. Keeping it out
//! of this type lets another tool own placement while presenting the same slice.

use crate::cut_manipulator::{CutFrameInput, CutManipulator, CutUpdate, DiscPose};
use crate::probe_section::SliceProbe;
use crate::section_view::{SectionView, SectionViewFrame, SectionViewUiOutcome};
use eframe::egui;
use glam::Vec3;
use occluview_core::scene::{SceneSection, SectionPlane};
use occluview_core::{Aabb, SceneMeshId};
use occluview_render::{ClipPlane, CutViewSpec};

const CUT_PREVIEW_RENDER_SIZE_PX: u16 = 512;
const CAP_COLOR: [f32; 4] = [0.776, 0.182, 0.175, 1.0];

#[derive(Default)]
pub(super) struct CutTool {
    manipulator: CutManipulator,
    cached_clip: Option<(Vec3, f32)>,
    section: SectionView,
    probe_linked: bool,
}

pub(super) type CutToolUiOutcome = SectionViewUiOutcome;

impl CutTool {
    pub(super) fn is_active(&self) -> bool {
        self.manipulator.is_active()
    }

    pub(super) fn is_planted(&self) -> bool {
        self.manipulator.is_planted()
    }

    pub(super) fn is_probe_linked(&self) -> bool {
        self.probe_linked
    }

    pub(super) fn pose(&self) -> Option<DiscPose> {
        self.manipulator.pose()
    }

    pub(super) fn slice_visible(&self) -> bool {
        self.section.slice_visible()
    }

    pub(super) fn wants_offscreen_slice(&self) -> bool {
        self.section.wants_offscreen_slice()
    }

    pub(super) fn can_render_bbox(bbox: Aabb) -> bool {
        !bbox.is_empty()
    }

    pub(super) const fn preview_size_px() -> u16 {
        CUT_PREVIEW_RENDER_SIZE_PX
    }

    pub(super) fn take_needs_render(&mut self) -> bool {
        self.section.take_needs_render()
    }

    pub(super) fn mark_dirty(&mut self) {
        self.section.mark_dirty();
    }

    pub(super) fn store_slice(
        &mut self,
        ctx: &egui::Context,
        image: egui::ColorImage,
        cam: crate::cut_ruler::SliceCam,
    ) {
        self.section.store_slice(ctx, image, cam);
    }

    pub(super) fn disable(&mut self) {
        self.manipulator.disarm();
        self.cached_clip = None;
        self.section.reset();
        self.probe_linked = false;
    }

    pub(super) fn enable(&mut self) {
        self.manipulator.arm();
        self.cached_clip = None;
        self.section.reset();
        self.probe_linked = false;
    }

    pub(super) fn plant_from_probe(
        &mut self,
        pose: DiscPose,
        keep_positive: bool,
        seed: SliceProbe,
    ) {
        if !self.manipulator.is_active() {
            self.manipulator.arm();
            self.section.reset();
        }
        self.manipulator.plant_pose(pose, keep_positive);
        self.probe_linked = true;
        self.cached_clip = self.manipulator.clip(Vec3::ZERO);
        self.section.sync(self.section_frame());
        self.section
            .set_measure_mode(crate::cut_ruler::SliceMeasureMode::Thickness);
        self.section
            .set_thickness(seed.entry, seed.exit, seed.thickness_mm);
    }

    pub(super) fn update(&mut self, frame: &CutFrameInput, eye: Vec3) -> CutUpdate {
        let mut out = self.manipulator.update(frame);
        self.cached_clip = self.manipulator.clip(eye);
        out.pose_changed |= self.section.sync(self.section_frame());
        if out.exited {
            self.disable();
        }
        out
    }

    pub(super) fn section_plane(&self) -> Option<SectionPlane> {
        self.section.section_plane()
    }

    pub(super) fn cut_view_focus(&self, bbox: Aabb) -> (Vec3, f32) {
        self.section.focus(bbox)
    }

    pub(super) fn zoom_slice_at_cursor(
        &mut self,
        viewport_rect: egui::Rect,
        pointer: Option<egui::Pos2>,
        notches: f32,
    ) -> bool {
        self.section.zoom_at_cursor(viewport_rect, pointer, notches)
    }

    pub(super) fn viewport_clip_plane(&self, bbox: Aabb) -> ClipPlane {
        if self.probe_linked {
            return ClipPlane::disabled();
        }
        if self.is_active() && Self::can_render_bbox(bbox) {
            self.cached_clip
                .map_or_else(ClipPlane::disabled, |(normal, distance)| {
                    ClipPlane::new(normal.to_array(), distance)
                })
        } else {
            ClipPlane::disabled()
        }
    }

    pub(super) fn cut_view_spec(&self, bbox: Aabb) -> Option<CutViewSpec> {
        if !(self.is_active() && Self::can_render_bbox(bbox)) {
            return None;
        }
        let (normal, distance) = self.cached_clip?;
        Some(CutViewSpec {
            plane: ClipPlane::new(normal.to_array(), distance),
            cap_color: CAP_COLOR,
            show_hollow: true,
        })
    }

    pub(super) fn show_section_panel<F>(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        section: Option<&SceneSection>,
        color_for: F,
    ) -> CutToolUiOutcome
    where
        F: Fn(SceneMeshId) -> egui::Color32,
    {
        self.section.show(ui, viewport_rect, section, color_for)
    }

    fn section_frame(&self) -> Option<SectionViewFrame> {
        self.pose()
            .zip(self.cached_clip)
            .and_then(|(pose, (normal, _))| SectionViewFrame::new(pose, normal))
    }
}

#[cfg(test)]
#[path = "cut_tool_tests.rs"]
mod tests;

//! Reusable passive Section view: framing, ruler, panel controls, and texture.
//!
//! A tool supplies a world-space disc pose and plane normal. This type owns no
//! viewport gesture state, so Cut View and Bridge Split can share the exact
//! section UI without competing for the same pointer interaction.

use crate::cut_manipulator::{pose_moved, DiscPose};
use crate::cut_ruler::{
    CutRuler, SectionDisplay, SectionPanelCommand, SectionRender, SliceBasis, SliceCam,
    SliceMeasureMode,
};
use crate::probe_section::SliceProbe;
use eframe::egui;
use glam::Vec3;
use occluview_core::scene::{SceneSection, SectionPlane};
use occluview_core::{Aabb, Camera, SceneMeshId};

const SLICE_ZOOM_STEP: f32 = 1.15;
const SLICE_ZOOM_MIN: f32 = 0.4;
const SLICE_ZOOM_MAX: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct SliceView {
    zoom: f32,
    pan: Vec3,
}

impl Default for SliceView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SectionPrefs {
    mode: SectionDisplay,
    measure_mode: SliceMeasureMode,
    magnet: bool,
}

impl Default for SectionPrefs {
    fn default() -> Self {
        Self {
            mode: SectionDisplay::Lines,
            measure_mode: SliceMeasureMode::Distance,
            magnet: true,
        }
    }
}

/// A world-space section driven by an external tool. The pose determines panel
/// framing while `normal` determines the oriented clipping and section plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SectionViewFrame {
    pose: DiscPose,
    normal: Vec3,
}

/// The current primary viewport orientation, reduced to the two screen axes
/// needed to orient the existing section panel in the same way as the main view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SectionMainView {
    right: Vec3,
    up: Vec3,
}

impl SectionMainView {
    pub(super) fn from_camera(camera: Camera) -> Self {
        let forward = camera.view_direction().normalize_or_zero();
        let up = camera.view_up().normalize_or_zero();
        let right = forward.cross(up).normalize_or_zero();
        Self { right, up }
    }

    pub(super) fn slice_basis(self, normal: Vec3) -> SliceBasis {
        SliceBasis::from_view_axes(normal, self.right, self.up)
    }
}

impl SectionViewFrame {
    pub(super) fn new(pose: DiscPose, normal: Vec3) -> Option<Self> {
        let normal = normal.normalize_or_zero();
        (normal.length_squared() > f32::EPSILON).then_some(Self { pose, normal })
    }

    pub(super) fn pose(self) -> DiscPose {
        self.pose
    }

    pub(super) fn normal(self) -> Vec3 {
        self.normal
    }

    pub(super) fn section_plane(self) -> Option<SectionPlane> {
        SectionPlane::new(self.normal, self.normal.dot(self.pose.center)).ok()
    }

    fn plane_cam(self) -> SliceCam {
        SliceCam {
            focus: self.normal * self.normal.dot(self.pose.center),
            normal: self.normal,
            half_extent: 1.0,
        }
    }
}

/// One passive section-panel session shared by tools that already own their own
/// placement interaction. It deliberately has no manipulator or viewport clip.
#[derive(Default)]
pub(super) struct SectionView {
    texture: Option<egui::TextureHandle>,
    slice_cam: Option<SliceCam>,
    ruler: CutRuler,
    slice_ready: bool,
    current_frame: Option<SectionViewFrame>,
    rendered_frame: Option<SectionViewFrame>,
    needs_render: bool,
    slice_view: SliceView,
    prefs: SectionPrefs,
    slice_basis: SliceBasis,
}

#[derive(Default)]
pub(super) struct SectionViewUiOutcome {
    pub(super) viewport_needs_render: bool,
    pub(super) consumed_pointer: bool,
    pub(super) thickness_changed: bool,
    pub(super) thickness_probe: Option<SliceProbe>,
    pub(super) command: SectionPanelCommand,
}

impl SectionView {
    pub(super) fn sync(&mut self, frame: Option<SectionViewFrame>) -> bool {
        let changed = frames_moved(self.current_frame, frame)
            || (self.slice_ready && frames_moved(self.rendered_frame, frame));
        self.current_frame = frame;
        if let Some(frame) = frame {
            self.ruler.sync_plane(frame.plane_cam());
        }
        if changed {
            self.slice_ready = false;
            self.needs_render = self.wants_offscreen_slice();
            self.slice_view = SliceView::default();
        }
        changed
    }

    /// Reorient the existing section image to the primary camera. Lines mode
    /// uses the new basis immediately; Mesh mode schedules one matching
    /// offscreen render so the texture and vector overlays cannot diverge.
    pub(super) fn sync_main_view(&mut self, main_view: SectionMainView) -> bool {
        let Some(frame) = self.current_frame else {
            return false;
        };
        let next = main_view.slice_basis(frame.normal());
        if next == self.slice_basis {
            return false;
        }
        self.slice_basis = next;
        if self.slice_ready {
            self.slice_ready = false;
        }
        self.needs_render = self.wants_offscreen_slice();
        true
    }

    pub(super) fn reset(&mut self) {
        self.texture = None;
        self.slice_cam = None;
        self.ruler.clear();
        self.slice_ready = false;
        self.current_frame = None;
        self.rendered_frame = None;
        self.needs_render = false;
        self.slice_view = SliceView::default();
        self.prefs = SectionPrefs::default();
        self.slice_basis = SliceBasis::default();
    }

    pub(super) fn frame(&self) -> Option<SectionViewFrame> {
        self.current_frame
    }

    pub(super) fn section_plane(&self) -> Option<SectionPlane> {
        self.current_frame.and_then(SectionViewFrame::section_plane)
    }

    pub(super) fn slice_visible(&self) -> bool {
        match self.prefs.mode {
            SectionDisplay::Lines => self.current_frame.is_some(),
            SectionDisplay::Mesh => self.slice_ready && self.slice_cam.is_some(),
        }
    }

    pub(super) fn wants_offscreen_slice(&self) -> bool {
        matches!(self.prefs.mode, SectionDisplay::Mesh)
    }

    pub(super) fn take_needs_render(&mut self) -> bool {
        let needs_render = self.needs_render;
        self.needs_render = false;
        needs_render
    }

    pub(super) fn mark_dirty(&mut self) {
        if self.current_frame.is_some() {
            self.slice_ready = false;
            self.needs_render = self.wants_offscreen_slice();
        }
    }

    pub(super) fn store_slice(
        &mut self,
        ctx: &egui::Context,
        image: egui::ColorImage,
        cam: SliceCam,
    ) {
        if let Some(texture) = self.texture.as_mut() {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            self.texture =
                Some(ctx.load_texture("occluview-section", image, egui::TextureOptions::LINEAR));
        }
        self.ruler.sync_plane(cam);
        self.slice_cam = Some(cam);
        self.rendered_frame = self.current_frame;
        self.slice_ready = true;
        self.needs_render = false;
    }

    pub(super) fn focus(&self, bbox: Aabb) -> (Vec3, f32) {
        self.posed_focus().unwrap_or_else(|| {
            let center = bbox.center() + self.slice_view.pan;
            (
                center,
                (bbox.half_diagonal().max(1.0) / self.slice_view.zoom).max(0.05),
            )
        })
    }

    pub(super) fn zoom_at_cursor(
        &mut self,
        viewport_rect: egui::Rect,
        pointer: Option<egui::Pos2>,
        notches: f32,
    ) -> bool {
        if notches == 0.0 {
            return false;
        }
        let (Some(pointer), Some(cam)) = (pointer, self.panel_cam()) else {
            return false;
        };
        let Some(image_rect) = crate::cut_ruler::section_image_rect_for(viewport_rect) else {
            return false;
        };
        if !image_rect.contains(pointer) {
            return false;
        }
        let new_zoom = (self.slice_view.zoom * SLICE_ZOOM_STEP.powf(notches))
            .clamp(SLICE_ZOOM_MIN, SLICE_ZOOM_MAX);
        if (new_zoom - self.slice_view.zoom).abs() <= f32::EPSILON {
            return false;
        }
        let half_ratio = self.slice_view.zoom / new_zoom;
        let (new_focus, _) = crate::cut_ruler::SlicePlaneMap::zoom_focus_at_cursor_with_basis(
            cam.focus,
            cam.half_extent,
            image_rect,
            pointer,
            half_ratio,
            self.slice_basis,
        );
        self.slice_view.pan += new_focus - cam.focus;
        self.slice_view.zoom = new_zoom;
        self.needs_render = self.wants_offscreen_slice();
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn show<F>(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        section: Option<&SceneSection>,
        color_for: F,
    ) -> SectionViewUiOutcome
    where
        F: Fn(SceneMeshId) -> egui::Color32,
    {
        let mut outcome = SectionViewUiOutcome::default();
        if !self.slice_visible() {
            return outcome;
        }
        let Some(cam) = self.panel_cam() else {
            return outcome;
        };
        let previous_thickness = self.ruler.thickness_probe();
        let render = SectionRender {
            mode: self.prefs.mode,
            measure_mode: self.prefs.measure_mode,
            magnet: self.prefs.magnet,
            texture: matches!(self.prefs.mode, SectionDisplay::Mesh)
                .then_some(self.texture.as_ref())
                .flatten(),
            section,
            color_for,
        };
        let out = crate::cut_ruler::show_section_panel_with_basis(
            ui,
            viewport_rect,
            cam,
            self.slice_basis,
            &mut self.ruler,
            render,
        );
        if out.mode != self.prefs.mode {
            self.prefs.mode = out.mode;
            if matches!(out.mode, SectionDisplay::Mesh) {
                self.slice_ready = false;
                self.needs_render = true;
            }
            outcome.viewport_needs_render = true;
        }
        self.prefs.measure_mode = out.measure_mode;
        self.prefs.magnet = out.magnet;
        if out.panned {
            self.slice_view.pan += out.pan_delta;
            self.needs_render = self.wants_offscreen_slice();
            outcome.viewport_needs_render = true;
        }
        outcome.consumed_pointer = out.consumed;
        outcome.command = out.command;
        outcome.thickness_probe = self.ruler.thickness_probe();
        outcome.thickness_changed = outcome.thickness_probe != previous_thickness;
        outcome
    }

    pub(super) fn set_thickness(&mut self, entry: Vec3, exit: Vec3, thickness_mm: f32) {
        if let Some(cam) = self.panel_cam() {
            self.ruler.set_thickness(entry, exit, thickness_mm, cam);
        }
    }

    pub(super) fn set_measure_mode(&mut self, mode: SliceMeasureMode) {
        self.prefs.measure_mode = mode;
    }

    pub(super) fn slice_basis(&self) -> SliceBasis {
        self.slice_basis
    }

    #[cfg(test)]
    pub(super) fn measure_mode(&self) -> SliceMeasureMode {
        self.prefs.measure_mode
    }

    #[cfg(test)]
    pub(super) fn set_display_mode(&mut self, mode: SectionDisplay) {
        if self.prefs.mode != mode {
            self.prefs.mode = mode;
            self.slice_ready = false;
            self.needs_render = matches!(mode, SectionDisplay::Mesh);
        }
    }

    #[cfg(test)]
    pub(super) fn display_mode(&self) -> SectionDisplay {
        self.prefs.mode
    }

    #[cfg(test)]
    pub(super) fn set_magnet(&mut self, magnet: bool) {
        self.prefs.magnet = magnet;
    }

    #[cfg(test)]
    pub(super) fn magnet(&self) -> bool {
        self.prefs.magnet
    }

    #[cfg(test)]
    pub(super) fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture.as_ref().map(egui::TextureHandle::id)
    }

    #[cfg(test)]
    pub(super) fn slice_ready(&self) -> bool {
        self.slice_ready
    }

    #[cfg(test)]
    pub(super) fn needs_render(&self) -> bool {
        self.needs_render
    }

    #[cfg(test)]
    pub(super) fn live_cam(&self) -> Option<SliceCam> {
        self.live_slice_cam()
    }

    #[cfg(test)]
    pub(super) fn ruler(&self) -> &CutRuler {
        &self.ruler
    }

    #[cfg(test)]
    pub(super) fn ruler_mut(&mut self) -> &mut CutRuler {
        &mut self.ruler
    }

    #[cfg(test)]
    pub(super) fn set_pan(&mut self, pan: Vec3) {
        self.slice_view.pan = pan;
    }

    #[cfg(test)]
    pub(super) fn pan(&self) -> Vec3 {
        self.slice_view.pan
    }

    fn panel_cam(&self) -> Option<SliceCam> {
        match self.prefs.mode {
            SectionDisplay::Mesh => self.slice_cam,
            SectionDisplay::Lines => self.live_slice_cam(),
        }
    }

    fn live_slice_cam(&self) -> Option<SliceCam> {
        let frame = self.current_frame?;
        let (focus, half_extent) = self.posed_focus()?;
        Some(SliceCam {
            focus,
            normal: frame.normal(),
            half_extent,
        })
    }

    fn posed_focus(&self) -> Option<(Vec3, f32)> {
        let pose = self.current_frame?.pose();
        let half = (pose.radius_mm * 1.6).max(1.0);
        Some((
            pose.center + self.slice_view.pan,
            (half / self.slice_view.zoom).max(0.05),
        ))
    }
}

fn frames_moved(lhs: Option<SectionViewFrame>, rhs: Option<SectionViewFrame>) -> bool {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => {
            pose_moved(&lhs.pose(), &rhs.pose()) || lhs.normal().dot(rhs.normal()) < 1.0 - 1.0e-5
        }
        (None, None) => false,
        _ => true,
    }
}

#[cfg(test)]
#[path = "section_view_tests.rs"]
mod tests;

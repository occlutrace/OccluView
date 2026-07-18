//! Interactive sculpt-brush state: exocad-style freeforming (Smooth / Add /
//! Remove wax knife) dragged directly on a scan surface inside the Mesh
//! Editor. This module owns the pure state and math — which brush is armed,
//! the live per-drag stroke bookkeeping, strength/scale conversions — while
//! the egui/viewport glue lives in `app::app_sculpt` and the geometry kernel
//! is [`occlu_mesh_edit::BrushSession`].

use glam::Affine3A;
use occluview_core::{BrushMode, BrushSession, SceneMeshId, Vertex};
use occluview_render::PreparedSceneTopology;

/// Brush radius bounds and default, in mm on the model.
pub(crate) const SCULPT_RADIUS_DEFAULT_MM: f32 = 3.0;
pub(crate) const SCULPT_RADIUS_MIN_MM: f32 = 0.5;
pub(crate) const SCULPT_RADIUS_MAX_MM: f32 = 15.0;
/// Strength slider bounds and default, in percent (UI units).
pub(crate) const SCULPT_STRENGTH_DEFAULT_PCT: f32 = 50.0;
pub(crate) const SCULPT_STRENGTH_MIN_PCT: f32 = 5.0;
pub(crate) const SCULPT_STRENGTH_MAX_PCT: f32 = 100.0;
/// Kernel stroke strength accumulated per second at a full slider. Strokes
/// apply once per input frame, so per-frame strength must scale with the
/// frame delta or brush speed would depend on the display refresh rate.
const SCULPT_STRENGTH_RATE_PER_SEC: f32 = 4.0;
/// Longest frame delta one stroke integrates. A hitch (load, window drag)
/// must not land as one giant dab when input resumes.
const SCULPT_MAX_FRAME_DT_SEC: f32 = 0.05;

/// Which sculpt brush the operator armed in the Mesh Editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SculptBrushKind {
    /// Volume-preserving relaxation: irons scanner noise and seams flat.
    Smooth,
    /// Additive wax knife: builds material up along the surface normal.
    Add,
    /// Subtractive wax knife: carves material away along the surface normal.
    Remove,
}

impl SculptBrushKind {
    pub(crate) fn brush_mode(self) -> BrushMode {
        match self {
            Self::Smooth => BrushMode::Smooth,
            Self::Add => BrushMode::Add,
            Self::Remove => BrushMode::Remove,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Add => "Add",
            Self::Remove => "Remove",
        }
    }
}

/// The armed brush plus the live drag, if one is in flight.
#[derive(Default)]
pub(crate) struct SculptTool {
    /// The armed brush; `None` = sculpting off, selection gestures own the
    /// primary button again.
    pub(crate) armed: Option<SculptBrushKind>,
    /// The stroke currently being dragged, present only while the primary
    /// button is held on the surface.
    pub(crate) active: Option<SculptStroke>,
}

impl SculptTool {
    /// Toggle `kind`: arming it takes over from any other brush; clicking the
    /// armed brush again disarms sculpting entirely. Any half-finished stroke
    /// is dropped (the caller reverts its live GPU preview).
    pub(crate) fn toggle(&mut self, kind: SculptBrushKind) {
        self.active = None;
        self.armed = if self.armed == Some(kind) {
            None
        } else {
            Some(kind)
        };
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = None;
        self.active = None;
    }
}

/// Bookkeeping for one live brush drag over a single layer.
pub(crate) struct SculptStroke {
    /// The sculpted layer's stable identity — dabs landing on other layers
    /// mid-drag are ignored, and the commit re-finds the layer by this id.
    pub(crate) layer_id: SceneMeshId,
    /// The kernel session, prepared once per drag over the layer's mesh.
    pub(crate) session: BrushSession,
    /// Display copy of the layer's vertex array, patched per dab from the
    /// session and written straight into the prepared GPU vertex buffer for
    /// live feedback (the scene's own `Mesh` stays untouched until commit).
    pub(crate) shadow: Vec<Vertex>,
    /// GPU topology identity of the mesh being sculpted — routes the live
    /// vertex write to the right prepared-scene entry.
    pub(crate) topology: PreparedSceneTopology,
    /// World → mesh-local transform for stroke centers.
    pub(crate) world_to_local: Affine3A,
    /// Brush radius converted into mesh-local units.
    pub(crate) local_radius_mm: f32,
    /// Whether any dab actually moved vertices — a drag that never touched
    /// the surface must not create an undo entry.
    pub(crate) touched: bool,
}

impl SculptStroke {
    /// Copy the session's live position and normal for every touched vertex
    /// id into the display shadow. Color and UV are preserved untouched, so
    /// textured/colored scans keep their look while being sculpted.
    pub(crate) fn patch_shadow(&mut self, touched: &[usize]) {
        let live = self.session.vertices();
        for &vertex_id in touched {
            if let (Some(target), Some(source)) =
                (self.shadow.get_mut(vertex_id), live.get(vertex_id))
            {
                target.position = source.position;
                target.normal = source.normal;
            }
        }
    }
}

/// Mean scale of a scene transform's linear part — converts the on-model mm
/// brush radius into mesh-local units. Scene placements are rigid in
/// practice (scale 1), so this is a defensive average, never zero.
pub(crate) fn mean_uniform_scale(transform: &Affine3A) -> f32 {
    let m = transform.matrix3;
    let mean = (m.x_axis.length() + m.y_axis.length() + m.z_axis.length()) / 3.0;
    if mean.is_finite() && mean > f32::EPSILON {
        mean
    } else {
        1.0
    }
}

/// Per-frame kernel stroke strength from the UI percent slider and the frame
/// delta, clamped to the kernel's 0..1 stroke range.
pub(crate) fn frame_strength(slider_pct: f32, dt_sec: f32) -> f32 {
    let slider = (slider_pct / 100.0).clamp(0.0, 1.0);
    let dt = dt_sec.clamp(0.0, SCULPT_MAX_FRAME_DT_SEC);
    (slider * dt * SCULPT_STRENGTH_RATE_PER_SEC).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn toggling_a_brush_arms_it_and_toggling_again_disarms() {
        let mut tool = SculptTool::default();
        tool.toggle(SculptBrushKind::Smooth);
        assert_eq!(tool.armed, Some(SculptBrushKind::Smooth));
        tool.toggle(SculptBrushKind::Add);
        assert_eq!(tool.armed, Some(SculptBrushKind::Add));
        tool.toggle(SculptBrushKind::Add);
        assert_eq!(tool.armed, None);
    }

    #[test]
    fn mean_uniform_scale_reads_a_rigid_transform_as_one() {
        let rigid = Affine3A::from_rotation_translation(
            Quat::from_rotation_y(0.7),
            Vec3::new(3.0, -2.0, 9.0),
        );
        assert!((mean_uniform_scale(&rigid) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mean_uniform_scale_survives_a_degenerate_transform() {
        let degenerate = Affine3A::from_scale(Vec3::ZERO);
        assert_eq!(mean_uniform_scale(&degenerate), 1.0);
    }

    #[test]
    fn frame_strength_scales_with_time_and_clamps_hitches() {
        let per_frame = frame_strength(50.0, 1.0 / 60.0);
        assert!(
            per_frame > 0.0 && per_frame < 0.1,
            "gentle per-frame dab: {per_frame}"
        );
        // A 2-second hitch integrates as if it were the clamp, not 2 seconds.
        assert_eq!(frame_strength(100.0, 2.0), frame_strength(100.0, 10.0));
        assert_eq!(frame_strength(0.0, 1.0), 0.0);
    }
}

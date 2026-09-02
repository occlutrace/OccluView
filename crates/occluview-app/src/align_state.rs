//! The align tool's whole state, one struct.
//!
//! `Align Scans` is the largest tool in the app. Its state used to be eighteen
//! loose fields on `OccluViewApp`; they are grouped here so the tool reads as
//! one unit and the app struct carries a single `align: AlignState` field.
//!
//! The struct is a plain field container; the methods that act on it are
//! `impl OccluViewApp` blocks in `app_align*.rs` and `align_*.rs` that access
//! the fields through `self.align.<field>`.

use crate::align_brush::AlignBrush;
use crate::align_drag::DragConstraint;
use crate::align_geometry::{AlignGeometry, PaintedVertices};
use crate::align_markings::AlignMarkings;
use crate::align_panel::AlignTab;
use crate::align_tool::AlignTool;
use crate::align_worker::{AlignSettings, AlignWorker};
use crate::app::app_align_display::AlignOverlay;
use crate::app::app_align_drag::AlignDrag;
use glam::Affine3A;
use occluview_align::DeviationStats;
use occluview_core::SceneMeshId;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct AlignState {
    pub(crate) tool: AlignTool,
    pub(crate) worker: Option<AlignWorker>,
    pub(crate) settings: AlignSettings,
    pub(crate) status: Option<String>,
    pub(crate) stats: Option<DeviationStats>,
    pub(crate) rejected: Vec<u32>,
    /// Per-layer overlay colours currently on screen.
    pub(crate) overlay_colors: Vec<(SceneMeshId, Arc<Vec<[u8; 4]>>)>,
    /// The flat arrays the align worker takes, kept between jobs so a settings
    /// change does not re-copy geometry that has not moved.
    pub(crate) geometry: AlignGeometry,
    /// The vertex buffer a deviation map is uploaded through, repainted across
    /// re-colours instead of rebuilt.
    pub(crate) painted: PaintedVertices,
    /// Set when new colours are attached and the GPU has not seen them yet.
    /// Consumed by the viewport sync, which is the one place that knows whether
    /// there is a prepared scene to write into.
    pub(crate) deviation_push_pending: bool,
    /// What the operator marked out of the match, on both scans. Owns its own
    /// revision and coverage counts, so no caller can change a mask without the
    /// caches downstream hearing about it.
    pub(crate) markings: AlignMarkings,
    pub(crate) drag: Option<AlignDrag>,
    pub(crate) constraint: DragConstraint,
    pub(crate) brush: AlignBrush,
    /// What the per-vertex colours on the moving scan currently mean.
    pub(crate) overlay: AlignOverlay,
    pub(crate) session_poses: Vec<(SceneMeshId, Affine3A)>,
    pub(crate) tab: AlignTab,
    pub(crate) ghosted: Vec<(SceneMeshId, f32)>,
}

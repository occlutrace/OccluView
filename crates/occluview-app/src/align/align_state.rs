//! The align tool's whole state, one struct.
//!
//! `Align Scans` is the largest tool in the app. Its state is grouped here so
//! the tool reads as one unit and the app struct carries a single
//! `align: AlignState` field.
//!
//! The struct is a plain field container; the methods that act on it are
//! `impl OccluViewApp` blocks in `app/align/*.rs` and `align_*.rs` that access
//! the fields through `self.align.<field>`.

use crate::align::align_brush::AlignBrush;
use crate::align::align_drag::DragConstraint;
use crate::align::align_geometry::{AlignGeometry, PaintedVertices};
use crate::align::align_markings::AlignMarkings;
use crate::align::align_panel::AlignTab;
use crate::align::align_tool::AlignTool;
use crate::align::align_worker::{AlignSettings, AlignWorker};
use crate::app::align::display::AlignOverlay;
use crate::app::align::drag::AlignDrag;
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
    /// Candidate review never owns scene pose or measurement authority.
    pub(crate) review: Option<AlignmentReview>,
    /// Only explicit acceptance can authorize a derived measurement.
    pub(crate) accepted: Option<AcceptedAlignment>,
    /// Input authority captured before worker submission.
    pub(crate) pending_review: Option<ReviewKey>,
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
    /// Last primary-pointer location in an open manual drag. egui may deliver
    /// the release in the same frame as the move, and its final pointer state
    /// alone no longer contains the motion path.
    pub(crate) drag_last_pointer_pos: Option<egui::Pos2>,
    /// Modifiers last seen while the primary drag was open. egui exposes the
    /// final modifiers for a whole `RawInput` batch, so event replay needs this
    /// preceding state to classify moves before a mid-frame modifier change.
    pub(crate) drag_modifiers: Option<egui::Modifiers>,
    /// Set only after a pointer segment changes the live pose. A click on the
    /// surface alone must not revoke an otherwise valid alignment result.
    pub(crate) drag_pose_changed: bool,
    pub(crate) constraint: DragConstraint,
    pub(crate) brush: AlignBrush,
    /// What the per-vertex colours on the moving scan currently mean.
    pub(crate) overlay: AlignOverlay,
    pub(crate) session_poses: Vec<(SceneMeshId, Affine3A)>,
    pub(crate) tab: AlignTab,
    /// Layers drawn faded while the map is up. Applied when the frame's
    /// uniforms are built, never written into the scene: a fade stored as the
    /// layer's opacity was captured by every history step and save taken while
    /// the map was up, and came back with no map to justify it.
    pub(crate) ghosted: Vec<SceneMeshId>,
}

/// Immutable authority snapshot; comparisons use complete values, not hashes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReviewKey {
    pub(crate) generation: u64,
    pub(crate) content_revision: u64,
    pub(crate) roles: [SceneMeshId; 2],
    pub(crate) geometry: [u64; 2],
    pub(crate) transforms: [Affine3A; 2],
    pub(crate) visible: [bool; 2],
    pub(crate) mask_revision: u64,
    pub(crate) matching: [u64; 2],
    pub(crate) orientation: occluview_align::Orientation,
}

pub(crate) struct AlignmentReview {
    pub(crate) key: ReviewKey,
    pub(crate) candidates: occluview_align::AlignmentSearchResult,
    pub(crate) selected: usize,
    pub(crate) preview_enabled: bool,
}

/// Operator authorization and geometric confidence remain separate facts.
pub(crate) struct AcceptedAlignment {
    pub(crate) key: Option<ReviewKey>,
    pub(crate) candidate_id: occluview_align::CandidateId,
    pub(crate) confidence: occluview_align::Confidence,
    pub(crate) evidence: occluview_align::CandidateEvidence,
}
#[cfg(test)]
impl AcceptedAlignment {
    /// Existing map tests isolate map invalidation from candidate search.
    pub(crate) fn test_authority() -> Self {
        Self {
            key: None,
            candidate_id: occluview_align::CandidateId {
                family: 0,
                proposal: 0,
            },
            confidence: occluview_align::Confidence::Weak,
            evidence: occluview_align::CandidateEvidence::default(),
        }
    }
}

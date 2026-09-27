//! Topology-edit policy: the density laws a remesh obeys, its budgets, and the
//! revision fence that tells a consumer the topology moved.
//!
//! Every number here is a law about edges and the surface under a brush —
//! target spacing, split and collapse hysteresis, how many operations one dab
//! may spend, how much journal it may write — and none of it knows what the
//! surface is for.
//!
//! `TopologyRevision` lives here for the same reason: it is a fact about the
//! mesh, and any consumer of a session has to be able to say that the topology
//! changed without inventing its own counter.

/// Fenced topology revision: one increment per topology-changing dab.
/// Display, history, and Apply gate on exact equality with the base
/// revision; a mismatch is a typed retryable fault, never a partial apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TopologyRevision(pub u32);

impl TopologyRevision {
    /// The next revision, wrapping rather than panicking at the limit.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// One versioned policy for the whole dab: a single target edge length with
/// hysteresis, plus latency guards. These numbers bound work per dab; they
/// are not visual-quality metrics and never override the operator picture.
#[derive(Clone, Copy, Debug)]
pub struct RemeshPolicy {
    /// Target edge length as a fraction of dab radius.
    pub target_fraction_of_radius: f64,
    /// Split above this multiple of the target edge length.
    pub split_hysteresis: f64,
    /// Collapse below this multiple of the same target. A candidate also
    /// has to keep every surviving edge below the split threshold.
    pub collapse_hysteresis: f64,
    /// Candidate edges scanned per dab.
    pub max_candidates_per_dab: usize,
    /// Committed operations per dab; exhaustion skips topology work while
    /// ordinary Smooth continues.
    pub max_operations_per_dab: usize,
    /// Longest-edge split sweeps per densify call. Each sweep halves every
    /// over-long edge still in the footprint.
    pub max_densify_sweeps: usize,
    /// Journal bytes admitted per dab.
    pub max_journal_bytes_per_dab: usize,
    /// Journal bytes admitted per stroke; the stroke keeps its Smooth
    /// deformation while topology work stands down past this.
    pub max_journal_bytes_per_stroke: usize,
    /// Net live vertex groups one stroke and the session may add. A merge
    /// retires a group, so a remesh that splits and merges in turn spends
    /// none of this; only surface that stays denser does.
    pub max_added_groups_per_stroke: usize,
    /// Net live groups one session may add.
    pub max_added_groups_per_session: usize,
    /// Optional cap on the target edge length, millimetres. A large Smooth
    /// brush would otherwise coarsen (radius/6 of 8 mm is 1.3 mm); the cap
    /// bounds live remesh spacing. None means uncapped.
    pub max_target_mm: Option<f64>,
}

impl RemeshPolicy {
    /// The standard live-remesh policy.
    pub fn standard() -> Self {
        Self {
            target_fraction_of_radius: 1.0 / 6.0,
            split_hysteresis: 4.0 / 3.0,
            collapse_hysteresis: 4.0 / 5.0,
            max_candidates_per_dab: 512,
            max_operations_per_dab: 48,
            max_densify_sweeps: 3,
            max_journal_bytes_per_dab: 256 * 1024,
            // A long pass over a dense seam spends thousands of merges, and
            // the live remesh stands down once this is reached. The browser
            // keeps 64 MB of sculpt undo, so one record may take half of it.
            max_journal_bytes_per_stroke: 32 * 1024 * 1024,
            max_added_groups_per_stroke: 50_000,
            max_added_groups_per_session: 100_000,
            max_target_mm: None,
        }
    }

    /// Target edge length for a dab radius; `None` rejects non-finite or
    /// non-positive radii before any candidate work starts.
    ///
    /// This is the radius-derived HALF of the target. The session caps it by
    /// the area-weighted spacing of the immutable opening surface (see
    /// `SculptSession::target_mm`).
    pub fn target_for_radius(&self, radius: f64) -> Option<f64> {
        if radius.is_finite() && radius > 0.0 {
            let target = radius * self.target_fraction_of_radius;
            Some(match self.max_target_mm {
                Some(cap) if cap.is_finite() && cap > 0.0 => target.min(cap),
                _ => target,
            })
        } else {
            None
        }
    }

    /// Whether an edge of this length is over the split hysteresis.
    pub fn should_split(&self, length: f64, target: f64) -> bool {
        length.is_finite() && target > 0.0 && length > target * self.split_hysteresis
    }

    /// The lower edge of the shared split/collapse hysteresis band.
    pub fn collapse_threshold(&self, target: f64) -> f64 {
        target * self.collapse_hysteresis
    }

    /// Groups still admitted. Every count is of live groups: the slots a
    /// merge retired are not surface. Counting slots made each split-merge
    /// cycle permanent, so a session that kept remeshing one seam ran out of
    /// splits and the live remesh went quiet for good.
    pub fn remaining_group_growth(
        &self,
        groups: u32,
        stroke_base: u32,
        session_base: u32,
    ) -> usize {
        self.max_added_groups_per_stroke
            .saturating_sub(groups.saturating_sub(stroke_base) as usize)
            .min(
                self.max_added_groups_per_session
                    .saturating_sub(groups.saturating_sub(session_base) as usize),
            )
    }
}

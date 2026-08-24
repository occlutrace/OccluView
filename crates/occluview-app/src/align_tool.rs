//! Align Scans point-pair selection.
//!
//! The first point selects the moving scan and the next point on another layer
//! selects the fixed scan. Two visible layers are preselected provisionally;
//! the first placed point overrides scene order.
//!
//! Points are stored in their layer's **local** coordinates. A marker's world
//! position is `pose x local`, recomputed every frame, so markers stay welded
//! to the surface after a fit moves the scan underneath them.

use glam::Vec3;
use occluview_core::SceneMeshId;

/// Pairs needed before a fit is attempted. Two pairs are solved through the
/// clicked surface normals; two bare points cannot determine a rotation.
pub(crate) const MIN_PAIRS_TO_ALIGN: usize = 2;

/// One clicked surface point, in its own layer's local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignPoint {
    /// The layer this point sits on.
    pub(crate) layer: SceneMeshId,
    /// Position in that layer's local coordinates.
    pub(crate) local: Vec3,
    /// Surface normal at the click, in that layer's local coordinates.
    pub(crate) normal: Vec3,
}

/// One correspondence: a point on the moving scan and its partner on the fixed
/// one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AlignPair {
    /// The point on the moving scan.
    pub(crate) moving: AlignPoint,
    /// The point on the fixed scan.
    pub(crate) fixed: AlignPoint,
}

/// What a click did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClickOutcome {
    /// The tool is not armed; nothing happened.
    Ignored,
    /// The first half of a new pair landed.
    StartedPair,
    /// A pair completed; carries its index.
    CompletedPair(usize),
    /// A click on the layer already holding the pending point moved that
    /// point instead of building a malformed pair.
    MovedPending,
    /// The click landed on a layer that is not in the pair.
    RefusedThirdLayer,
}

/// The tool's whole state.
#[derive(Clone, Debug, Default)]
pub(crate) struct AlignTool {
    armed: bool,
    moving: Option<SceneMeshId>,
    fixed: Option<SceneMeshId>,
    pairs: Vec<AlignPair>,
    pending: Option<AlignPoint>,
    /// Whether the roles are still the arm-time guess rather than the
    /// operator's. A guess is overridden by the first placed point; a choice
    /// never is.
    implied: bool,
}

impl AlignTool {
    /// Arm the tool. Clicks start landing on surfaces.
    pub(crate) fn arm(&mut self) {
        self.armed = true;
    }

    /// Disarm and drop everything the session collected.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
        self.clear();
    }

    /// Whether clicks are being interpreted as point placements.
    pub(crate) fn is_armed(&self) -> bool {
        self.armed
    }

    /// The scan that will move.
    pub(crate) fn moving_layer(&self) -> Option<SceneMeshId> {
        self.moving
    }

    /// The scan that stays put.
    pub(crate) fn fixed_layer(&self) -> Option<SceneMeshId> {
        self.fixed
    }

    /// Completed pairs, in the order they were placed.
    pub(crate) fn pairs(&self) -> &[AlignPair] {
        &self.pairs
    }

    /// The half-placed point waiting for its partner.
    pub(crate) fn pending(&self) -> Option<AlignPoint> {
        self.pending
    }

    /// Whether a fit can be attempted.
    pub(crate) fn can_align(&self) -> bool {
        self.pairs.len() >= MIN_PAIRS_TO_ALIGN
    }

    /// Whether a deviation map can be measured.
    ///
    /// One point on each layer is enough: naming the two surfaces is all a
    /// measurement needs, and this is the "just compare two files" path that
    /// requires no alignment at all.
    pub(crate) fn can_measure(&self) -> bool {
        self.moving.is_some() && self.fixed.is_some()
    }

    /// Adopt the pair implied by a scene that holds exactly two eligible
    /// layers. Does nothing once any layer has been named.
    ///
    /// Scene order is the only thing there is to go on here, and scene order is
    /// the order the files were opened in — which the operator has no reason to
    /// remember and no way to see. So this is recorded as a guess, and
    /// [`Self::click`] overrides it.
    pub(crate) fn imply_pair(&mut self, eligible: &[SceneMeshId]) {
        if !self.armed || self.moving.is_some() || eligible.len() != 2 {
            return;
        }
        self.moving = Some(eligible[0]);
        self.fixed = Some(eligible[1]);
        self.implied = true;
    }

    /// Whether the roles are still the arm-time guess. The panel says so, and
    /// offers the swap, only while this holds.
    pub(crate) fn roles_are_implied(&self) -> bool {
        self.implied
    }

    /// Trade the two roles, keeping every point where the operator put it.
    ///
    /// Each pair's halves swap with them, because a pair stores "the moving
    /// point and its partner" — leaving the halves alone would fit the scans
    /// with every correspondence reversed. Marks the roles as chosen, so no
    /// later click undoes the swap.
    pub(crate) fn swap_roles(&mut self) -> bool {
        if self.moving.is_none() || self.fixed.is_none() {
            return false;
        }
        std::mem::swap(&mut self.moving, &mut self.fixed);
        for pair in &mut self.pairs {
            std::mem::swap(&mut pair.moving, &mut pair.fixed);
        }
        self.implied = false;
        true
    }

    /// Place a clicked surface point.
    pub(crate) fn click(&mut self, point: AlignPoint) -> ClickOutcome {
        if !self.armed {
            return ClickOutcome::Ignored;
        }
        let layer = point.layer;

        // The operator has now said which scan they mean, so the arm-time guess
        // gives way: whichever layer they touch first is the one that moves.
        if self.implied {
            self.implied = false;
            if self.fixed == Some(layer) {
                self.swap_roles();
            }
        }

        // Nothing named yet: this click names the moving scan.
        let Some(moving) = self.moving else {
            self.moving = Some(layer);
            self.pending = Some(point);
            return ClickOutcome::StartedPair;
        };

        // Correcting the half-placed point rather than building a bad pair.
        if self.pending.is_some_and(|pending| pending.layer == layer) {
            self.pending = Some(point);
            return ClickOutcome::MovedPending;
        }

        // The second surface names the fixed scan.
        let Some(fixed) = self.fixed else {
            if layer == moving {
                self.pending = Some(point);
                return ClickOutcome::StartedPair;
            }
            self.fixed = Some(layer);
            return self.close_pair(point);
        };

        if layer != moving && layer != fixed {
            return ClickOutcome::RefusedThirdLayer;
        }
        match self.pending {
            None => {
                self.pending = Some(point);
                ClickOutcome::StartedPair
            }
            Some(_) => self.close_pair(point),
        }
    }

    /// Combine the pending point with `point` into a pair, oriented so the
    /// moving half always belongs to the moving layer.
    fn close_pair(&mut self, point: AlignPoint) -> ClickOutcome {
        let Some(pending) = self.pending.take() else {
            self.pending = Some(point);
            return ClickOutcome::StartedPair;
        };
        let (moving, fixed) = if Some(pending.layer) == self.moving {
            (pending, point)
        } else {
            (point, pending)
        };
        self.pairs.push(AlignPair { moving, fixed });
        ClickOutcome::CompletedPair(self.pairs.len() - 1)
    }

    /// Undo one click: the pending point first, then the last whole pair.
    /// Never clears the pair identity — Back walks the points, Clear resets.
    pub(crate) fn back(&mut self) -> bool {
        if self.pending.take().is_some() {
            return true;
        }
        self.pairs.pop().is_some()
    }

    /// Drop the arrows but keep the pair.
    ///
    /// The Manually tab has nothing to do with arrows, and an operator who nudges
    /// a scan by hand has moved it out from under every point they placed — the
    /// arrows then draw a fit that no longer describes anything. Coming back to
    /// the Automatically tab with a clean slate is what the operator asked for by
    /// name. The two scan names stay, because they are what "compare these two"
    /// needs and the operator did not un-choose them.
    ///
    /// Returns whether there was anything to drop.
    pub(crate) fn clear_points(&mut self) -> bool {
        if self.pairs.is_empty() && self.pending.is_none() {
            return false;
        }
        self.pairs.clear();
        self.pending = None;
        true
    }

    /// Drop every point and both layer names, leaving the tool armed so the
    /// next click can start a fresh pair — which is how a third scan gets
    /// aligned to the second.
    pub(crate) fn clear(&mut self) {
        self.moving = None;
        self.fixed = None;
        self.pairs.clear();
        self.pending = None;
        self.implied = false;
    }

    /// Forget a layer that has left the scene. A pair naming a removed layer
    /// is not a pair, so this resets rather than leaving a half-valid one.
    pub(crate) fn forget_layer(&mut self, layer: SceneMeshId) {
        if self.moving == Some(layer) || self.fixed == Some(layer) {
            self.clear();
        }
    }
}

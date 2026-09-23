//! What the operator has marked out of the match, on **both** scans at once.
//!
//! Dental CAD software lets the operator mark either mesh independently, so
//! this owns two masks rather than one. The type exists because the pieces
//! below have to move together and used to sit as five loose fields on the
//! application struct: the two masks, the revision every cache keys on, and
//! the scratch list of vertices the last dab touched.
//! Changing one without the others is how markings ended up on one surface and
//! not the other, and how a stale revision handed a cache the wrong answer.
//!
//! It also keeps a running count of what is marked. Walking a full arch's mask
//! to answer "how much is marked?" is a two-million-byte scan, and the panel
//! asks every frame the Brush window is open.

use std::sync::Arc;

use glam::DVec3;
use occluview_align::{apply_brush, invert, set_all, MaskEdit, Rigid, INCLUDED};

/// The colour marked-out surface is painted.
///
/// Blue, because that is the colour dental CAD software paints an excluded
/// region, and an operator who works in that dialog should not have to learn
/// a second convention here. Defined next to the markings themselves because
/// both the surface and the sentence in the Brush window use it — they were
/// two separate literals in two files, each with a comment claiming they
/// matched. Opaque: a marked-out vertex is fully painted.
pub(crate) const MARKED_OUT_COLOR: [u8; 4] = [58, 108, 196, 255];

/// Which scan of the pair a marking belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignSide {
    /// The scan being placed.
    #[default]
    Moving,
    /// The scan that stays put.
    Fixed,
}

impl AlignSide {
    /// Both sides, for the commands that mean "the mesh" rather than "this one".
    pub(crate) const BOTH: [Self; 2] = [Self::Moving, Self::Fixed];
}

/// What a whole-mesh command left behind, for the status line.
///
/// `marked` is the vertex count the mask reports as fitting, so `marked == 0` on
/// a non-empty mesh means the command excluded EVERYTHING — the state
/// `MaskCommand::MarkAutomatic` reaches when the brush covered the whole layer,
/// and the one the report has to name honestly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaskCommandOutcome {
    /// Vertices the command left as fitting.
    pub(crate) marked: usize,
    /// Vertices the mesh has, i.e. the most the command could mark.
    pub(crate) vertex_count: usize,
}

/// One whole-mesh command from the Brush tool window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaskCommand {
    /// Clear every marking — the whole scan takes part in the match.
    FitEverywhere,
    /// Mark the whole scan, so best-fit matching has no effect.
    FitNowhere,
    /// Swap marked for unmarked.
    InvertMarkings,
    /// Keep only a disc of surface at each arrow end as the matching region.
    MarkAutomatic,
}

impl MaskCommand {
    /// Every command, in the order the Brush tool window lists them.
    pub(crate) const ALL: [Self; 4] = [
        Self::FitEverywhere,
        Self::FitNowhere,
        Self::InvertMarkings,
        Self::MarkAutomatic,
    ];

    /// Catalog key for the button label.
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::FitEverywhere => "align-mask-fit-everywhere",
            Self::FitNowhere => "align-mask-fit-nowhere",
            Self::InvertMarkings => "align-mask-invert",
            Self::MarkAutomatic => "align-mask-automatic",
        }
    }

    /// Catalog key for the one-line hint.
    pub(crate) fn hint_key(self) -> &'static str {
        match self {
            Self::FitEverywhere => "align-mask-fit-everywhere-hint",
            Self::FitNowhere => "align-mask-fit-nowhere-hint",
            Self::InvertMarkings => "align-mask-invert-hint",
            Self::MarkAutomatic => "align-mask-automatic-hint",
        }
    }

    /// Catalog key for the after-report status line.
    pub(crate) fn report_key(self) -> &'static str {
        match self {
            Self::FitEverywhere => "align-mask-fit-everywhere-report",
            Self::FitNowhere => "align-mask-fit-nowhere-report",
            Self::InvertMarkings => "align-mask-invert-report",
            Self::MarkAutomatic => "align-mask-automatic-report",
        }
    }

    /// Catalog key for the report when the command reached one named scan.
    ///
    /// The Mesh selection can narrow a command to one surface, and a report
    /// that said "whole mesh marked" would then be read as both arches when
    /// only one was touched.
    pub(crate) fn report_one_key(self) -> &'static str {
        match self {
            Self::FitEverywhere => "align-mask-fit-everywhere-report-one",
            Self::FitNowhere => "align-mask-fit-nowhere-report-one",
            Self::InvertMarkings => "align-mask-invert-report-one",
            Self::MarkAutomatic => "align-mask-automatic-report-one",
        }
    }
}

/// Which mesh a mask was painted on.
///
/// A vertex count on its own is not an identity. Two arches can carry the same
/// count, and a repair or a sculpt can replace a mesh under the tool while
/// keeping it — so a mask checked only by length could pass, and then excluded an
/// arbitrary region of a surface nobody had marked, with nothing on screen saying
/// so. The geometry id changes whenever the vertices do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MarkedOn {
    /// The mesh the marks were painted on.
    pub(crate) geometry: u64,
    /// How many vertices it had.
    pub(crate) vertex_count: usize,
}

/// One scan's markings, and how much of it they cover.
#[derive(Clone, Debug, Default)]
struct SideMarkings {
    /// One byte per vertex, or nothing if this side was never marked.
    mask: Option<Arc<Vec<u8>>>,
    /// The mesh those bytes describe.
    painted_on: Option<MarkedOn>,
    /// How many of those bytes are `EXCLUDED`. Kept in step with `mask` by
    /// every method below, so the panel never has to count.
    marked: usize,
    /// The vertices the last edit actually changed on this side.
    ///
    /// Per side, not per pair: with the Brush window's default Both target one
    /// stroke dabs both scans, and a single shared list would be overwritten by
    /// the second dab — the first scan's changed vertices would then never be
    /// re-coloured, so half the stroke would be invisible.
    touched: Vec<u32>,
}

impl SideMarkings {
    /// The mask, but only if it still describes this exact mesh. A mask left over
    /// from other geometry is not a reading about this one.
    fn fitting(&self, mesh: MarkedOn) -> Option<&Arc<Vec<u8>>> {
        if self.painted_on != Some(mesh) {
            return None;
        }
        self.mask
            .as_ref()
            .filter(|mask| mask.len() == mesh.vertex_count)
    }

    /// Whether marks exist that no longer describe this mesh.
    fn stale_for(&self, mesh: MarkedOn) -> bool {
        self.mask.is_some() && self.fitting(mesh).is_none()
    }

    /// Take the mask out for editing, or make a fresh unmarked one.
    fn take_for_edit(&mut self, mesh: MarkedOn) -> Arc<Vec<u8>> {
        let existing = self.mask.take().filter(|_| self.painted_on == Some(mesh));
        self.painted_on = Some(mesh);
        match existing {
            Some(existing) if existing.len() == mesh.vertex_count => existing,
            _ => {
                self.marked = 0;
                Arc::new(vec![INCLUDED; mesh.vertex_count])
            }
        }
    }
}

/// The markings on both scans of one alignment.
#[derive(Clone, Debug, Default)]
pub(crate) struct AlignMarkings {
    /// Markings on the scan being placed.
    moving: SideMarkings,
    /// Markings on the scan that stays put.
    fixed: SideMarkings,
    /// Bumped on every change. Caches downstream key on this rather than on the
    /// mask contents, which would mean hashing an arch every frame.
    revision: u64,
    /// Whether the pointer is mid-stroke. A stroke defers the measurement until
    /// the operator lifts the button.
    stroke_open: bool,
}

impl AlignMarkings {
    /// One side's state.
    fn side(&self, side: AlignSide) -> &SideMarkings {
        match side {
            AlignSide::Moving => &self.moving,
            AlignSide::Fixed => &self.fixed,
        }
    }

    /// One side's state, for editing.
    fn side_mut(&mut self, side: AlignSide) -> &mut SideMarkings {
        match side {
            AlignSide::Moving => &mut self.moving,
            AlignSide::Fixed => &mut self.fixed,
        }
    }

    /// The mask to hand a job for this side, if it matches the mesh.
    pub(crate) fn mask_for(&self, side: AlignSide, mesh: MarkedOn) -> Option<Arc<Vec<u8>>> {
        self.side(side).fitting(mesh).map(Arc::clone)
    }

    /// Whether this side carries marks that no longer describe the mesh in front
    /// of the operator. The panel says so rather than letting them wonder why
    /// their excluded region stopped taking effect.
    pub(crate) fn stale_for(&self, side: AlignSide, mesh: MarkedOn) -> bool {
        self.side(side).stale_for(mesh)
    }

    /// The generation every downstream cache keys on.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The vertices the last dab changed on this side.
    pub(crate) fn touched(&self, side: AlignSide) -> &[u32] {
        &self.side(side).touched
    }

    /// The pointer came up. Returns whether a stroke was actually open, which
    /// is the caller's cue to re-measure.
    pub(crate) fn close_stroke(&mut self) -> bool {
        std::mem::take(&mut self.stroke_open)
    }

    /// What share of the two scans is marked, or nothing if neither carries a
    /// mask that fits its mesh. Free to call: the counts are maintained here.
    #[allow(dead_code)]
    pub(crate) fn marked_fraction(&self, moving: MarkedOn, fixed: MarkedOn) -> Option<f32> {
        let mut marked = 0usize;
        let mut total = 0usize;
        for (side, mesh) in [(AlignSide::Moving, moving), (AlignSide::Fixed, fixed)] {
            let state = self.side(side);
            if state.fitting(mesh).is_some() {
                marked += state.marked;
                total += mesh.vertex_count;
            }
        }
        if total == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        Some(marked as f32 / total as f32)
    }

    /// Whether anything at all is marked on either scan.
    pub(crate) fn any(&self) -> bool {
        self.moving.mask.is_some() || self.fixed.mask.is_some()
    }

    /// Whether this side carries marks that still describe the mesh in front of
    /// the operator.
    ///
    /// The panel and the preview ask this before they attach anything. A mask
    /// that exists but marks nothing (Fit everywhere leaves exactly that) is
    /// not a reason to replace a scan's colours on the GPU, and the brush
    /// opening on an unmarked pair must not repaint both arches for nothing.
    pub(crate) fn has_marks(&self, side: AlignSide, mesh: MarkedOn) -> bool {
        let state = self.side(side);
        state.marked > 0 && state.fitting(mesh).is_some()
    }

    /// Paint one dab. Returns how many vertices changed; the list of which ones
    /// is in [`Self::touched`] for this side.
    pub(crate) fn dab(&mut self, side: AlignSide, mesh: &MarkedMesh<'_>, edit: &MaskEdit) -> usize {
        // The list is taken out of this side so the edit below can borrow the
        // mask and the list mutably at once, and so each side keeps its own.
        let mut touched = std::mem::take(&mut self.side_mut(side).touched);
        let mut owned = self.side_mut(side).take_for_edit(mesh.identity());
        touched.clear();
        // In place through `Arc::make_mut`: this type holds the only reference
        // while the dab runs, so nothing is copied.
        let changed = apply_brush(
            Arc::make_mut(&mut owned).as_mut_slice(),
            mesh.positions,
            mesh.pose,
            edit,
            &mut touched,
        );
        let state = self.side_mut(side);
        state.marked = if edit.erase {
            state.marked.saturating_sub(changed)
        } else {
            state.marked.saturating_add(changed)
        };
        state.mask = Some(owned);
        state.touched = touched;
        self.stroke_open = true;
        if changed > 0 {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    /// Run one whole-mesh command against one side.
    ///
    /// Returns `None` when the command reached no mask at all, else what it
    /// left. The count is the part the caller could not see before: `MarkAutomatic`
    /// on a layer smaller than the brush radius clears EVERY vertex, so it
    /// excludes nothing while the status line still said "Fit only at the arrow
    /// ends" — the next Best fit then used the whole surface under a sentence
    /// claiming the opposite.
    pub(crate) fn command(
        &mut self,
        side: AlignSide,
        command: MaskCommand,
        mesh: &MarkedMesh<'_>,
        keep: &AutoKeep<'_>,
    ) -> Option<MaskCommandOutcome> {
        if mesh.vertex_count == 0 {
            return None;
        }
        if command == MaskCommand::MarkAutomatic && keep.centres.is_empty() {
            return None;
        }
        let mut owned = self.side_mut(side).take_for_edit(mesh.identity());
        let previously_marked = self.side(side).marked;
        let mask = Arc::make_mut(&mut owned).as_mut_slice();
        let marked = match command {
            MaskCommand::FitEverywhere => {
                set_all(mask, false);
                0
            }
            MaskCommand::FitNowhere => {
                set_all(mask, true);
                mesh.vertex_count
            }
            MaskCommand::InvertMarkings => {
                invert(mask);
                mesh.vertex_count - previously_marked
            }
            MaskCommand::MarkAutomatic => {
                // Written as mark-everything then clear-the-discs, because the
                // discs are what the operator wants MATCHED and the mask stores
                // what is ignored.
                set_all(mask, true);
                let mut cleared = 0usize;
                let mut touched = std::mem::take(&mut self.side_mut(side).touched);
                touched.clear();
                for center in keep.centres {
                    cleared += apply_brush(
                        mask,
                        mesh.positions,
                        mesh.pose,
                        &MaskEdit {
                            center: *center,
                            radius_mm: keep.radius_mm,
                            erase: true,
                        },
                        &mut touched,
                    );
                }
                self.side_mut(side).touched = touched;
                mesh.vertex_count - cleared
            }
        };
        let state = self.side_mut(side);
        state.marked = marked;
        state.mask = Some(owned);
        self.revision = self.revision.wrapping_add(1);
        Some(MaskCommandOutcome {
            marked,
            vertex_count: mesh.vertex_count,
        })
    }

    /// Trade the two sides, because the scans traded roles.
    ///
    /// A marking belongs to a surface, not to a role. When the operator swaps
    /// which scan moves, leaving the masks alone would take the region they
    /// painted on one arch and apply it to the other — silently excluding
    /// anatomy nobody marked.
    pub(crate) fn swap_sides(&mut self) -> bool {
        if !self.any() {
            return false;
        }
        std::mem::swap(&mut self.moving, &mut self.fixed);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Drop every marking on both scans. Bumps the revision only if there was
    /// something to drop, so a Cancel on an unmarked pair costs no re-measure.
    pub(crate) fn clear(&mut self) -> bool {
        if !self.any() {
            self.stroke_open = false;
            return false;
        }
        self.moving = SideMarkings::default();
        self.fixed = SideMarkings::default();
        self.stroke_open = false;
        self.revision = self.revision.wrapping_add(1);
        true
    }
}

/// What [`MaskCommand::MarkAutomatic`] leaves in the match: a disc of surface
/// at each arrow end. Every other command ignores it.
pub(crate) struct AutoKeep<'a> {
    /// Arrow ends on this side's mesh, in world coordinates.
    pub(crate) centres: &'a [DVec3],
    /// How much surface to keep around each one.
    pub(crate) radius_mm: f64,
}

/// The mesh one marking operation acts on, as plain arrays.
///
/// Passed rather than looked up so this whole type stays reachable from a test
/// with no scene, no camera and no GPU behind it.
pub(crate) struct MarkedMesh<'a> {
    /// Vertex positions in the mesh's own local frame, three floats per vertex.
    pub(crate) positions: &'a [f32],
    /// Where that mesh sits in the world.
    pub(crate) pose: Rigid,
    /// How many vertices the mesh has.
    pub(crate) vertex_count: usize,
    /// Which mesh this is. Changes whenever its vertices do.
    pub(crate) geometry: u64,
}

impl MarkedMesh<'_> {
    /// What a mask painted on this mesh has to match later.
    pub(crate) fn identity(&self) -> MarkedOn {
        MarkedOn {
            geometry: self.geometry,
            vertex_count: self.vertex_count,
        }
    }
}

// Split out to hold the workspace's 800-line file budget.
#[cfg(test)]
#[path = "align_markings_tests.rs"]
mod tests;

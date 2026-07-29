//! What the operator has marked out of the match, on **both** scans at once.
//!
//! exocad marks on either mesh, so this owns two masks rather than one. The
//! type exists because the pieces below have to move together and used to sit
//! as five loose fields on the application struct: the two masks, the revision
//! every cache keys on, and the scratch list of vertices the last dab touched.
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
/// Blue, because that is the colour exocad paints an excluded region, and an
/// operator who works in that dialog should not have to learn a second
/// convention here. Defined next to the markings themselves because both the
/// surface and the sentence in the Brush window use it — they were two separate
/// literals in two files, each with a comment claiming they matched.
pub(crate) const MARKED_OUT_COLOR: [u8; 4] = [58, 108, 196, 255];

/// The colour surface that still takes part in the match is tinted — a neutral
/// stone, so the marked surface is the only thing that draws the eye.
pub(crate) const MARKED_IN_COLOR: [u8; 4] = [228, 216, 196, 255];

/// Which scan of the pair a marking belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignSide {
    /// The scan being placed.
    Moving,
    /// The scan that stays put.
    Fixed,
}

impl AlignSide {
    /// Both sides, for the commands that mean "the mesh" rather than "this one".
    pub(crate) const BOTH: [Self; 2] = [Self::Moving, Self::Fixed];
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

    /// The label on the button, verbatim from exocad.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Fit everywhere",
            Self::FitNowhere => "Fit nowhere",
            Self::InvertMarkings => "Invert markings",
            Self::MarkAutomatic => "Mark automatic",
        }
    }

    /// What the button does, in one line.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Clear all existing markings",
            Self::FitNowhere => "Mark the complete mesh — best-fit matching will have no effect",
            Self::InvertMarkings => "Mark unmarked areas and vice versa",
            Self::MarkAutomatic => "Match only on a small area around each arrow end",
        }
    }

    /// What to tell the operator afterwards.
    pub(crate) fn report(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Markings cleared — matching on the whole scan",
            Self::FitNowhere => "Whole mesh marked — best-fit matching will have no effect",
            Self::InvertMarkings => "Markings inverted",
            Self::MarkAutomatic => "Matching only around the arrow ends",
        }
    }
}

/// One scan's markings, and how much of it they cover.
#[derive(Clone, Debug, Default)]
struct SideMarkings {
    /// One byte per vertex, or nothing if this side was never marked.
    mask: Option<Arc<Vec<u8>>>,
    /// How many of those bytes are [`EXCLUDED`]. Kept in step with `mask` by
    /// every method below, so the panel never has to count.
    marked: usize,
}

impl SideMarkings {
    /// The mask, but only if it still describes a mesh of this size. A mask
    /// left over from other geometry is not a reading about this mesh.
    fn fitting(&self, vertex_count: usize) -> Option<&Arc<Vec<u8>>> {
        self.mask.as_ref().filter(|mask| mask.len() == vertex_count)
    }

    /// Take the mask out for editing, or make a fresh unmarked one.
    fn take_for_edit(&mut self, vertex_count: usize) -> Arc<Vec<u8>> {
        match self.mask.take() {
            Some(existing) if existing.len() == vertex_count => existing,
            _ => {
                self.marked = 0;
                Arc::new(vec![INCLUDED; vertex_count])
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
    /// The vertices the last dab actually changed. Held across dabs so a stroke
    /// does not allocate per frame.
    touched: Vec<u32>,
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
    pub(crate) fn mask_for(&self, side: AlignSide, vertex_count: usize) -> Option<Arc<Vec<u8>>> {
        self.side(side).fitting(vertex_count).map(Arc::clone)
    }

    /// The generation every downstream cache keys on.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The vertices the last dab changed.
    pub(crate) fn touched(&self) -> &[u32] {
        &self.touched
    }

    /// Note that the pointer came up. Returns whether a stroke was actually
    /// open, which is the caller's cue to re-measure.
    pub(crate) fn close_stroke(&mut self) -> bool {
        std::mem::take(&mut self.stroke_open)
    }

    /// What share of the two scans is marked, or nothing if neither carries a
    /// mask that fits its mesh. Free to call: the counts are maintained here.
    pub(crate) fn marked_fraction(
        &self,
        moving_vertices: usize,
        fixed_vertices: usize,
    ) -> Option<f32> {
        let mut marked = 0usize;
        let mut total = 0usize;
        for (side, vertex_count) in [
            (AlignSide::Moving, moving_vertices),
            (AlignSide::Fixed, fixed_vertices),
        ] {
            let state = self.side(side);
            if state.fitting(vertex_count).is_some() {
                marked += state.marked;
                total += vertex_count;
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

    /// Paint one dab. Returns how many vertices changed; the list of which ones
    /// is in [`Self::touched`].
    pub(crate) fn dab(&mut self, side: AlignSide, mesh: &MarkedMesh<'_>, edit: &MaskEdit) -> usize {
        let mut owned = self.side_mut(side).take_for_edit(mesh.vertex_count);
        let mut touched = std::mem::take(&mut self.touched);
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
        self.touched = touched;
        let state = self.side_mut(side);
        state.marked = if edit.erase {
            state.marked.saturating_sub(changed)
        } else {
            state.marked.saturating_add(changed)
        };
        state.mask = Some(owned);
        self.stroke_open = true;
        if changed > 0 {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    /// Run one whole-mesh command against one side. Returns whether it reached
    /// a mask at all.
    pub(crate) fn command(
        &mut self,
        side: AlignSide,
        command: MaskCommand,
        mesh: &MarkedMesh<'_>,
        keep: &AutoKeep<'_>,
    ) -> bool {
        if mesh.vertex_count == 0 {
            return false;
        }
        if command == MaskCommand::MarkAutomatic && keep.centres.is_empty() {
            return false;
        }
        let mut owned = self.side_mut(side).take_for_edit(mesh.vertex_count);
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
                let mut touched = std::mem::take(&mut self.touched);
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
                self.touched = touched;
                mesh.vertex_count - cleared
            }
        };
        let state = self.side_mut(side);
        state.marked = marked;
        state.mask = Some(owned);
        self.revision = self.revision.wrapping_add(1);
        true
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
        self.touched.clear();
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
        self.touched.clear();
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
}

// Split out to hold the workspace's 800-line file budget.
#[cfg(test)]
#[path = "align_markings_tests.rs"]
mod tests;

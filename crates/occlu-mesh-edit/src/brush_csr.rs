//! Compressed-sparse-row neighbour storage for the brush kernel.
//!
//! Hot dab passes iterate a vertex's one-ring, incident triangles, and soup
//! siblings tens of thousands of times per dab. A `Vec<Vec<usize>>` makes every
//! lookup chase a per-vertex heap pointer (a cache miss). CSR — one flat `data`
//! array sliced by per-vertex `offsets`, stored as `u32` — makes a lookup one
//! bounds pair plus a dense slice, and builds from two arrays instead of one
//! `Vec` per vertex (a real win on a million-vertex `prepare`).
//!
//! # Growing without losing the flat fast path
//!
//! Dynamic-topology densification ([`super::brush::BrushSession::refine_dab`])
//! splits edges mid-stroke, which rewrites a handful of rows and appends new
//! ones. A flat CSR cannot absorb that in place, and rebuilding it per dab
//! would be O(mesh) on a million-vertex scan. Instead an EDITED row moves into
//! an overlay `Vec<u32>` and `overlay_of` redirects it; every untouched row —
//! all but a few dozen per split — still reads straight out of the flat
//! arrays. `overlay_of` stays empty until the first edit, so a session that
//! never densifies pays one bounds check against an empty vector.

/// Sentinel in `overlay_of`: this row still lives in the flat base arrays.
const NO_OVERLAY: u32 = u32::MAX;

/// Shared empty slice for out-of-range / empty rows.
const EMPTY_ROW: &[u32] = &[];

/// A compressed-sparse-row adjacency: `data[offsets[i]..offsets[i + 1]]` are
/// the neighbours of row `i`, as `u32` vertex/triangle ids, unless row `i` has
/// been edited and redirected into `overlay`.
pub(crate) struct Csr {
    /// `base_rows + 1` prefix sums; base row `i` spans `offsets[i]..offsets[i + 1]`.
    offsets: Vec<u32>,
    /// Flat neighbour ids for every base row, concatenated in row order.
    data: Vec<u32>,
    /// Total row count, including rows appended by [`Self::push_row`].
    rows: usize,
    /// Per-row overlay slot, or [`NO_OVERLAY`]. Empty until the first edit.
    overlay_of: Vec<u32>,
    /// Edited/appended rows, indexed by the slots in `overlay_of`.
    overlay: Vec<Vec<u32>>,
}

impl Csr {
    /// Build directly from an iterator of `(row, neighbour)` pairs via a
    /// counting sort — no intermediate `Vec<Vec<_>>`. Pairs are appended in
    /// first-seen order within each row.
    pub(crate) fn from_pairs(
        rows: usize,
        pairs: impl Iterator<Item = (usize, usize)> + Clone,
    ) -> Self {
        let mut counts = vec![0u32; rows + 1];
        for (row, _) in pairs.clone() {
            if row < rows {
                counts[row] += 1;
            }
        }
        // Prefix-sum the counts into offsets.
        let mut running = 0u32;
        for slot in &mut counts {
            let here = *slot;
            *slot = running;
            running += here;
        }
        let offsets = counts;
        // Scatter into the flat array using a moving write cursor per row.
        let mut cursor: Vec<u32> = offsets.clone();
        let mut data = vec![0u32; running as usize];
        for (row, neighbour) in pairs {
            if row < rows {
                let at = cursor[row] as usize;
                #[allow(clippy::cast_possible_truncation)]
                {
                    data[at] = neighbour as u32;
                }
                cursor[row] += 1;
            }
        }
        Self {
            offsets,
            data,
            rows,
            overlay_of: Vec::new(),
            overlay: Vec::new(),
        }
    }

    /// Build from an existing `Vec<Vec<usize>>` (for connectivity produced by a
    /// shared helper that still returns rows-of-vecs), flattening it into CSR.
    pub(crate) fn from_rows(rows: &[Vec<usize>]) -> Self {
        let total: usize = rows.iter().map(Vec::len).sum();
        let mut offsets = Vec::with_capacity(rows.len() + 1);
        let mut data = Vec::with_capacity(total);
        let mut running = 0u32;
        offsets.push(0);
        for row in rows {
            for &neighbour in row {
                #[allow(clippy::cast_possible_truncation)]
                data.push(neighbour as u32);
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                running += row.len() as u32;
            }
            offsets.push(running);
        }
        Self {
            offsets,
            data,
            rows: rows.len(),
            overlay_of: Vec::new(),
            overlay: Vec::new(),
        }
    }

    /// The neighbours of row `i` as a dense slice; empty for an out-of-range
    /// row (the kernel indexes by vertex id, and a stale id must not panic).
    #[inline]
    pub(crate) fn row(&self, i: usize) -> &[u32] {
        if let Some(&slot) = self.overlay_of.get(i) {
            if slot != NO_OVERLAY {
                return self
                    .overlay
                    .get(slot as usize)
                    .map_or(EMPTY_ROW, Vec::as_slice);
            }
        }
        let (Some(&start), Some(&end)) = (self.offsets.get(i), self.offsets.get(i + 1)) else {
            return EMPTY_ROW;
        };
        self.data
            .get(start as usize..end as usize)
            .unwrap_or(EMPTY_ROW)
    }

    /// Number of neighbours in row `i`.
    #[inline]
    pub(crate) fn row_len(&self, i: usize) -> usize {
        self.row(i).len()
    }

    /// Whether row `i` has no neighbours.
    #[inline]
    pub(crate) fn is_empty_row(&self, i: usize) -> bool {
        self.row(i).is_empty()
    }

    /// Append an empty row and return its id — one per vertex/triangle minted
    /// by densification, so every parallel per-row array stays the same length.
    pub(crate) fn push_row(&mut self) -> usize {
        let id = self.rows;
        self.rows += 1;
        self.reserve_overlay_index();
        let slot = self.overlay.len();
        self.overlay.push(Vec::new());
        self.point_at_overlay(id, slot);
        id
    }

    /// Append `value` to row `i` unless it is already there. Order is
    /// append-only, so the same edit sequence always yields the same row.
    pub(crate) fn add_neighbour(&mut self, i: usize, value: u32) {
        if i >= self.rows || self.row(i).contains(&value) {
            return;
        }
        let slot = self.overlay_slot(i);
        if let Some(row) = self.overlay.get_mut(slot) {
            row.push(value);
        }
    }

    /// Drop every occurrence of `value` from row `i`, preserving the order of
    /// the survivors.
    pub(crate) fn remove_neighbour(&mut self, i: usize, value: u32) {
        if i >= self.rows || !self.row(i).contains(&value) {
            return;
        }
        let slot = self.overlay_slot(i);
        if let Some(row) = self.overlay.get_mut(slot) {
            row.retain(|&neighbour| neighbour != value);
        }
    }

    /// Materialize row `i` into the overlay (copying the flat row on first
    /// touch) and return its overlay slot.
    fn overlay_slot(&mut self, i: usize) -> usize {
        self.reserve_overlay_index();
        if let Some(&slot) = self.overlay_of.get(i) {
            if slot != NO_OVERLAY {
                return slot as usize;
            }
        }
        let materialized = self.row(i).to_vec();
        let slot = self.overlay.len();
        self.overlay.push(materialized);
        self.point_at_overlay(i, slot);
        slot
    }

    /// Size the redirect table to the current row count, allocating it lazily
    /// so an un-densified session never pays for it.
    fn reserve_overlay_index(&mut self) {
        if self.overlay_of.len() < self.rows {
            self.overlay_of.resize(self.rows, NO_OVERLAY);
        }
    }

    fn point_at_overlay(&mut self, row: usize, slot: usize) {
        if let (Some(entry), Ok(slot)) = (self.overlay_of.get_mut(row), u32::try_from(slot)) {
            *entry = slot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rows_round_trips_every_neighbour() {
        let rows = vec![vec![1usize, 2], vec![], vec![0, 1, 3], vec![2]];
        let csr = Csr::from_rows(&rows);
        for (i, row) in rows.iter().enumerate() {
            let got: Vec<usize> = csr.row(i).iter().map(|&n| n as usize).collect();
            assert_eq!(&got, row, "row {i}");
            assert_eq!(csr.row_len(i), row.len());
            assert_eq!(csr.is_empty_row(i), row.is_empty());
        }
    }

    #[test]
    fn from_pairs_groups_by_row_in_order() {
        // Undirected-ish pairs; each row collects its neighbours in first-seen
        // order, matching what an incidence/sibling build produces.
        let pairs = vec![(0usize, 5usize), (2, 7), (0, 6), (2, 8), (2, 9)];
        let csr = Csr::from_pairs(3, pairs.into_iter());
        assert_eq!(csr.row(0), &[5, 6]);
        assert!(csr.is_empty_row(1));
        assert_eq!(csr.row(2), &[7, 8, 9]);
    }

    #[test]
    fn empty_mesh_has_only_the_zero_offset() {
        let csr = Csr::from_rows(&[]);
        assert_eq!(csr.offsets, vec![0]);
        assert!(csr.data.is_empty());
    }

    #[test]
    fn editing_a_row_leaves_every_other_row_on_the_flat_path() {
        let mut csr = Csr::from_rows(&[vec![1usize, 2], vec![0], vec![0]]);
        csr.remove_neighbour(0, 2);
        csr.add_neighbour(0, 7);
        assert_eq!(csr.row(0), &[1, 7]);
        // Untouched rows still read the base arrays byte-for-byte.
        assert_eq!(csr.row(1), &[0]);
        assert_eq!(csr.row(2), &[0]);
    }

    #[test]
    fn appended_rows_extend_the_row_count_and_accept_neighbours() {
        let mut csr = Csr::from_rows(&[vec![1usize], vec![0]]);
        let fresh = csr.push_row();
        assert_eq!(fresh, 2);
        assert!(csr.is_empty_row(fresh));
        csr.add_neighbour(fresh, 0);
        csr.add_neighbour(fresh, 1);
        // A duplicate is ignored, keeping rings free of repeats.
        csr.add_neighbour(fresh, 1);
        assert_eq!(csr.row(fresh), &[0, 1]);
    }

    #[test]
    fn out_of_range_rows_and_edits_are_silent_no_ops() {
        let mut csr = Csr::from_rows(&[vec![1usize], vec![0]]);
        assert!(csr.row(99).is_empty());
        assert_eq!(csr.row_len(99), 0);
        csr.add_neighbour(99, 3);
        csr.remove_neighbour(99, 3);
        assert!(csr.row(1).is_empty() || csr.row(1) == [0]);
    }

    #[test]
    fn removing_a_missing_neighbour_never_materializes_a_row() {
        let mut csr = Csr::from_rows(&[vec![1usize, 2], vec![0]]);
        csr.remove_neighbour(0, 42);
        assert_eq!(csr.row(0), &[1, 2]);
        assert!(csr.overlay.is_empty());
    }
}

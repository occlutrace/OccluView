//! Compressed-sparse-row neighbour storage for the brush kernel.
//!
//! Hot dab passes iterate a vertex's one-ring, incident triangles, and soup
//! siblings tens of thousands of times per dab. A `Vec<Vec<usize>>` makes every
//! lookup chase a per-vertex heap pointer (a cache miss). CSR — one flat `data`
//! array sliced by per-vertex `offsets`, stored as `u32` — makes a lookup one
//! bounds pair plus a dense slice, and builds from two arrays instead of one
//! `Vec` per vertex (a real win on a million-vertex `prepare`).

/// A read-only compressed-sparse-row adjacency: `data[offsets[i]..offsets[i+1]]`
/// are the neighbours of row `i`, as `u32` vertex/triangle ids.
pub(crate) struct Csr {
    /// `rows + 1` prefix sums; row `i` spans `offsets[i]..offsets[i + 1]`.
    offsets: Vec<u32>,
    /// Flat neighbour ids for every row, concatenated in row order.
    data: Vec<u32>,
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
        Self { offsets, data }
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
        Self { offsets, data }
    }

    /// The neighbours of row `i` as a dense slice.
    #[inline]
    pub(crate) fn row(&self, i: usize) -> &[u32] {
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.data[start..end]
    }

    /// Number of neighbours in row `i`.
    #[inline]
    pub(crate) fn row_len(&self, i: usize) -> usize {
        (self.offsets[i + 1] - self.offsets[i]) as usize
    }

    /// Whether row `i` has no neighbours.
    #[inline]
    pub(crate) fn is_empty_row(&self, i: usize) -> bool {
        self.offsets[i + 1] == self.offsets[i]
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
}

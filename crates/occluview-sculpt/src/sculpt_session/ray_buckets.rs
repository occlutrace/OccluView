use super::DVec3;

type Cell = (i32, i32, i32);
/// The inclusive cell range a triangle's bounding box covers. A triangle is
/// stored in every cell of its range, so the range is its cell list.
pub(super) type CellSpan = (Cell, Cell);

/// Maximum number of triangle-to-cell references stored by the ray index.
/// This is an index-work budget, not an RSS estimate or a geometry limit:
/// excess and unrepresentable triangles remain live in the exact-scan spill
/// list.
const MAX_BUCKET_REFERENCES: usize = 1 << 20;

fn span_cells(span: CellSpan) -> impl Iterator<Item = Cell> {
    let (first, last) = span;
    (first.0..=last.0).flat_map(move |x| {
        (first.1..=last.1).flat_map(move |y| (first.2..=last.2).map(move |z| (x, y, z)))
    })
}

/// Count an inclusive span without visiting its cells. A span whose extent or
/// product cannot be represented is unsuitable for the bounded grid and is
/// handled by the exact-scan spill list.
fn span_cell_count(span: CellSpan) -> Option<usize> {
    let extent = |low: i32, high: i32| {
        u64::try_from(
            i64::from(high)
                .checked_sub(i64::from(low))?
                .checked_add(1)?,
        )
        .ok()
    };
    let (low, high) = span;
    let cells = extent(low.0, high.0)?
        .checked_mul(extent(low.1, high.1)?)?
        .checked_mul(extent(low.2, high.2)?)?;
    usize::try_from(cells).ok()
}

fn cell_coordinate(value: f64, origin: f64, cell: f64) -> Option<i32> {
    let coordinate = ((value - origin) / cell).floor();
    if !coordinate.is_finite()
        || coordinate < f64::from(i32::MIN)
        || coordinate > f64::from(i32::MAX)
    {
        return None;
    }
    Some(coordinate as i32)
}

/// Triangle bucket grid for raycasts against the current deformed surface.
/// Triangle AABBs stamp occupied cells up to a fixed reference budget; broad
/// or unrepresentable AABBs are kept in `spilled_triangles` and ray-tested
/// exactly. The triangle slot remains present in `triangle_cells` either way.
pub(super) struct TriBuckets {
    pub(super) cell: f64,
    pub(super) lo: DVec3,
    /// Bounds of occupied regular cells, maintained without rebinning the mesh.
    pub(super) min_cell: Cell,
    pub(super) max_cell: Cell,
    pub(super) occupied_axes: [std::collections::BTreeMap<i32, usize>; 3],
    pub(super) map: crate::hash::FxHashMap<Cell, Vec<u32>>,
    /// Per triangle, its regular cell span; `None` means it is exactly scanned
    /// from `spilled_triangles`.
    pub(super) triangle_cells: Vec<Option<CellSpan>>,
    /// Ascending live triangle ids whose spans are not represented in `map`.
    pub(super) spilled_triangles: Vec<u32>,
    /// Total number of triangle-to-cell references currently in `map`.
    bucket_references: usize,
    /// Test-only source counter for proving batch removal scans each affected
    /// bucket once, independent of the number of changed triangles.
    #[cfg(test)]
    last_batch_cell_passes: usize,
}

impl TriBuckets {
    pub(super) fn build(verts: &[f32], tris: &[u32], cell: f64) -> TriBuckets {
        let mut lo = DVec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let v = |i: u32| {
            let k = i as usize * 3;
            DVec3::new(verts[k] as f64, verts[k + 1] as f64, verts[k + 2] as f64)
        };
        for i in 0..verts.len() / 3 {
            let p = v(i as u32);
            lo = lo.min(p);
        }
        let mut grid = TriBuckets {
            cell,
            lo,
            min_cell: (0, 0, 0),
            max_cell: (0, 0, 0),
            occupied_axes: Default::default(),
            map: crate::hash::FxHashMap::default(),
            triangle_cells: Vec::with_capacity(tris.len() / 3),
            spilled_triangles: Vec::new(),
            bucket_references: 0,
            #[cfg(test)]
            last_batch_cell_passes: 0,
        };
        let mut spilled = Vec::new();
        for (ti, _) in tris.as_chunks::<3>().0.iter().enumerate() {
            let triangle = ti as u32;
            let span = grid.span_of_triangle(verts, tris, triangle);
            let indexed = grid.assign_triangle(triangle, span, &mut spilled);
            grid.triangle_cells.push(indexed);
        }
        grid.spilled_triangles = spilled;
        grid.refresh_occupied_bounds();
        grid
    }

    /// Room for `triangles` appended faces, including the worst-case spill ids.
    pub(super) fn reserve_growth(&mut self, triangles: usize) {
        self.triangle_cells.reserve(triangles);
        self.spilled_triangles.reserve(triangles);
    }

    pub(super) fn refresh_occupied_bounds(&mut self) {
        let low = std::array::from_fn::<_, 3, _>(|axis| {
            self.occupied_axes[axis]
                .first_key_value()
                .map_or(0, |(&key, _)| key)
        });
        let high = std::array::from_fn::<_, 3, _>(|axis| {
            self.occupied_axes[axis]
                .last_key_value()
                .map_or(0, |(&key, _)| key)
        });
        self.min_cell = (low[0], low[1], low[2]);
        self.max_cell = (high[0], high[1], high[2]);
    }

    pub(super) fn span_for_points(&self, a: DVec3, b: DVec3, c: DVec3) -> Option<CellSpan> {
        let low = a.min(b).min(c);
        let high = a.max(b).max(c);
        Some((
            (
                cell_coordinate(low.x, self.lo.x, self.cell)?,
                cell_coordinate(low.y, self.lo.y, self.cell)?,
                cell_coordinate(low.z, self.lo.z, self.cell)?,
            ),
            (
                cell_coordinate(high.x, self.lo.x, self.cell)?,
                cell_coordinate(high.y, self.lo.y, self.cell)?,
                cell_coordinate(high.z, self.lo.z, self.cell)?,
            ),
        ))
    }

    fn span_of_triangle(&self, verts: &[f32], tris: &[u32], triangle: u32) -> Option<CellSpan> {
        let offset = triangle as usize * 3;
        let vertex = |index: u32| {
            let k = index as usize * 3;
            DVec3::new(verts[k] as f64, verts[k + 1] as f64, verts[k + 2] as f64)
        };
        self.span_for_points(
            vertex(tris[offset]),
            vertex(tris[offset + 1]),
            vertex(tris[offset + 2]),
        )
    }

    /// Count a cell becoming occupied or empty on the axis bounds.
    fn note_occupancy(&mut self, cell: Cell, occupied: bool) {
        for (axis, coordinate) in [cell.0, cell.1, cell.2].into_iter().enumerate() {
            if occupied {
                *self.occupied_axes[axis].entry(coordinate).or_default() += 1;
            } else if let std::collections::btree_map::Entry::Occupied(mut entry) =
                self.occupied_axes[axis].entry(coordinate)
            {
                *entry.get_mut() -= 1;
                if *entry.get() == 0 {
                    entry.remove();
                }
            }
        }
    }

    fn add_triangle_to_buckets(&mut self, triangle: u32, span: CellSpan, references: usize) {
        for cell in span_cells(span) {
            let became_occupied = {
                let entries = self.map.entry(cell).or_default();
                let was_empty = entries.is_empty();
                // IDs are unique within a cell, but updates can append an
                // older ID after untouched faces. Ray hits resolve exact
                // distance ties by ID, so bucket storage need not stay sorted.
                entries.push(triangle);
                was_empty
            };
            if became_occupied {
                self.note_occupancy(cell, true);
            }
        }
        self.bucket_references += references;
        debug_assert!(self.bucket_references <= MAX_BUCKET_REFERENCES);
    }

    /// Remove selected triangle ids from the union of their old cells. Each
    /// affected bucket is compacted once, even when changed faces overlap it.
    /// Updates use sorted touched-id membership; drops select a dense tail.
    fn remove_triangles_from_cells(
        &mut self,
        cells: &[Cell],
        mut should_remove: impl FnMut(u32) -> bool,
    ) -> (usize, usize) {
        let mut removed_references = 0;
        let mut cell_passes = 0;
        for &cell in cells {
            let became_empty = if let Some(entries) = self.map.get_mut(&cell) {
                cell_passes += 1;
                let old_len = entries.len();
                entries.retain(|&candidate| !should_remove(candidate));
                removed_references += old_len - entries.len();
                entries.is_empty()
            } else {
                false
            };
            if became_empty {
                self.map.remove(&cell);
                self.note_occupancy(cell, false);
            }
        }
        (removed_references, cell_passes)
    }

    /// Merge ascending spill ids into the existing ascending list in place.
    /// Batch updates avoid repeated linear shifts when many faces spill.
    fn merge_spilled_triangles(&mut self, additions: &[u32]) {
        if additions.is_empty() {
            return;
        }
        let mut old = self.spilled_triangles.len();
        let mut added = additions.len();
        self.spilled_triangles.reserve(added);
        self.spilled_triangles.extend_from_slice(additions);
        let mut write = old + added;
        while old > 0 && added > 0 {
            write -= 1;
            if self.spilled_triangles[old - 1] > additions[added - 1] {
                self.spilled_triangles[write] = self.spilled_triangles[old - 1];
                old -= 1;
            } else {
                self.spilled_triangles[write] = additions[added - 1];
                added -= 1;
            }
        }
        while added > 0 {
            write -= 1;
            self.spilled_triangles[write] = additions[added - 1];
            added -= 1;
        }
    }

    /// Put a triangle into the regular index only when its checked span fits
    /// the remaining association budget. Otherwise retain its live slot in
    /// the exact-scan spill list.
    fn assign_triangle(
        &mut self,
        triangle: u32,
        candidate: Option<CellSpan>,
        spilled: &mut Vec<u32>,
    ) -> Option<CellSpan> {
        let Some((span, references)) = candidate.and_then(|span| {
            span_cell_count(span).map(|references| (span, references))
        }) else {
            spilled.push(triangle);
            return None;
        };
        if references > MAX_BUCKET_REFERENCES.saturating_sub(self.bucket_references) {
            spilled.push(triangle);
            return None;
        }
        self.add_triangle_to_buckets(triangle, span, references);
        Some(span)
    }

    pub(super) fn update_triangles(&mut self, verts: &[f32], tris: &[u32], triangles: &[u32]) {
        // Process changed ids in ascending order. Releasing the entire old
        // batch first lets its new spans reuse references freed by that batch;
        // assignment order is stable when the remaining budget is tight.
        let mut touched = triangles.to_vec();
        touched.sort_unstable();
        touched.dedup();
        if touched.is_empty() {
            return;
        }
        #[cfg(test)]
        {
            self.last_batch_cell_passes = 0;
        }
        let updates: Vec<_> = touched
            .iter()
            .map(|&triangle| (triangle, self.span_of_triangle(verts, tris, triangle)))
            .collect();

        let mut affected_cells = Vec::new();
        let mut references = 0;
        for &(triangle, _) in &updates {
            if let Some(span) = self.triangle_cells[triangle as usize].take() {
                references += span_cell_count(span)
                    .expect("a regular ray span has a checked cell count");
                affected_cells.extend(span_cells(span));
            }
        }
        affected_cells.sort_unstable();
        affected_cells.dedup();

        let (removed_references, _cell_passes) = self.remove_triangles_from_cells(
            &affected_cells,
            |triangle| touched.binary_search(&triangle).is_ok(),
        );
        debug_assert_eq!(removed_references, references);
        debug_assert!(self.bucket_references >= references);
        self.bucket_references -= references;
        #[cfg(test)]
        {
            self.last_batch_cell_passes = _cell_passes;
        }

        let mut touched_index = 0;
        self.spilled_triangles.retain(|&triangle| {
            while touched_index < touched.len() && touched[touched_index] < triangle {
                touched_index += 1;
            }
            let remove = touched.get(touched_index) == Some(&triangle);
            if remove {
                touched_index += 1;
            }
            !remove
        });
        let mut spilled = Vec::new();
        for (triangle, next) in updates {
            let indexed = self.assign_triangle(triangle, next, &mut spilled);
            self.triangle_cells[triangle as usize] = indexed;
        }
        self.merge_spilled_triangles(&spilled);
        self.refresh_occupied_bounds();
    }

    /// Index brand-new trailing triangles (densification appends). Reads the
    /// already-extended index buffer; `first_new` is the first appended id.
    /// Stops at `live_tris`: a topology undo truncates the live prefix but
    /// leaves dead words in the allocated tail, and those stale triples can
    /// name vertices the same undo already removed. Indexing them walks
    /// `verts` out of bounds.
    pub(super) fn insert_triangles(
        &mut self,
        verts: &[f32],
        tris: &[u32],
        first_new: u32,
        live_tris: u32,
    ) {
        let count = (tris.len() / 3).min(live_tris as usize);
        let mut spilled = Vec::new();
        for index in first_new as usize..count {
            let triangle = index as u32;
            let span = self.span_of_triangle(verts, tris, triangle);
            let indexed = self.assign_triangle(triangle, span, &mut spilled);
            self.triangle_cells.push(indexed);
        }
        self.merge_spilled_triangles(&spilled);
        self.refresh_occupied_bounds();
    }

    /// Forget every triangle at or past `from_id` (topology undo). The caller
    /// truncates its own index buffer afterwards.
    pub(super) fn drop_triangles(&mut self, from_id: u32) {
        let from = (from_id as usize).min(self.triangle_cells.len());
        #[cfg(test)]
        {
            self.last_batch_cell_passes = 0;
        }
        let mut affected_cells = Vec::new();
        let mut references = 0;
        for index in from..self.triangle_cells.len() {
            if let Some(span) = self.triangle_cells[index].take() {
                references += span_cell_count(span)
                    .expect("a regular ray span has a checked cell count");
                affected_cells.extend(span_cells(span));
            }
        }
        affected_cells.sort_unstable();
        affected_cells.dedup();
        let (removed_references, _cell_passes) =
            self.remove_triangles_from_cells(&affected_cells, |triangle| triangle >= from_id);
        debug_assert_eq!(removed_references, references);
        debug_assert!(self.bucket_references >= references);
        self.bucket_references -= references;
        #[cfg(test)]
        {
            self.last_batch_cell_passes = _cell_passes;
        }
        self.spilled_triangles.retain(|&triangle| triangle < from_id);
        self.triangle_cells.truncate(from);
        self.refresh_occupied_bounds();
    }

    /// Reconcile the dense ray-grid prefix after topology history replay.
    /// The journal names every slot whose corners changed; only a size delta
    /// may add or remove a trailing row. This keeps local undo proportional to
    /// its journal instead of rebuilding every triangle in the case.
    pub(super) fn reconcile_history(
        &mut self,
        verts: &[f32],
        tris: &[u32],
        live_tris: u32,
        changed: &[u32],
    ) {
        let indexed = self.triangle_cells.len() as u32;
        if indexed > live_tris {
            self.drop_triangles(live_tris);
        }
        let common = indexed.min(live_tris);
        let mut existing: Vec<u32> = changed
            .iter()
            .copied()
            .filter(|&triangle| triangle < common)
            .collect();
        existing.sort_unstable();
        existing.dedup();
        if !existing.is_empty() {
            self.update_triangles(verts, tris, &existing);
        }
        if indexed < live_tris {
            self.insert_triangles(verts, tris, indexed, live_tris);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broad_triangle() -> Vec<f32> {
        vec![0.0, 0.0, 0.0, 20_000.0, 0.0, 0.0, 0.0, 20_000.0, 0.0]
    }

    fn small_triangle() -> Vec<f32> {
        vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0]
    }

    fn repeated_face_indices(count: usize) -> Vec<u32> {
        (0..count).flat_map(|_| [0, 1, 2]).collect()
    }

    #[test]
    fn broad_finite_span_spills_without_enumerating_its_cells() {
        let verts = broad_triangle();
        let grid = TriBuckets::build(&verts, &[0, 1, 2], 2.0);

        assert_eq!(grid.triangle_cells, vec![None]);
        assert_eq!(grid.spilled_triangles, vec![0]);
        assert_eq!(grid.bucket_references, 0);
        assert!(grid.map.is_empty());
        assert_eq!(grid.min_cell, (0, 0, 0));
        assert_eq!(grid.max_cell, (0, 0, 0));
    }

    #[test]
    fn unrepresentable_cell_coordinates_and_count_overflow_spill() {
        let verts = vec![-f32::MAX, 0.0, 0.0, f32::MAX, 0.0, 0.0, 0.0, 1.0, 0.0];
        let grid = TriBuckets::build(&verts, &[0, 1, 2], 2.0);

        assert_eq!(grid.triangle_cells, vec![None]);
        assert_eq!(grid.spilled_triangles, vec![0]);
        assert!(grid.map.is_empty());
        assert_eq!(
            span_cell_count(((i32::MIN, i32::MIN, i32::MIN), (i32::MAX, i32::MAX, i32::MAX))),
            None
        );
    }

    #[test]
    fn deformation_releases_and_reacquires_bucket_references() {
        let mut verts = broad_triangle();
        let tris = vec![0, 1, 2];
        let mut grid = TriBuckets::build(&verts, &tris, 2.0);
        assert_eq!(grid.spilled_triangles, vec![0]);

        verts[3] = 2.0;
        verts[7] = 2.0;
        grid.update_triangles(&verts, &tris, &[0]);
        assert_eq!(grid.triangle_cells.len(), 1);
        assert!(grid.triangle_cells[0].is_some());
        assert!(grid.spilled_triangles.is_empty());
        assert_eq!(grid.bucket_references, 4);
        assert_eq!(grid.map.values().map(Vec::len).sum::<usize>(), 4);

        verts[3] = 20_000.0;
        verts[7] = 20_000.0;
        grid.update_triangles(&verts, &tris, &[0]);
        assert_eq!(grid.triangle_cells, vec![None]);
        assert_eq!(grid.spilled_triangles, vec![0]);
        assert_eq!(grid.bucket_references, 0);
        assert!(grid.map.is_empty());
        assert_eq!(grid.min_cell, (0, 0, 0));
        assert_eq!(grid.max_cell, (0, 0, 0));
    }

    #[test]
    fn overlapping_batch_updates_compact_each_affected_bucket_once() {
        let verts = small_triangle();
        let tris = repeated_face_indices(32);
        let mut grid = TriBuckets::build(&verts, &tris, 2.0);

        grid.update_triangles(&verts, &tris, &(0..32).collect::<Vec<_>>());

        // Every face shares the same four-cell span. Work is one retain pass
        // per affected bucket, not one pass per face per bucket.
        assert_eq!(grid.last_batch_cell_passes, 4);
        assert_eq!(grid.bucket_references, 32 * 4);
        assert_eq!(grid.map.len(), 4);
        assert!(grid.map.values().all(|faces| faces.len() == 32));
    }

    #[test]
    fn dropping_a_dense_tail_compacts_each_affected_bucket_once() {
        let verts = small_triangle();
        let tris = repeated_face_indices(32);
        let mut grid = TriBuckets::build(&verts, &tris, 2.0);

        grid.drop_triangles(16);

        assert_eq!(grid.last_batch_cell_passes, 4);
        assert_eq!(grid.triangle_cells.len(), 16);
        assert_eq!(grid.bucket_references, 16 * 4);
        assert_eq!(grid.map.len(), 4);
        assert!(grid
            .map
            .values()
            .all(|faces| faces == &(0..16).collect::<Vec<_>>()));
    }

    #[test]
    fn reinserting_a_low_id_appends_after_untouched_bucket_members() {
        let mut verts = small_triangle();
        verts.extend_from_slice(&small_triangle());
        let tris = vec![0, 1, 2, 3, 4, 5];
        let mut grid = TriBuckets::build(&verts, &tris, 2.0);

        // Shrink face 0 into cell (0, 0, 0), which face 1 still occupies.
        verts[3] = 1.5;
        verts[7] = 1.5;
        grid.update_triangles(&verts, &tris, &[0]);

        assert_eq!(grid.map.get(&(0, 0, 0)), Some(&vec![1, 0]));
        assert_eq!(grid.triangle_cells[0], Some(((0, 0, 0), (0, 0, 0))));
        assert_eq!(grid.bucket_references, 5);
    }

    #[test]
    fn appended_spills_are_removed_with_their_dense_tail() {
        let mut verts = small_triangle();
        verts.extend_from_slice(&[20_000.0, 0.0, 0.0, 0.0, 20_000.0, 0.0, 0.0, 0.0, 0.0]);
        let tris = vec![0, 1, 2, 3, 4, 5];
        let mut grid = TriBuckets::build(&verts, &tris[..3], 2.0);
        let base_references = grid.bucket_references;

        grid.insert_triangles(&verts, &tris, 1, 2);
        assert_eq!(grid.triangle_cells, vec![Some(((0, 0, 0), (1, 1, 0))), None]);
        assert_eq!(grid.spilled_triangles, vec![1]);
        assert_eq!(grid.bucket_references, base_references);

        grid.drop_triangles(1);
        assert_eq!(grid.triangle_cells.len(), 1);
        assert!(grid.spilled_triangles.is_empty());
        assert_eq!(grid.bucket_references, base_references);
        assert!(grid.map.values().all(|triangles| triangles == &[0]));
    }

    #[test]
    fn history_reconcile_moves_live_slots_between_regular_and_spilled() {
        let mut verts = small_triangle();
        verts.extend_from_slice(&[20_000.0, 0.0, 0.0, 0.0, 20_000.0, 0.0, 0.0, 0.0, 0.0]);
        let tris = vec![0, 1, 2, 3, 4, 5];
        let mut grid = TriBuckets::build(&verts, &tris[..3], 2.0);

        // Redo appends a broad face, which stays live while spilling.
        grid.reconcile_history(&verts, &tris, 2, &[]);
        assert_eq!(grid.triangle_cells.len(), 2);
        assert_eq!(grid.spilled_triangles, vec![1]);

        // Undo rewires the surviving face to a broad span and truncates the
        // appended tail. Both the dense slots and the reference count follow.
        verts[3] = 20_000.0;
        verts[7] = 20_000.0;
        grid.reconcile_history(&verts, &tris, 1, &[0, 1]);
        assert_eq!(grid.triangle_cells, vec![None]);
        assert_eq!(grid.spilled_triangles, vec![0]);
        assert_eq!(grid.bucket_references, 0);
        assert!(grid.map.is_empty());

        // Redo restores a small surviving face and appends the broad face.
        verts[3] = 2.0;
        verts[7] = 2.0;
        grid.reconcile_history(&verts, &tris, 2, &[0]);
        assert_eq!(grid.triangle_cells.len(), 2);
        assert!(grid.triangle_cells[0].is_some());
        assert_eq!(grid.spilled_triangles, vec![1]);
        assert_eq!(grid.bucket_references, 4);
    }
}

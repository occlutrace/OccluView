use super::DVec3;

type Cell = (i32, i32, i32);
/// The inclusive cell range a triangle's bounding box covers. A triangle is
/// stored in every cell of its range, so the range is its cell list.
pub(super) type CellSpan = (Cell, Cell);

fn span_cells(span: CellSpan) -> impl Iterator<Item = Cell> {
    let (first, last) = span;
    (first.0..=last.0).flat_map(move |x| {
        (first.1..=last.1).flat_map(move |y| (first.2..=last.2).map(move |z| (x, y, z)))
    })
}

/// Triangle bucket grid for the session raycast (the brush must hit the
/// current deformed surface; JS-side raycasting a 300k-tri mesh per pointer
/// move was the drag lag). Cells stamped by triangle AABB; a 3D-DDA walk
/// tests only the cells along the ray.
pub(super) struct TriBuckets {
    pub(super) cell: f64,
    pub(super) lo: DVec3,
    /// Bounds of occupied cells, maintained without rebinning the mesh.
    pub(super) min_cell: Cell,
    pub(super) max_cell: Cell,
    pub(super) occupied_axes: [std::collections::BTreeMap<i32, usize>; 3],
    pub(super) map: crate::hash::FxHashMap<Cell, Vec<u32>>,
    /// Per triangle, the cell range it is stored under. Two corners, not a
    /// list: nothing is allocated per triangle.
    pub(super) triangle_cells: Vec<CellSpan>,

    pub(super) dropped_triangle_visits: usize,
}

impl TriBuckets {
    pub(super) fn build(verts: &[f32], tris: &[u32], cell: f64) -> TriBuckets {
        let mut lo = DVec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = DVec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let v = |i: u32| {
            let k = i as usize * 3;
            DVec3::new(verts[k] as f64, verts[k + 1] as f64, verts[k + 2] as f64)
        };
        for i in 0..verts.len() / 3 {
            let p = v(i as u32);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let mut grid = TriBuckets {
            cell,
            lo,
            min_cell: (0, 0, 0),
            max_cell: (0, 0, 0),
            occupied_axes: Default::default(),
            map: crate::hash::FxHashMap::default(),
            triangle_cells: Vec::with_capacity(tris.len() / 3),
            dropped_triangle_visits: 0,
        };
        for (ti, t) in tris.as_chunks::<3>().0.iter().enumerate() {
            let span = grid.span_for_points(v(t[0]), v(t[1]), v(t[2]));
            for key in span_cells(span) {
                grid.map.entry(key).or_default().push(ti as u32);
            }
            grid.triangle_cells.push(span);
        }
        let mut occupied_axes: [std::collections::BTreeMap<i32, usize>; 3] = Default::default();
        for &(x, y, z) in grid.map.keys() {
            for (axis, coordinate) in [x, y, z].into_iter().enumerate() {
                *occupied_axes[axis].entry(coordinate).or_default() += 1;
            }
        }
        grid.occupied_axes = occupied_axes;
        grid.refresh_occupied_bounds();
        grid
    }

    /// Room for `triangles` appended faces.
    pub(super) fn reserve_growth(&mut self, triangles: usize) {
        self.triangle_cells.reserve(triangles);
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

    pub(super) fn span_for_points(&self, a: DVec3, b: DVec3, c: DVec3) -> CellSpan {
        let low = a.min(b).min(c);
        let high = a.max(b).max(c);
        (
            (
                ((low.x - self.lo.x) / self.cell).floor() as i32,
                ((low.y - self.lo.y) / self.cell).floor() as i32,
                ((low.z - self.lo.z) / self.cell).floor() as i32,
            ),
            (
                ((high.x - self.lo.x) / self.cell).floor() as i32,
                ((high.y - self.lo.y) / self.cell).floor() as i32,
                ((high.z - self.lo.z) / self.cell).floor() as i32,
            ),
        )
    }

    fn span_of_triangle(&self, verts: &[f32], tris: &[u32], triangle: u32) -> CellSpan {
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

    pub(super) fn update_triangles(&mut self, verts: &[f32], tris: &[u32], triangles: &[u32]) {
        // Sorted lists, not hash sets: this runs once per step over the faces
        // the step touched, and a sorted probe allocates nothing per entry.
        let mut touched = triangles.to_vec();
        touched.sort_unstable();
        touched.dedup();
        let mut updates = Vec::with_capacity(touched.len());
        let mut affected_cells: Vec<Cell> = Vec::new();
        for &triangle in &touched {
            let next = self.span_of_triangle(verts, tris, triangle);
            affected_cells.extend(span_cells(self.triangle_cells[triangle as usize]));
            affected_cells.extend(span_cells(next));
            updates.push((triangle, next));
        }
        affected_cells.sort_unstable();
        affected_cells.dedup();

        // Thin geometry concentrates many triangles in the same 2 mm bucket.
        // Removing each changed triangle with a separate linear search made a
        // large Smooth footprint O(changed * bucket_size). Clear every affected
        // bucket once, then add the complete changed batch back in linear time.
        let previous_occupancy: Vec<_> = affected_cells
            .iter()
            .map(|&cell| (cell, self.map.contains_key(&cell)))
            .collect();
        for cell in &affected_cells {
            if let Some(entries) = self.map.get_mut(cell) {
                entries.retain(|triangle| touched.binary_search(triangle).is_err());
            }
        }
        for (triangle, next) in updates {
            for cell in span_cells(next) {
                self.map.entry(cell).or_default().push(triangle);
            }
            self.triangle_cells[triangle as usize] = next;
        }
        for (cell, was_occupied) in previous_occupancy {
            let occupied = self
                .map
                .get(&cell)
                .is_some_and(|entries| !entries.is_empty());
            if !occupied {
                self.map.remove(&cell);
            }
            if occupied != was_occupied {
                self.note_occupancy(cell, occupied);
            }
        }
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
        for index in first_new as usize..count {
            let triangle = index as u32;
            let span = self.span_of_triangle(verts, tris, triangle);
            for cell in span_cells(span) {
                let occupied = self
                    .map
                    .get(&cell)
                    .is_some_and(|entries| !entries.is_empty());
                self.map.entry(cell).or_default().push(triangle);
                if !occupied {
                    self.note_occupancy(cell, true);
                }
            }
            self.triangle_cells.push(span);
        }
        self.refresh_occupied_bounds();
    }

    /// Forget every triangle at or past `from_id` (topology undo). The caller
    /// truncates its own index buffer afterwards.
    pub(super) fn drop_triangles(&mut self, from_id: u32) {
        {
            self.dropped_triangle_visits = self
                .dropped_triangle_visits
                .saturating_add(self.triangle_cells.len().saturating_sub(from_id as usize));
        }
        for index in from_id as usize..self.triangle_cells.len() {
            let triangle = index as u32;
            for cell in span_cells(self.triangle_cells[index]) {
                let mut became_empty = false;
                if let Some(entries) = self.map.get_mut(&cell) {
                    entries.retain(|&candidate| candidate != triangle);
                    became_empty = entries.is_empty();
                }
                if became_empty {
                    self.map.remove(&cell);
                    self.note_occupancy(cell, false);
                }
            }
        }
        self.triangle_cells.truncate(from_id as usize);
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

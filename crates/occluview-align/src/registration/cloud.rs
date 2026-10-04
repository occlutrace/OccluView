//! Even point clouds of a surface and bounded nearest-point lookup in them.
//!
//! A scan's triangle density says nothing about its shape, so every stage of
//! the search reads the surface through representatives spread evenly over
//! its area: one per occupied grid cell and facing. The cell order is the
//! order in which the source triangles first reach each cell, so equal input
//! gives an equal cloud.

use glam::DVec3;
use std::collections::HashMap;

/// Largest number of lookup cells; a larger extent gets larger cells.
const MAX_GRID_CELLS: usize = 4_000_000;
/// A triangle is cut into at most this many pieces along each edge.
const MAX_SPLIT: usize = 64;
/// Double areas at or below this are not surface.
const MIN_DOUBLE_AREA: f64 = 1e-12;

/// One representative of a patch of surface.
#[derive(Clone, Copy, Debug)]
pub(super) struct Point {
    /// Area-weighted centre of the patch.
    pub position: DVec3,
    /// Unit area-weighted normal of the patch.
    pub normal: DVec3,
    /// Area the patch stands for, in square millimetres.
    pub area: f64,
    /// A point of the patch that lies on the source surface itself.
    pub anchor: DVec3,
}

#[derive(Clone, Copy, Default)]
struct Bin {
    position: DVec3,
    normal: DVec3,
    area: f64,
    anchor: DVec3,
    anchor_area: f64,
}

impl Bin {
    fn add(&mut self, position: DVec3, normal: DVec3, area: f64, anchor: DVec3) {
        self.position += position * area;
        self.normal += normal * area;
        self.area += area;
        if area > self.anchor_area {
            self.anchor = anchor;
            self.anchor_area = area;
        }
    }

    fn point(&self) -> Option<Point> {
        let normal = self.normal.try_normalize()?;
        (self.area > 0.).then(|| Point {
            position: self.position / self.area,
            normal,
            area: self.area,
            anchor: self.anchor,
        })
    }
}

struct Cell {
    /// Normal of the first patch that reached the cell.
    facing: DVec3,
    /// Surface facing the same way as `facing`, and surface facing against
    /// it. A thin shell keeps its two sides apart.
    sides: [Bin; 2],
}

/// Collects surface into cells of one spacing.
pub(super) struct Gather {
    spacing: f64,
    slots: HashMap<[i32; 3], u32>,
    cells: Vec<Cell>,
    area: f64,
}

impl Gather {
    pub(super) fn new(spacing: f64) -> Self {
        Self {
            spacing,
            slots: HashMap::new(),
            cells: Vec::new(),
            area: 0.,
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn key(&self, position: DVec3) -> Option<[i32; 3]> {
        let scaled = position / self.spacing;
        (scaled.is_finite() && scaled.abs().max_element() < 1e9).then(|| {
            [
                scaled.x.floor() as i32,
                scaled.y.floor() as i32,
                scaled.z.floor() as i32,
            ]
        })
    }

    fn add(&mut self, position: DVec3, normal: DVec3, area: f64, anchor: DVec3) {
        let Some(key) = self.key(position) else {
            return;
        };
        let cells = &mut self.cells;
        let slot = *self.slots.entry(key).or_insert_with(|| {
            cells.push(Cell {
                facing: normal,
                sides: [Bin::default(); 2],
            });
            u32::try_from(cells.len() - 1).unwrap_or(u32::MAX)
        });
        let Some(cell) = cells.get_mut(slot as usize) else {
            return;
        };
        let side = usize::from(normal.dot(cell.facing) < 0.);
        cell.sides[side].add(position, normal, area, anchor);
        self.area += area;
    }

    /// Add one triangle, cut so that no piece is longer than the spacing.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub(super) fn add_triangle(&mut self, [apex, left, right]: [DVec3; 3]) {
        let (along, across) = (left - apex, right - apex);
        let cross = along.cross(across);
        let double_area = cross.length();
        if !(double_area.is_finite() && double_area > MIN_DOUBLE_AREA) {
            return;
        }
        let normal = cross / double_area;
        let area = double_area * 0.5;
        let longest = along
            .length()
            .max(across.length())
            .max((right - left).length());
        let split = ((longest / self.spacing).ceil() as usize).clamp(1, MAX_SPLIT);
        let step = 1. / split as f64;
        let share = area * step * step;
        // The centre of the piece whose corner nearest the apex is `row`
        // steps along one edge and `column` steps along the other; `thirds`
        // is one for a piece that points the way the triangle does and two
        // for one that points back.
        let centre = |row: usize, column: usize, thirds: f64| {
            apex + along * ((row as f64 + thirds / 3.) * step)
                + across * ((column as f64 + thirds / 3.) * step)
        };
        for row in 0..split {
            for column in 0..split - row {
                let upright = centre(row, column, 1.);
                self.add(upright, normal, share, upright);
                if column + 1 < split - row {
                    let inverted = centre(row, column, 2.);
                    self.add(inverted, normal, share, inverted);
                }
            }
        }
    }

    /// Total area added, in square millimetres.
    pub(super) fn area(&self) -> f64 {
        self.area
    }

    pub(super) fn finish(self) -> Cloud {
        let points: Vec<Point> = self
            .cells
            .iter()
            .flat_map(|cell| cell.sides.iter().filter_map(Bin::point))
            .collect();
        Cloud::new(points, self.spacing)
    }
}

/// An even cloud and a grid over it.
pub(super) struct Cloud {
    pub points: Vec<Point>,
    pub spacing: f64,
    min: DVec3,
    max: DVec3,
    cell: f64,
    dims: [usize; 3],
    /// Start of every cell's run in `order`, and one past the last.
    starts: Vec<u32>,
    /// Point ordinals grouped by cell, ascending inside a cell.
    order: Vec<u32>,
}

impl Cloud {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub(super) fn new(points: Vec<Point>, spacing: f64) -> Self {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for point in &points {
            min = min.min(point.position);
            max = max.max(point.position);
        }
        if points.is_empty() {
            (min, max) = (DVec3::ZERO, DVec3::ZERO);
        }
        let mut cell = spacing.max(1e-6) * 2.;
        let dims_for = |cell: f64| {
            let extent = (max - min) / cell;
            [extent.x, extent.y, extent.z].map(|e| (e.floor() as usize).saturating_add(1))
        };
        let mut dims = dims_for(cell);
        while dims
            .iter()
            .try_fold(1usize, |total, &d| total.checked_mul(d))
            .is_none_or(|total| total > MAX_GRID_CELLS)
        {
            cell *= 1.5;
            dims = dims_for(cell);
        }
        let mut cloud = Self {
            points,
            spacing,
            min,
            max,
            cell,
            dims,
            starts: Vec::new(),
            order: Vec::new(),
        };
        let total = dims[0] * dims[1] * dims[2];
        let mut counts = vec![0u32; total + 1];
        let homes: Vec<usize> = cloud
            .points
            .iter()
            .map(|point| {
                let [x, y, z] = cloud.home(point.position);
                cloud.slot([x.max(0) as usize, y.max(0) as usize, z.max(0) as usize])
            })
            .collect();
        for &home in &homes {
            counts[home + 1] += 1;
        }
        for slot in 0..total {
            counts[slot + 1] += counts[slot];
        }
        let mut next = counts.clone();
        let mut order = vec![0u32; cloud.points.len()];
        for (ordinal, &home) in homes.iter().enumerate() {
            order[next[home] as usize] = u32::try_from(ordinal).unwrap_or(u32::MAX);
            next[home] += 1;
        }
        cloud.starts = counts;
        cloud.order = order;
        cloud
    }

    /// The same surface at a coarser spacing.
    pub(super) fn coarser(&self, spacing: f64) -> Self {
        let mut gather = Gather::new(spacing);
        for point in &self.points {
            gather.add(point.position, point.normal, point.area, point.anchor);
        }
        gather.finish()
    }

    /// Opposite corners of the box around the cloud's points.
    pub(super) fn bounds(&self) -> (DVec3, DVec3) {
        (self.min, self.max)
    }

    /// Total area the cloud stands for.
    #[cfg(test)]
    pub(super) fn area(&self) -> f64 {
        self.points.iter().map(|point| point.area).sum()
    }

    #[allow(clippy::cast_possible_truncation)]
    fn home(&self, position: DVec3) -> [i64; 3] {
        let scaled = (position - self.min) / self.cell;
        [scaled.x, scaled.y, scaled.z].map(|value| value.floor().clamp(-1e12, 1e12) as i64)
    }

    fn slot(&self, [x, y, z]: [usize; 3]) -> usize {
        (z.min(self.dims[2] - 1) * self.dims[1] + y.min(self.dims[1] - 1)) * self.dims[0]
            + x.min(self.dims[0] - 1)
    }

    /// The members of one cell, or nothing outside the grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn members(&self, [x, y, z]: [i64; 3]) -> &[u32] {
        let inside = |value: i64, size: usize| value >= 0 && (value as usize) < size;
        if !(inside(x, self.dims[0]) && inside(y, self.dims[1]) && inside(z, self.dims[2])) {
            return &[];
        }
        let slot = self.slot([x as usize, y as usize, z as usize]);
        &self.order[self.starts[slot] as usize..self.starts[slot + 1] as usize]
    }

    fn outside(&self, position: DVec3, radius: f64) -> bool {
        self.points.is_empty()
            || !position.is_finite()
            || position.cmplt(self.min - radius).any()
            || position.cmpgt(self.max + radius).any()
    }

    /// The nearest representative within `radius`, with its squared distance.
    /// Equal distances resolve to the lower ordinal.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub(super) fn nearest(&self, position: DVec3, radius: f64) -> Option<(u32, f64)> {
        if self.outside(position, radius) {
            return None;
        }
        let home = self.home(position);
        let reach = (radius / self.cell).ceil() as i64;
        let limit = radius * radius;
        let mut best: Option<(u32, f64)> = None;
        for ring in 0..=reach {
            for dz in -ring..=ring {
                for dy in -ring..=ring {
                    let shell = dz.abs() == ring || dy.abs() == ring;
                    let mut visit = |dx: i64| {
                        for &ordinal in self.members([home[0] + dx, home[1] + dy, home[2] + dz]) {
                            let distance = self.points[ordinal as usize]
                                .position
                                .distance_squared(position);
                            if distance <= limit
                                && best.is_none_or(|(other, found)| {
                                    distance.total_cmp(&found).then(ordinal.cmp(&other)).is_lt()
                                })
                            {
                                best = Some((ordinal, distance));
                            }
                        }
                    };
                    if shell {
                        (-ring..=ring).for_each(&mut visit);
                    } else {
                        visit(-ring);
                        visit(ring);
                    }
                }
            }
            // Every point not yet visited is at least `ring` cells away.
            let safe = ring as f64 * self.cell;
            if best.is_some_and(|(_, distance)| distance <= safe * safe) {
                break;
            }
        }
        best
    }

    /// Every representative within `radius`, with its squared distance.
    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn within(&self, position: DVec3, radius: f64, mut visit: impl FnMut(u32, f64)) {
        if self.outside(position, radius) {
            return;
        }
        let low = self.home(position - radius);
        let high = self.home(position + radius);
        let limit = radius * radius;
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    for &ordinal in self.members([x, y, z]) {
                        let distance = self.points[ordinal as usize]
                            .position
                            .distance_squared(position);
                        if distance <= limit {
                            visit(ordinal, distance);
                        }
                    }
                }
            }
        }
    }

    /// Distance from `position` to the surface the cloud stands for, read as
    /// the offset from the nearest representative's plane, with that
    /// representative. Absent when the nearest representative is farther than
    /// `radius`, or the point lies beside the patch instead of over it.
    pub(super) fn surface_distance(&self, position: DVec3, radius: f64) -> Option<(u32, f64)> {
        let (ordinal, squared) = self.nearest(position, radius + self.spacing)?;
        let point = &self.points[ordinal as usize];
        let offset = (position - point.position).dot(point.normal).abs();
        let beside = (squared - offset * offset).max(0.).sqrt();
        (beside <= self.spacing * 1.5 && offset <= radius).then_some((ordinal, offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 x 10 mm square in the plane z = 0, as quads of half a millimetre.
    fn sheet(spacing: f64) -> Cloud {
        let mut gather = Gather::new(spacing);
        let at = |i: u32, j: u32| DVec3::new(f64::from(i) * 0.5, f64::from(j) * 0.5, 0.);
        for i in 0..20 {
            for j in 0..20 {
                gather.add_triangle([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                gather.add_triangle([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        gather.finish()
    }

    #[test]
    fn a_large_triangle_is_cut_and_keeps_its_area() {
        let mut gather = Gather::new(1.);
        gather.add_triangle([
            DVec3::new(0., 0., 0.),
            DVec3::new(10., 0., 0.),
            DVec3::new(10., 10., 0.),
        ]);
        assert!((gather.area() - 50.).abs() < 1e-9);
        let cloud = gather.finish();
        assert!((cloud.area() - 50.).abs() < 1e-9);
        // No piece of surface is farther than one cell from a representative.
        assert!(cloud.points.len() >= 45, "{}", cloud.points.len());
        assert!(cloud.points.iter().all(|point| point.area < 1.5));
    }

    #[test]
    fn a_cloud_keeps_the_area_and_spreads_it_evenly() {
        let cloud = sheet(1.);
        assert!((cloud.area() - 100.).abs() < 1e-9);
        assert_eq!(cloud.points.len(), 100);
        for point in &cloud.points {
            assert!((point.area - 1.).abs() < 1e-9, "{}", point.area);
            assert!((point.normal - DVec3::Z).length() < 1e-12);
            assert!(point.anchor.z.abs() < 1e-12);
        }
    }

    #[test]
    fn the_two_sides_of_a_thin_shell_stay_apart() {
        let mut gather = Gather::new(1.);
        let (a, b, c) = (
            DVec3::new(0., 0., 0.2),
            DVec3::new(0.9, 0., 0.2),
            DVec3::new(0., 0.9, 0.2),
        );
        gather.add_triangle([a, b, c]);
        let down = DVec3::new(0., 0., 0.4);
        gather.add_triangle([a + down, c + down, b + down]);
        let cloud = gather.finish();
        assert_eq!(cloud.points.len(), 2);
        assert!(cloud.points[0].normal.dot(cloud.points[1].normal) < -0.99);
    }

    #[test]
    fn nearest_agrees_with_a_full_scan() {
        let cloud = sheet(0.5);
        for (index, query) in [
            DVec3::new(3.3, 4.1, 0.7),
            DVec3::new(-2., 5., 0.),
            DVec3::new(9.9, 9.9, -1.5),
            DVec3::new(30., 30., 30.),
        ]
        .into_iter()
        .enumerate()
        {
            let radius = 3.;
            let expected = cloud
                .points
                .iter()
                .enumerate()
                .map(|(ordinal, point)| (ordinal, point.position.distance_squared(query)))
                .filter(|(_, distance)| *distance <= radius * radius)
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let found = cloud.nearest(query, radius);
            assert_eq!(
                found.map(|(ordinal, _)| ordinal as usize),
                expected.map(|(ordinal, _)| ordinal),
                "query {index}"
            );
            let mut counted = 0;
            cloud.within(query, radius, |_, _| counted += 1);
            let brute = cloud
                .points
                .iter()
                .filter(|point| point.position.distance_squared(query) <= radius * radius)
                .count();
            assert_eq!(counted, brute, "query {index}");
        }
    }

    #[test]
    fn surface_distance_reads_the_offset_over_the_patch_only() {
        let cloud = sheet(0.5);
        let over = cloud.surface_distance(DVec3::new(5.1, 5.2, 0.3), 0.5);
        assert!(over.is_some_and(|(_, offset)| (offset - 0.3).abs() < 1e-9));
        assert!(cloud
            .surface_distance(DVec3::new(12., 5., 0.1), 0.5)
            .is_none());
        assert!(cloud
            .surface_distance(DVec3::new(5., 5., 0.8), 0.5)
            .is_none());
    }

    #[test]
    fn a_coarser_cloud_stands_for_the_same_area() {
        let fine = sheet(0.25);
        let coarse = fine.coarser(1.);
        assert!((coarse.area() - fine.area()).abs() < 1e-9);
        assert_eq!(coarse.points.len(), 100);
    }
}

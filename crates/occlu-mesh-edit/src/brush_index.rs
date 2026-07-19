//! Uniform-grid spatial index over vertex positions, for the brush-radius
//! queries a freeform stroke needs every frame of an interactive drag.
//!
//! Built once when a [`super::brush::BrushSession`] is prepared (rayon-
//! parallel bucket assignment), then rebuilt with cell size matched to the
//! current brush radius on a deliberate size change — keeping a radius
//! query's cell scan bounded regardless of brush size vs. mesh scale.

use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;

/// Integer bucket coordinates for one grid cell.
type CellKey = (i32, i32, i32);

/// Grid resolution as a fraction of the mesh's bounding-box diagonal: small
/// enough that a few-millimeter brush on a dental arch (bbox tens of mm) still
/// visits only a handful of cells per query; large enough to avoid an
/// unbounded query on a huge or tiny mesh.
const CELLS_ACROSS_DIAGONAL: f32 = 96.0;

/// Largest per-axis cell reach a radius query will scan before falling back
/// to a linear pass over every occupied cell. Cell size is fixed to mesh
/// scale, so a brush far larger than the mesh would otherwise make the
/// `(2·reach+1)³` triple loop explode into millions of lookups and freeze the
/// UI. Past this reach, returning every vertex (a valid conservative
/// superset) is correct and bounded by vertex count.
const MAX_NEIGHBORHOOD_REACH: i32 = 16;

/// A uniform-grid spatial index over a fixed set of vertex positions,
/// captured at build time. A session moving vertices during strokes must
/// rebuild before drift changes cell membership relative to the radii in use
/// (see [`super::brush::BrushSession`] for the rebuild cadence it uses).
pub(crate) struct VertexGrid {
    cell_size: f32,
    origin: Vec3,
    cells: HashMap<CellKey, Vec<u32>>,
}

impl VertexGrid {
    /// Build the index over `positions` with a cell size derived from the
    /// mesh's own scale (`vertex id = array index`, truncated to `u32` —
    /// mesh-edit vertex counts never approach `u32::MAX`).
    pub(crate) fn build(positions: &[Vec3]) -> Self {
        let (lo, hi) = bounds(positions);
        let diagonal = (hi - lo).length();
        let cell_size = if diagonal.is_finite() && diagonal > f32::EPSILON {
            diagonal / CELLS_ACROSS_DIAGONAL
        } else {
            1.0
        };
        Self::build_with_cell_size(positions, cell_size)
    }

    /// Build the index with an EXPLICIT cell size, so a session can match the
    /// grid to the current brush radius (bounding `reach`, hence the query's
    /// cell-scan cost, regardless of brush size vs. mesh scale). A tiny brush
    /// on a huge scan and a huge brush on a small crop both stay cheap.
    pub(crate) fn build_with_cell_size(positions: &[Vec3], cell_size: f32) -> Self {
        let (lo, _hi) = bounds(positions);
        let cell_size = if cell_size.is_finite() && cell_size > f32::EPSILON {
            cell_size
        } else {
            1.0
        };
        let origin = lo;

        // Parallel bucket-key computation, then a single-threaded fold into
        // the map: `HashMap` insertion isn't safely shareable across threads,
        // but computing the (potentially expensive-at-scale) key per vertex
        // is embarrassingly parallel.
        let keyed: Vec<(CellKey, u32)> = positions
            .par_iter()
            .enumerate()
            .map(|(index, &position)| {
                #[allow(clippy::cast_possible_truncation)]
                let vertex_id = index as u32;
                (cell_key(position, origin, cell_size), vertex_id)
            })
            .collect();

        let mut cells: HashMap<CellKey, Vec<u32>> = HashMap::with_capacity(positions.len());
        for (key, vertex_id) in keyed {
            cells.entry(key).or_default().push(vertex_id);
        }

        Self {
            cell_size,
            origin,
            cells,
        }
    }

    /// Move `vertex_id` from the cell of `from` to the cell of `to`, if they
    /// differ. Keeps the index exact as a stroke moves vertices — O(touched)
    /// per dab — instead of periodically rebuilding the whole grid (O(n), the
    /// stall a big scan showed). A within-cell move is a near-free no-op.
    pub(crate) fn relocate(&mut self, vertex_id: usize, from: Vec3, to: Vec3) {
        let from_key = cell_key(from, self.origin, self.cell_size);
        let to_key = cell_key(to, self.origin, self.cell_size);
        if from_key == to_key {
            return;
        }
        #[allow(clippy::cast_possible_truncation)]
        let id = vertex_id as u32;
        if let Some(bucket) = self.cells.get_mut(&from_key) {
            if let Some(index) = bucket.iter().position(|&existing| existing == id) {
                bucket.swap_remove(index);
            }
        }
        self.cells.entry(to_key).or_default().push(id);
    }

    /// Every vertex id within `radius` of `center` (by cell coverage — a
    /// conservative superset; callers filter by exact distance).
    /// Deterministic without sorting: each vertex lives in one cell, and the
    /// scan visits cells in a fixed `(dx, dy, dz)` order (sorting per dab was
    /// a real cost on a big brush). The rare radius-dwarfs-the-grid fallback
    /// still sorts, since it walks the hash map in unspecified order.
    pub(crate) fn query_radius(&self, center: Vec3, radius: f32) -> Vec<usize> {
        if !(radius.is_finite() && radius > 0.0) {
            return Vec::new();
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let reach = (radius / self.cell_size).ceil() as i32 + 1;
        if reach > MAX_NEIGHBORHOOD_REACH {
            // The radius dwarfs the grid: one linear pass over every occupied
            // cell (O(vertices)) beats an O(reach³) neighborhood scan and can
            // never freeze. The caller filters by exact distance anyway.
            let mut found: Vec<usize> = self
                .cells
                .values()
                .flat_map(|bucket| bucket.iter().map(|&id| id as usize))
                .collect();
            found.sort_unstable();
            found.dedup();
            return found;
        }
        let center_key = cell_key(center, self.origin, self.cell_size);
        let mut found = Vec::new();
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    let key = (center_key.0 + dx, center_key.1 + dy, center_key.2 + dz);
                    if let Some(bucket) = self.cells.get(&key) {
                        found.extend(bucket.iter().map(|&id| id as usize));
                    }
                }
            }
        }
        found
    }
}

fn cell_key(position: Vec3, origin: Vec3, cell_size: f32) -> CellKey {
    let relative = (position - origin) / cell_size;
    #[allow(clippy::cast_possible_truncation)]
    let floor = |value: f32| -> i32 {
        if !value.is_finite() {
            return 0;
        }
        value
            .floor()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i32
    };
    (floor(relative.x), floor(relative.y), floor(relative.z))
}

fn bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for &position in positions {
        if position.is_finite() {
            lo = lo.min(position);
            hi = hi.max(position);
        }
    }
    if lo.x > hi.x {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_of_points(n: usize, spacing: f32) -> Vec<Vec3> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let f = i as f32;
                Vec3::new(f * spacing, 0.0, 0.0)
            })
            .collect()
    }

    #[test]
    fn query_radius_finds_exactly_the_points_inside_the_sphere() {
        // 21 points spaced 1mm apart on a line; center a 3mm-radius query at
        // the midpoint and check against a brute-force reference.
        let positions = grid_of_points(21, 1.0);
        let grid = VertexGrid::build(&positions);
        let center = positions[10];
        let radius = 3.0;

        let mut found = grid.query_radius(center, radius);
        found.sort_unstable();
        let mut expected: Vec<usize> = positions
            .iter()
            .enumerate()
            .filter(|(_, &p)| p.distance(center) <= radius)
            .map(|(i, _)| i)
            .collect();
        expected.sort_unstable();

        assert_eq!(found, expected);
    }

    #[test]
    fn query_radius_is_a_conservative_superset_never_missing_a_true_hit() {
        // A less regular layout: verify every brute-force hit is present in
        // the grid's candidate set (the grid may over-include; it must never
        // under-include).
        let positions: Vec<Vec3> = (0..500)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 * 0.37;
                Vec3::new(t.sin() * 10.0, t.cos() * 10.0, (t * 0.5).sin() * 4.0)
            })
            .collect();
        let grid = VertexGrid::build(&positions);
        let center = Vec3::new(2.0, -1.0, 0.5);
        let radius = 2.5;

        let found: std::collections::HashSet<usize> =
            grid.query_radius(center, radius).into_iter().collect();
        for (index, &position) in positions.iter().enumerate() {
            if position.distance(center) <= radius {
                assert!(
                    found.contains(&index),
                    "grid must not miss true hit {index} at distance {}",
                    position.distance(center)
                );
            }
        }
    }

    #[test]
    fn empty_input_never_panics() {
        let grid = VertexGrid::build(&[]);
        assert!(grid.query_radius(Vec3::ZERO, 1.0).is_empty());
    }

    #[test]
    fn degenerate_and_non_finite_positions_do_not_panic() {
        let positions = vec![
            Vec3::ZERO,
            Vec3::new(f32::NAN, 0.0, 0.0),
            Vec3::new(f32::INFINITY, 0.0, 0.0),
            Vec3::ZERO,
        ];
        let grid = VertexGrid::build(&positions);
        let found = grid.query_radius(Vec3::ZERO, 1.0);
        assert!(found.contains(&0));
        assert!(found.contains(&3));
    }

    #[test]
    fn zero_or_negative_radius_returns_empty() {
        let positions = grid_of_points(5, 1.0);
        let grid = VertexGrid::build(&positions);
        assert!(grid.query_radius(Vec3::ZERO, 0.0).is_empty());
        assert!(grid.query_radius(Vec3::ZERO, -1.0).is_empty());
    }

    #[test]
    fn a_radius_dwarfing_a_small_object_falls_back_instead_of_exploding() {
        // A tiny object (bbox ~0.06mm) queried with a huge relative radius:
        // cell size is fixed to mesh scale, so a neighborhood scan would be
        // O(reach^3) with reach in the hundreds and freeze without the cap.
        let positions: Vec<Vec3> = (0..20)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let f = i as f32;
                Vec3::new(f * 0.003, (f * 0.7).sin() * 0.02, 0.0)
            })
            .collect();
        let grid = VertexGrid::build(&positions);
        // radius ~8x the bbox diagonal — the pathological "big brush, small
        // object" case. Must complete instantly and include every point.
        let found = grid.query_radius(Vec3::ZERO, 0.5);
        assert_eq!(found.len(), positions.len());
        for id in 0..positions.len() {
            assert!(found.contains(&id));
        }
    }
}

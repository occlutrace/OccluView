//! Uniform-grid spatial index over vertex positions, for the brush-radius
//! queries a freeform stroke needs every frame of an interactive drag.
//!
//! Built ONCE per [`super::brush::BrushSession`] (bucket assignment is
//! rayon-parallel: the one-time O(n) cost this pays off against is scanning
//! every vertex of a multi-hundred-thousand-triangle scan on every stroke).
//! Cell size is derived from the mesh's own scale, not the brush radius, so a
//! session survives the operator resizing the brush without a rebuild.

use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;

/// Integer bucket coordinates for one grid cell.
type CellKey = (i32, i32, i32);

/// Grid resolution as a fraction of the mesh's bounding-box diagonal. Small
/// enough that a typical few-millimeter brush on a dental arch (bbox diagonal
/// tens of mm) still only visits a handful of cells per query; large enough
/// that a session never needs an unbounded query on a huge or tiny mesh.
const CELLS_ACROSS_DIAGONAL: f32 = 96.0;

/// A uniform-grid spatial index over a fixed set of vertex positions.
/// Positions are captured at build time; a session that moves vertices during
/// strokes MUST rebuild the grid before the moved region can drift far enough
/// to change cell membership relative to the brush radii in use (see
/// [`super::brush::BrushSession`] for the rebuild cadence it actually uses).
pub(crate) struct VertexGrid {
    cell_size: f32,
    origin: Vec3,
    cells: HashMap<CellKey, Vec<u32>>,
}

impl VertexGrid {
    /// Build the index over `positions` (vertex id = array index, truncated to
    /// `u32` — mesh-edit vertex counts never approach `u32::MAX`).
    pub(crate) fn build(positions: &[Vec3]) -> Self {
        let (lo, hi) = bounds(positions);
        let diagonal = (hi - lo).length();
        let cell_size = if diagonal.is_finite() && diagonal > f32::EPSILON {
            diagonal / CELLS_ACROSS_DIAGONAL
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

    /// The grid's cell size, in mm — the unit a session measures vertex
    /// drift against to know when a rebuild is due (see the struct doc's
    /// rebuild-cadence contract).
    pub(crate) fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Every vertex id within `radius` of `center` (by cell coverage — a
    /// conservative superset; callers filter by exact distance). Deterministic
    /// ascending order.
    pub(crate) fn query_radius(&self, center: Vec3, radius: f32) -> Vec<usize> {
        if !(radius.is_finite() && radius > 0.0) {
            return Vec::new();
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let reach = (radius / self.cell_size).ceil() as i32 + 1;
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
        found.sort_unstable();
        found.dedup();
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

        let found = grid.query_radius(center, radius);
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
}

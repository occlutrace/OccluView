//! Tests for the nearest-surface index.
//!
//! The load-bearing one is [`the_walk_answers_exactly_as_a_full_scan_does`]:
//! the traversal is an optimisation over "test every triangle", and the only
//! thing that makes it safe to optimise further is a test that pins it to that
//! answer, tie-break included.

use super::{closest_point_on_triangle, SurfaceHit, SurfaceIndex};
use crate::Soup;
use glam::DVec3;

/// A flat `n` x `n` grid of quads on z = 0, spacing `step`, as a soup.
fn plane(n: usize, step: f64) -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::with_capacity((n + 1) * (n + 1) * 3);
    for j in 0..=n {
        for i in 0..=n {
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            {
                positions.push((i as f64 * step) as f32);
                positions.push((j as f64 * step) as f32);
                positions.push(0.0);
            }
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    let stride = u32::try_from(n + 1).unwrap();
    let span = u32::try_from(n).unwrap();
    for j in 0..span {
        for i in 0..span {
            let a = j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride]);
            indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
        }
    }
    (positions, indices)
}

fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

#[test]
fn nearest_on_a_plane_is_the_foot_of_the_perpendicular() {
    let (positions, indices) = plane(8, 1.0);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let hit = index.nearest(DVec3::new(3.3, 4.7, 2.5), 10.0).unwrap();
    assert!((hit.point.x - 3.3).abs() < 1e-6);
    assert!((hit.point.y - 4.7).abs() < 1e-6);
    assert!(hit.point.z.abs() < 1e-6);
}

#[test]
fn the_normal_is_the_geometric_plane_normal() {
    let (positions, indices) = plane(4, 1.0);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let hit = index.nearest(DVec3::new(1.5, 1.5, 3.0), 10.0).unwrap();
    assert!((hit.normal.dot(DVec3::Z).abs() - 1.0).abs() < 1e-9);
    assert!((hit.normal.length() - 1.0).abs() < 1e-9);
}

#[test]
fn nothing_is_returned_beyond_the_radius() {
    let (positions, indices) = plane(4, 1.0);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    assert!(index.nearest(DVec3::new(2.0, 2.0, 50.0), 1.0).is_none());
}

#[test]
fn a_query_beside_the_sheet_snaps_to_its_edge() {
    let (positions, indices) = plane(4, 1.0);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let hit = index.nearest(DVec3::new(-3.0, 2.0, 0.0), 10.0).unwrap();
    assert!(
        hit.point.x.abs() < 1e-6,
        "expected the x = 0 border, got {hit:?}"
    );
}

#[test]
fn build_refuses_empty_and_degenerate_input() {
    assert!(SurfaceIndex::build(soup(&[], &[])).is_none());
    let positions = [0.0; 9];
    let indices = [0, 1, 2];
    assert!(SurfaceIndex::build(soup(&positions, &indices)).is_none());
}

#[test]
fn build_ignores_out_of_range_and_non_finite_triangles() {
    let (mut positions, mut indices) = plane(4, 1.0);
    indices.extend_from_slice(&[9999, 10000, 10001]);
    let base = u32::try_from(positions.len() / 3).unwrap();
    positions.extend_from_slice(&[f32::NAN, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
    indices.extend_from_slice(&[base, base + 1, base + 2]);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    assert!(index.nearest(DVec3::new(1.5, 1.5, 1.0), 5.0).is_some());
}

#[test]
fn the_cell_size_follows_triangle_size() {
    let (coarse_positions, coarse_indices) = plane(4, 4.0);
    let (fine_positions, fine_indices) = plane(32, 0.25);
    let coarse = SurfaceIndex::build(soup(&coarse_positions, &coarse_indices)).unwrap();
    let fine = SurfaceIndex::build(soup(&fine_positions, &fine_indices)).unwrap();
    assert!(
        coarse.cell_size() > fine.cell_size() * 4.0,
        "coarse {} vs fine {}",
        coarse.cell_size(),
        fine.cell_size()
    );
    assert!(coarse.nearest(DVec3::new(6.0, 6.0, 1.0), 10.0).is_some());
    assert!(fine.nearest(DVec3::new(4.0, 4.0, 1.0), 10.0).is_some());
}

#[test]
fn repeated_builds_answer_identically() {
    let (positions, indices) = plane(16, 0.5);
    let first = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let second = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    for k in 0..50 {
        let q = DVec3::new(f64::from(k) * 0.17, f64::from(k) * 0.09, 0.4);
        let a = first.nearest(q, 5.0);
        let b = second.nearest(q, 5.0);
        assert_eq!(a.map(|hit| hit.triangle), b.map(|hit| hit.triangle));
        assert_eq!(
            a.map(|hit| hit.point.to_array()),
            b.map(|hit| hit.point.to_array())
        );
    }
}

/// Deterministic bit-mixer standing in for a random generator: the queries must
/// be scattered, but a failure has to reproduce on the next run and on another
/// machine.
struct Scatter(u64);

impl Scatter {
    fn new() -> Self {
        Self(0x2545_F491_4F6C_DD1D)
    }

    /// The next value in `[0, 1)`.
    #[allow(clippy::cast_precision_loss)]
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The next point inside the given box.
    fn point(&mut self, low: DVec3, high: DVec3) -> DVec3 {
        let unit = DVec3::new(self.unit(), self.unit(), self.unit());
        low + (high - low) * unit
    }
}

/// The answer a full scan over every triangle gives, with the query's own
/// tie-break. This is the definition the index must reproduce.
fn brute_nearest(index: &SurfaceIndex, point: DVec3, radius: f64) -> Option<SurfaceHit> {
    if !point.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let limit = radius * radius;
    let mut best: Option<(f64, u32, DVec3, usize)> = None;
    for (slot, corners) in index.corners.iter().enumerate() {
        let candidate = closest_point_on_triangle(point, corners[0], corners[1], corners[2]);
        let distance = (candidate - point).length_squared();
        if distance > limit {
            continue;
        }
        let source = index.sources[slot];
        let better = match best {
            None => true,
            Some((best_distance, best_source, _, _)) => {
                distance < best_distance || (distance == best_distance && source < best_source)
            }
        };
        if better {
            best = Some((distance, source, candidate, slot));
        }
    }
    best.map(|(_, triangle, point, slot)| SurfaceHit {
        point,
        normal: index.normals[slot],
        triangle,
    })
}

/// Compare the two answers on every field a caller can observe.
fn assert_same(index: &SurfaceIndex, point: DVec3, radius: f64) {
    let shape = |hit: SurfaceHit| (hit.triangle, hit.point.to_array(), hit.normal.to_array());
    assert_eq!(
        index.nearest(point, radius).map(shape),
        brute_nearest(index, point, radius).map(shape),
        "at {point:?} within {radius} mm"
    );
}

/// A mesh with everything the walk has to survive: even quads that share every
/// edge, a bumpy sheet with uneven triangle sizes, and two far-off slabs that
/// leave a wide empty gap between them and the rest.
fn awkward_mesh() -> (Vec<f32>, Vec<u32>) {
    let (mut positions, mut indices) = plane(11, 0.9);

    let mut scatter = Scatter::new();
    let base = u32::try_from(positions.len() / 3).unwrap();
    let side = 9u32;
    for j in 0..=side {
        for i in 0..=side {
            let step = 0.3 + f64::from(i) * 0.25;
            #[allow(clippy::cast_possible_truncation)]
            {
                positions.push((f64::from(i) * step) as f32);
                positions.push((f64::from(j) * 0.7) as f32);
                positions.push((2.5 + scatter.unit() * 1.5) as f32);
            }
        }
    }
    let stride = side + 1;
    for j in 0..side {
        for i in 0..side {
            let a = base + j * stride + i;
            indices.extend_from_slice(&[a, a + 1, a + stride]);
            indices.extend_from_slice(&[a + 1, a + stride + 1, a + stride]);
        }
    }

    // Two isolated slabs, far enough out that most of the grid between them is
    // empty: the case where a query has to prove a large volume holds nothing.
    for corner in [-14.0f32, 16.0] {
        let base = u32::try_from(positions.len() / 3).unwrap();
        positions.extend_from_slice(&[
            corner,
            corner,
            -6.0, //
            corner + 3.0,
            corner,
            -6.0, //
            corner + 3.0,
            corner + 3.0,
            -5.0, //
            corner,
            corner + 3.0,
            -5.0,
        ]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, indices)
}

#[test]
fn the_walk_answers_exactly_as_a_full_scan_does() {
    let (positions, indices) = awkward_mesh();
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let mut scatter = Scatter::new();

    // A box well wider than the mesh, so a good share of the queries start
    // outside it, and several radii, so some answers land beyond reach.
    let low = DVec3::new(-20.0, -20.0, -12.0);
    let high = DVec3::new(22.0, 22.0, 12.0);
    for _ in 0..1_500 {
        let point = scatter.point(low, high);
        for radius in [0.35, 1.0, 2.0, 5.0, 40.0] {
            assert_same(&index, point, radius);
        }
    }
}

#[test]
fn a_query_on_a_shared_edge_breaks_the_tie_the_same_way_a_full_scan_does() {
    let (positions, indices) = awkward_mesh();
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();

    // Grid vertices, edge midpoints, and the diagonal each quad is split on:
    // every one of these sits on geometry two or more triangles share, so the
    // answer is decided by the tie-break and nothing else.
    for j in 0..11 {
        for i in 0..11 {
            let x = f64::from(i) * 0.9;
            let y = f64::from(j) * 0.9;
            for offset in [
                DVec3::ZERO,
                DVec3::new(0.45, 0.0, 0.0),
                DVec3::new(0.0, 0.45, 0.0),
                DVec3::new(0.45, 0.45, 0.0),
            ] {
                let on_surface = DVec3::new(x, y, 0.0) + offset;
                for lift in [0.0, 0.4, -0.4] {
                    assert_same(&index, on_surface + DVec3::Z * lift, 2.0);
                }
            }
        }
    }
}

#[test]
fn queries_in_empty_space_agree_with_a_full_scan_too() {
    let (positions, indices) = awkward_mesh();
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let mut scatter = Scatter::new();

    // The gap between the slabs and the sheet: inside the mesh box, far from
    // any triangle. This is the query that has nothing to find and must still
    // agree — including on finding nothing.
    let low = DVec3::new(-13.0, -13.0, -5.5);
    let high = DVec3::new(15.0, 15.0, -1.0);
    let mut misses = 0;
    for _ in 0..600 {
        let point = scatter.point(low, high);
        for radius in [1.0, 5.0, 12.0] {
            assert_same(&index, point, radius);
        }
        if index.nearest(point, 1.0).is_none() {
            misses += 1;
        }
    }
    assert!(misses > 100, "the empty region was not empty: {misses}");
}

/// A 3-4-5 tiled sheet at height `z`. Every triangle's longest edge is exactly
/// 5 mm, which pins the grid's cell to exactly 10 mm and puts the cell walls on
/// round coordinates — the only way to write a test about what happens exactly
/// on a shell boundary.
fn sheet_345(z: f32, positions: &mut Vec<f32>, indices: &mut Vec<u32>) {
    let base = u32::try_from(positions.len() / 3).unwrap();
    let (columns, rows) = (4u32, 3u32);
    for row in 0..=rows {
        for column in 0..=columns {
            #[allow(clippy::cast_precision_loss)]
            positions.extend_from_slice(&[(column * 3) as f32, (row * 4) as f32, z]);
        }
    }
    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let corner = base + row * stride + column;
            indices.extend_from_slice(&[corner, corner + 1, corner + stride]);
            indices.extend_from_slice(&[corner + 1, corner + stride + 1, corner + stride]);
        }
    }
}

#[test]
fn a_tie_one_shell_further_out_still_wins_on_the_lower_index() {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    // The far sheet is built first, so it holds the lower triangle indices.
    sheet_345(30.0, &mut positions, &mut indices);
    let far_triangles = u32::try_from(indices.len() / 3).unwrap();
    sheet_345(0.0, &mut positions, &mut indices);

    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    assert!(
        (index.cell_size() - 10.0).abs() < 1e-12,
        "the fixture depends on a 10 mm cell, got {}",
        index.cell_size()
    );

    // Straight above a shared vertex of the near sheet and straight below one
    // of the far sheet: exactly 15 mm from both, and the closest point is that
    // vertex exactly, so the tie is a tie in binary too. The near sheet sits
    // one shell out, the far sheet two.
    let query = DVec3::new(3.0, 4.0, 15.0);
    let hit = index.nearest(query, 20.0).unwrap();
    assert!(
        hit.triangle < far_triangles,
        "the equal-distance triangle with the lower index must win, got {}",
        hit.triangle
    );
    assert_same(&index, query, 20.0);
}

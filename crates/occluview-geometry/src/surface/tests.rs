//! Tests for the nearest-surface index.
//!
//! The central one is [`the_walk_answers_exactly_as_a_full_scan_does`]: the
//! traversal is an optimisation over "test every triangle" and must return the
//! same answer, tie-break included.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::Soup;
use super::{closest_feature_on_triangle, Feature, SurfaceHit, SurfaceIndex};
use glam::DVec3;

#[test]
fn widely_separated_small_triangles_keep_a_bounded_surface_grid() {
    let positions = [
        0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0e30, 0.0, 0.0, 1.0e30, 1.0, 0.0, 1.0e30,
        0.0, 1.0,
    ];
    let indices = [0, 1, 2, 3, 4, 5];
    let index = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    })
    .expect("two finite triangles");
    assert_eq!(index.triangle_count(), 2);
    let hit = index
        .nearest(DVec3::new(0.1, 0.25, 0.25), 1.0)
        .expect("near triangle");
    assert_eq!(hit.triangle, 0);
    assert!((hit.point - DVec3::new(0.0, 0.25, 0.25)).length() < 1.0e-12);
}

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
fn surface_area_counts_only_unmasked_non_degenerate_triangles() {
    let (positions, indices) = plane(8, 1.0);
    let whole = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    assert!((whole.surface_area_mm2() - 64.0).abs() < 1e-9);

    let mut mask = vec![0; positions.len() / 3];
    for (vertex, excluded) in mask.iter_mut().enumerate() {
        if positions[vertex * 3] < 4.0 {
            *excluded = 1;
        }
    }
    let masked = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: Some(&mask),
    })
    .unwrap();
    assert!((masked.surface_area_mm2() - 32.0).abs() < 1e-9);

    let mut with_degenerate = positions.clone();
    let degenerate_vertex = u32::try_from(with_degenerate.len() / 3).unwrap();
    with_degenerate.extend_from_slice(&[100.0, 100.0, 0.0, 101.0, 100.0, 0.0, 102.0, 100.0, 0.0]);
    let mut with_degenerate_indices = indices;
    with_degenerate_indices.extend_from_slice(&[
        degenerate_vertex,
        degenerate_vertex + 1,
        degenerate_vertex + 2,
    ]);
    let degenerate = SurfaceIndex::build(soup(&with_degenerate, &with_degenerate_indices)).unwrap();
    assert!((degenerate.surface_area_mm2() - 64.0).abs() < 1e-9);
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
fn duplicated_triangle_soup_vertices_still_form_connected_components() {
    // Binary STL stores three fresh vertex records per facet. Adjacent facets
    // therefore share positions, not vertex ids; component discovery must weld
    // those exact positions before choosing a coarse Best Fit hypothesis.
    let positions = vec![
        0.0, 0.0, 0.0, // first triangle
        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, // adjacent triangle, all ids duplicated
        1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 10.0, 0.0, 0.0, // isolated triangle
        11.0, 0.0, 0.0, 10.0, 1.0, 0.0,
    ];
    let indices: Vec<u32> = (0..9).collect();
    let index = SurfaceIndex::build(soup(&positions, &indices)).expect("usable soup");

    assert_eq!(
        index.component_bounds().len(),
        2,
        "adjacent STL facets must not become separate pseudo-components"
    );
}

#[test]
fn representative_surface_samples_are_bounded_and_keep_normals() {
    let (positions, indices) = plane(4, 1.0);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();

    let samples = index.representative_samples(2);

    assert_eq!(samples.len(), 2);
    assert!(samples
        .iter()
        .all(|sample| sample.normal.dot(DVec3::Z) > 0.99));
    assert!(samples
        .iter()
        .all(|sample| sample.point.x >= 0.0 && sample.point.x <= 4.0));
}

#[test]
fn representative_samples_keep_component_ids_after_spatial_reordering() {
    // Put the first source component far to the right and the second one at
    // the origin. The spatial index reorders them by cell; the component tag
    // has to follow the same permutation or global Best Fit seeds inherit the
    // wrong ambiguity identity.
    let positions = vec![
        100.0, 0.0, 0.0, 101.0, 0.0, 0.0, 100.0, 1.0, 0.0, // far
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, // near
    ];
    let indices = vec![0, 1, 2, 3, 4, 5];
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let samples = index.representative_samples(2);

    assert_eq!(index.component_bounds().len(), 2);
    for sample in samples {
        let (low, high) = index.component_bounds()[sample.component];
        assert!(
            sample.point.x >= low.x - 1e-9 && sample.point.x <= high.x + 1e-9,
            "sample at x={} was tagged as component {} with bounds {low:?}..{high:?}",
            sample.point.x,
            sample.component,
        );
    }
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
#[allow(clippy::float_cmp)]
fn brute_nearest(index: &SurfaceIndex, point: DVec3, radius: f64) -> Option<SurfaceHit> {
    if !point.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let limit = radius * radius;
    let mut best: Option<(f64, u32, DVec3, usize, Feature)> = None;
    for (slot, corners) in index.corners.iter().enumerate() {
        let (candidate, feature) =
            closest_feature_on_triangle(point, corners[0], corners[1], corners[2]);
        let distance = (candidate - point).length_squared();
        if distance > limit {
            continue;
        }
        let source = index.sources[slot];
        let better = match best {
            None => true,
            Some((best_distance, best_source, _, _, _)) => {
                distance < best_distance || (distance == best_distance && source < best_source)
            }
        };
        if better {
            best = Some((distance, source, candidate, slot, feature));
        }
    }
    best.map(|(_, triangle, point, slot, feature)| index.hit(slot, triangle, point, feature))
}

/// Compare the two answers on every field a caller can observe.
fn assert_same(index: &SurfaceIndex, point: DVec3, radius: f64) {
    let shape = |hit: SurfaceHit| {
        (
            hit.triangle,
            hit.point.to_array(),
            hit.normal.to_array(),
            hit.pseudo_normal.to_array(),
            hit.on_border,
        )
    };
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

/// The fixed half of "Exclude selected parts".
///
/// The only reliable way to keep marked surface out of a match is to leave it out
/// of the index: a query that could still land on it would match against
/// geometry the operator has explicitly said not to use, and a deviation
/// measured to it would be a distance to a surface that is not in the
/// comparison. A triangle with any marked corner straddles the boundary, so the
/// whole triangle goes.
#[test]
fn marked_triangles_are_left_out_of_the_index_entirely() {
    let (positions, indices) = plane(8, 1.0);
    let vertex_count = positions.len() / 3;

    let whole = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    })
    .expect("a plane is a surface");

    // Mark out everything with x below 4, which is half the sheet.
    let mut mask = vec![0; vertex_count];
    for vertex in 0..vertex_count {
        if positions[vertex * 3] < 4.0 {
            mask[vertex] = 1;
        }
    }
    let masked = SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: Some(&mask),
    })
    .expect("the unmarked half is still a surface");

    // Over the marked half the whole index answers and the masked one does not.
    let over_marked = DVec3::new(1.0, 1.0, 2.0);
    assert!(
        whole.nearest(over_marked, 4.0).is_some(),
        "the unmasked index must still answer here"
    );
    assert!(
        masked.nearest(over_marked, 1.0).is_none(),
        "a query over marked surface must find nothing near it"
    );

    // Over the unmarked half both answer, and they agree.
    let over_kept = DVec3::new(6.0, 6.0, 2.0);
    let (Some(from_whole), Some(from_masked)) = (
        whole.nearest(over_kept, 4.0),
        masked.nearest(over_kept, 4.0),
    ) else {
        panic!("both indices must answer over surface neither excluded");
    };
    assert!(
        (from_whole.point - from_masked.point).length() < 1e-9,
        "masking one half must not move the other: {:?} vs {:?}",
        from_whole.point,
        from_masked.point
    );
}

/// A mask that marks everything leaves no surface at all, and that has to be
/// reported rather than answered with an empty index a query walks forever.
#[test]
fn marking_the_whole_mesh_leaves_no_index() {
    let (positions, indices) = plane(4, 1.0);
    let mask = vec![1; positions.len() / 3];
    assert!(SurfaceIndex::build(Soup {
        positions: &positions,
        indices: &indices,
        mask: Some(&mask),
    })
    .is_none());
}

/// Exact controlled queries must preserve unrestricted grid and brute answers.
#[test]
fn controlled_queries_agree_on_ten_thousand_probes() {
    use super::super::GeometryLimits;
    use super::{BuildOutcome, GeometryControl, QueryOutcome};
    let (positions, indices) = awkward_mesh();
    let mesh = soup(&positions, &indices);
    let unrestricted = SurfaceIndex::build(mesh).unwrap();
    let control = GeometryControl::new(
        super::super::CancelFlag::new(),
        std::time::Duration::from_secs(10),
        GeometryLimits::default(),
    );
    let BuildOutcome::Complete(index) =
        SurfaceIndex::build_controlled(mesh, glam::DAffine3::IDENTITY, &control)
    else {
        panic!("exact synthetic index");
    };
    for i in 0..10_000usize {
        let point = DVec3::new(
            super::radical_inverse(i + 1, 2) * 70. - 20.,
            super::radical_inverse(i + 1, 3) * 70. - 20.,
            super::radical_inverse(i + 1, 5) * 12. - 6.,
        );
        let radius = 0.1 + super::radical_inverse(i + 1, 7) * 15.;
        let QueryOutcome::Complete(actual) = index.nearest_controlled(point, radius, &control)
        else {
            panic!("bounded ordinary query");
        };
        let expected = unrestricted.nearest(point, radius);
        let brute = brute_nearest(&unrestricted, point, radius);
        assert_eq!(actual.map(|h| h.triangle), expected.map(|h| h.triangle));
        assert_eq!(actual.map(|h| h.on_border), expected.map(|h| h.on_border));
        assert_eq!(actual, brute);
        if let (Some(a), Some(b)) = (actual, expected) {
            assert!(a.point.distance(b.point) <= 1e-10);
        }
    }
    assert_eq!(control.counters().query_calls, 10_000);
}

/// ID35: a dense bucket's upper bound never becomes exact nearest evidence.
#[test]
fn deadline_and_work_caps_are_honest_in_dense_queries() {
    use super::super::{CancelFlag, GeometryLimits, GeometryStop};
    use super::{GeometryControl, QueryOutcome};
    let positions = [0., 0., 0., 10., 0., 0., 0., 10., 0.];
    let indices: Vec<u32> = (0..20_000).flat_map(|_| [0, 1, 2]).collect();
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let control = GeometryControl::new(
        CancelFlag::new(),
        std::time::Duration::from_secs(10),
        GeometryLimits::default(),
    );
    let result = index.nearest_controlled(DVec3::new(2., 2., 1.), 2., &control);
    assert!(matches!(
        result,
        QueryOutcome::Interrupted {
            best: Some(_),
            reason: GeometryStop::WorkLimit
        }
    ));
    assert_eq!(control.counters().triangle_tests, 16_384);
    assert_eq!(control.counters().query_calls, 1);
    let expired = GeometryControl::new(
        CancelFlag::new(),
        std::time::Duration::ZERO,
        GeometryLimits::default(),
    );
    assert!(matches!(
        index.nearest_controlled(DVec3::ZERO, 1., &expired),
        QueryOutcome::Interrupted {
            best: None,
            reason: GeometryStop::Deadline
        }
    ));
    assert_eq!(expired.counters().triangle_tests, 0);
    assert_eq!(index.nearest(DVec3::splat(f64::NAN), 1.), None);
}

/// ID35: fallible memory and topology/bucket work stop before exceeding caps.
#[test]
fn deadline_and_work_caps_are_honest_in_builds() {
    use super::super::{CancelFlag, GeometryLimits, GeometryStop};
    use super::{BuildOutcome, GeometryControl};
    let (positions, indices) = plane(50, 0.5);
    for (memory_bytes, operations, expected) in [
        (4096, 80_000_000, GeometryStop::ResourceLimit),
        (256 * 1024 * 1024, 5000, GeometryStop::WorkLimit),
    ] {
        let control = GeometryControl::new(
            CancelFlag::new(),
            std::time::Duration::from_secs(10),
            GeometryLimits {
                operations,
                memory_bytes,
                ..Default::default()
            },
        );
        assert!(
            matches!(SurfaceIndex::build_controlled(soup(&positions, &indices), glam::DAffine3::IDENTITY, &control), BuildOutcome::Partial { value: None, reason } if reason == expected)
        );
        assert!(control.counters().operations <= operations);
        assert!(control.counters().peak_memory_bytes <= memory_bytes as u64);
        assert_eq!(control.counters().memory_bytes, 0);
    }
}

/// ID34: scheduled cancellation reaches topology, buckets, occupancy and inner queries.
#[test]
fn cancel_every_stage_of_surface_index() {
    use super::super::{CancelFlag, GeometryLimits, GeometryStop};
    use super::{BuildOutcome, GeometryControl};
    use std::time::{Duration, Instant};
    let (positions, indices) = plane(90, 0.4);
    let baseline = GeometryControl::new(
        CancelFlag::new(),
        Duration::from_secs(10),
        GeometryLimits::default(),
    );
    let BuildOutcome::Complete(index) = SurfaceIndex::build_controlled(
        soup(&positions, &indices),
        glam::DAffine3::IDENTITY,
        &baseline,
    ) else {
        panic!("baseline index");
    };
    drop(index);
    let operations = baseline.counters().operations;
    for numerator in [1, 3, 6, 8, 9] {
        let flag = CancelFlag::new();
        let control = GeometryControl::new(
            flag.clone(),
            Duration::from_secs(10),
            GeometryLimits::default(),
        );
        let boundary = operations * numerator / 10;
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let echo = control.clone();
            scope.spawn(move || {
                let started = Instant::now();
                while echo.counters().operations < boundary
                    && started.elapsed() < Duration::from_secs(2)
                {
                    std::thread::yield_now();
                }
                let at = Instant::now();
                flag.cancel();
                sent.send(at).unwrap();
            });
            let result = SurfaceIndex::build_controlled(
                soup(&positions, &indices),
                glam::DAffine3::IDENTITY,
                &control,
            );
            let terminal = Instant::now();
            let cancelled = received.recv().unwrap();
            assert!(matches!(
                result,
                BuildOutcome::Partial {
                    value: None,
                    reason: GeometryStop::Cancelled
                }
            ));
            assert!(terminal.saturating_duration_since(cancelled) < Duration::from_millis(100));
        });
        assert_eq!(control.counters().memory_bytes, 0);
    }
    assert_inner_query_cancelled();
}

fn assert_inner_query_cancelled() {
    use super::super::{CancelFlag, GeometryLimits, GeometryStop};
    use super::{GeometryControl, QueryOutcome};
    use std::time::{Duration, Instant};
    let positions = [0., 0., 0., 10., 0., 0., 0., 10., 0.];
    let indices: Vec<u32> = (0..40_000).flat_map(|_| [0, 1, 2]).collect();
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    let flag = CancelFlag::new();
    let control = GeometryControl::new(
        flag.clone(),
        Duration::from_secs(10),
        GeometryLimits {
            single_query_tests: u64::MAX,
            ..Default::default()
        },
    );
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let echo = control.clone();
        scope.spawn(move || {
            let started = Instant::now();
            while echo.counters().triangle_tests < 128 && started.elapsed() < Duration::from_secs(2)
            {
                std::thread::yield_now();
            }
            let at = Instant::now();
            flag.cancel();
            sent.send(at).unwrap();
        });
        let result = index.nearest_controlled(DVec3::new(2., 2., 1.), 2., &control);
        let terminal = Instant::now();
        let cancelled = received.recv().unwrap();
        assert!(matches!(
            result,
            QueryOutcome::Interrupted {
                reason: GeometryStop::Cancelled,
                ..
            }
        ));
        assert!(terminal.saturating_duration_since(cancelled) < Duration::from_millis(100));
    });
}

/// Overlapping buckets cannot consume the distance budget by retesting a small
/// surface. This also checks tie/border/normal equality with the brute oracle.
#[test]
fn overlapping_buckets_reuse_distances_without_changing_exact_hits() {
    use super::super::{CancelFlag, GeometryLimits};
    use super::{GeometryControl, QueryOutcome};
    let (positions, indices) = plane(6, 1.);
    let index = SurfaceIndex::build(soup(&positions, &indices)).unwrap();
    for lift in [0., 0.5, 2.] {
        let point = DVec3::new(index.cell_size(), index.cell_size(), lift);
        let control = GeometryControl::new(
            CancelFlag::new(),
            std::time::Duration::from_secs(10),
            GeometryLimits {
                triangle_tests: 72,
                single_query_tests: 72,
                ..GeometryLimits::default()
            },
        );
        let actual = index.nearest_controlled(point, 5., &control);
        assert_eq!(
            actual,
            QueryOutcome::Complete(brute_nearest(&index, point, 5.))
        );
        assert!(control.counters().triangle_tests <= 72);
    }
}

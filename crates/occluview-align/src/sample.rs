//! Deterministic sampling and vertex normals shared by the refine and
//! deviation stages.

use glam::DVec3;

use crate::Soup;

/// Read one vertex position, or `None` when it is out of range or not finite.
pub(crate) fn vertex_at(positions: &[f32], vertex: usize) -> Option<DVec3> {
    let xyz = positions.get(vertex * 3..vertex * 3 + 3)?;
    let point = DVec3::new(f64::from(xyz[0]), f64::from(xyz[1]), f64::from(xyz[2]));
    point.is_finite().then_some(point)
}

/// Up to `budget` vertex indices taken at a fixed stride, skipping masked and
/// non-finite vertices.
///
/// A stride rather than a random draw: the result must be identical between
/// runs, and a stride over a scan's vertex order already spreads samples over
/// the whole surface.
///
/// Contract relied on by the refine stages: the result is empty exactly when the
/// soup has no usable vertex, so emptiness never depends on the budget. The
/// stride comes from the usable count, so a level with no usable vertex returns
/// nothing at every budget, and no budget can report evidence where another
/// reports none. (The sampled *sets* do not nest, because each budget chooses
/// its own stride alignment.) A refine that treated "the dense level returned
/// nothing" as recoverable was guarding a state this property makes unreachable.
#[must_use]
pub(crate) fn sample_vertices(soup: Soup<'_>, budget: usize) -> Vec<u32> {
    let count = soup.vertex_count();
    if count == 0 || budget == 0 {
        return Vec::new();
    }
    // Count usable vertices first. Computing the stride from the raw vertex
    // count let a periodic exclusion mask alias every sampled index away,
    // returning no evidence even while valid triangles remained between the
    // stride positions. A second linear pass keeps the output bounded without
    // allocating a temporary vector containing the whole mesh.
    let usable = (0..count)
        .filter(|&vertex| !soup.is_excluded(vertex) && vertex_at(soup.positions, vertex).is_some())
        .count();
    if usable == 0 {
        return Vec::new();
    }
    let stride = usable.div_ceil(budget).max(1);
    let mut out = Vec::with_capacity(usable.div_ceil(stride));
    let mut usable_seen = 0usize;
    for vertex in 0..count {
        if soup.is_excluded(vertex) || vertex_at(soup.positions, vertex).is_none() {
            continue;
        }
        if usable_seen.is_multiple_of(stride) {
            if let Ok(index) = u32::try_from(vertex) {
                out.push(index);
            }
        }
        usable_seen += 1;
    }
    out
}

/// Area-weighted vertex normals computed from triangle winding.
#[must_use]
pub(crate) fn vertex_normals(soup: Soup<'_>) -> Vec<DVec3> {
    let count = soup.vertex_count();
    let mut normals = vec![DVec3::ZERO; count];
    for slice in soup.indices.as_chunks::<3>().0 {
        let mut corners = [DVec3::ZERO; 3];
        let mut vertices = [0usize; 3];
        let mut usable = true;
        for (slot, &raw) in slice.iter().enumerate() {
            let Ok(vertex) = usize::try_from(raw) else {
                usable = false;
                break;
            };
            let Some(point) = vertex_at(soup.positions, vertex) else {
                usable = false;
                break;
            };
            if vertex >= count {
                usable = false;
                break;
            }
            if soup.is_excluded(vertex) {
                usable = false;
                break;
            }
            corners[slot] = point;
            vertices[slot] = vertex;
        }
        if !usable {
            continue;
        }
        let face = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        if !face.is_finite() {
            continue;
        }
        for vertex in vertices {
            if !soup.is_excluded(vertex) {
                normals[vertex] += face;
            }
        }
    }
    for normal in &mut normals {
        *normal = normal.normalize_or_zero();
    }
    normals
}

/// Bounding-box centre and diagonal of the soup, in millimetres, in the
/// soup's own frame.
///
/// `None` when no finite vertex is available. The centre is reported in the
/// soup's local frame; callers must not infer it from the coordinate origin.
#[must_use]
pub fn bounds_of(soup: Soup<'_>) -> Option<(DVec3, f64)> {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut seen = false;
    for vertex in 0..soup.vertex_count() {
        if !soup.is_excluded(vertex) {
            if let Some(point) = vertex_at(soup.positions, vertex) {
                min = min.min(point);
                max = max.max(point);
                seen = true;
            }
        }
    }
    seen.then(|| ((min + max) * 0.5, (max - min).length()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]
    use super::{bounds_of, sample_vertices, vertex_normals};
    use crate::Soup;
    use glam::DVec3;

    fn quad() -> (Vec<f32>, Vec<u32>) {
        (
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 1.0, 0.0,
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    #[test]
    fn sampling_is_deterministic_and_bounded_by_the_budget() {
        let (positions, indices) = quad();
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        let first = sample_vertices(soup, 2);
        let second = sample_vertices(soup, 2);
        assert_eq!(first, second);
        assert!(first.len() <= 2, "budget ignored: {first:?}");
    }

    #[test]
    fn sampling_skips_masked_and_non_finite_vertices() {
        let (mut positions, indices) = quad();
        positions[0] = f32::NAN;
        let mask = [0u8, 1, 0, 0];
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: Some(&mask),
        };
        let sampled = sample_vertices(soup, 16);
        assert!(!sampled.contains(&0), "a NaN vertex was sampled");
        assert!(!sampled.contains(&1), "a masked vertex was sampled");
        assert_eq!(sampled, vec![2, 3]);
    }

    #[test]
    fn sampling_refills_after_a_stride_would_alias_the_mask() {
        let positions = vec![0.0; 100 * 3];
        let mut mask = vec![crate::EXCLUDED; 100];
        mask[1] = crate::INCLUDED;
        let soup = Soup {
            positions: &positions,
            indices: &[],
            mask: Some(&mask),
        };

        let sampled = sample_vertices(soup, 8);

        assert_eq!(sampled, vec![1]);
    }

    #[test]
    fn vertex_normals_face_the_winding_not_the_file() {
        let (positions, indices) = quad();
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        for normal in vertex_normals(soup) {
            assert!((normal.dot(DVec3::Z) - 1.0).abs() < 1e-9, "{normal:?}");
        }
    }

    #[test]
    fn the_bounds_diagonal_spans_the_soup() {
        let (positions, indices) = quad();
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        let Some((_, diagonal)) = bounds_of(soup) else {
            panic!("a quad has bounds");
        };
        assert!((diagonal - 2.0f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn an_empty_soup_has_no_extent_and_no_samples() {
        let soup = Soup {
            positions: &[],
            indices: &[],
            mask: None,
        };
        assert!(bounds_of(soup).is_none(), "an empty soup has no bounds");
        assert!(sample_vertices(soup, 8).is_empty());
    }

    #[test]
    fn masked_geometry_does_not_influence_normals_or_bounds() {
        let (positions, indices) = quad();
        let mask = [1, 1, 0, 0];
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: Some(&mask),
        };
        let normals = vertex_normals(soup);
        assert_eq!(normals[0], DVec3::ZERO);
        assert_eq!(normals[1], DVec3::ZERO);
        assert_eq!(
            normals[2],
            DVec3::ZERO,
            "a face touching an excluded vertex is not usable geometry"
        );
        assert_eq!(
            normals[3],
            DVec3::ZERO,
            "a face touching an excluded vertex is not usable geometry"
        );
        let Some((center, diagonal)) = bounds_of(soup) else {
            panic!("the included vertices still have bounds");
        };
        assert_eq!(center, DVec3::new(0.5, 1.0, 0.0));
        assert!((diagonal - 1.0).abs() < 1e-9);
    }
    /// The property the refine stages actually rely on: emptiness is a fact
    /// about the soup, not about the budget. A level cannot report evidence
    /// where another reports none, so "the dense level returned nothing" can
    /// never mean "the coarse level had evidence the dense one lost".
    ///
    /// The sampled sets themselves do *not* nest - each budget aligns its own
    /// stride - which is why this asserts emptiness rather than a subset.
    #[test]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // Fixture grid.
    fn emptiness_never_depends_on_the_sampling_budget() {
        let count = 250_000usize;
        let positions: Vec<f32> = (0..count).flat_map(|i| [i as f32, 0.0, 0.0]).collect();
        let indices: Vec<u32> = (0..count as u32).collect();
        let usable = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        for budget in [1usize, 7, 8_000, 40_000, 250_000] {
            assert!(
                !sample_vertices(usable, budget).is_empty(),
                "a fully usable soup must sample something at budget {budget}"
            );
        }

        let excluded = vec![1u8; count];
        let masked = Soup {
            positions: &positions,
            indices: &indices,
            mask: Some(&excluded),
        };
        for budget in [1usize, 8_000, 40_000] {
            assert!(
                sample_vertices(masked, budget).is_empty(),
                "a fully excluded soup must stay empty at budget {budget}"
            );
        }
    }

    /// A soup with nothing usable returns nothing at every budget: empty is a
    /// property of the input, never of the budget.
    #[test]
    fn an_unusable_soup_is_empty_at_every_budget() {
        let positions: Vec<f32> = vec![f32::NAN; 300];
        let indices: Vec<u32> = (0..300).collect();
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        for budget in [8_000usize, 40_000] {
            assert!(sample_vertices(soup, budget).is_empty());
        }
    }
}

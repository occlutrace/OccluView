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
#[must_use]
pub(crate) fn sample_vertices(soup: Soup<'_>, budget: usize) -> Vec<u32> {
    let count = soup.vertex_count();
    if count == 0 || budget == 0 {
        return Vec::new();
    }
    let stride = count.div_ceil(budget).max(1);
    let mut out = Vec::with_capacity(count.div_ceil(stride));
    let mut vertex = 0usize;
    while vertex < count {
        if !soup.is_excluded(vertex) && vertex_at(soup.positions, vertex).is_some() {
            if let Ok(index) = u32::try_from(vertex) {
                out.push(index);
            }
        }
        vertex += stride;
    }
    out
}

/// Area-weighted vertex normals for the whole soup, normalized.
///
/// Computed from triangle geometry: a scan's stored normals routinely disagree
/// with its own winding, and the orientation test that decides whether a
/// correspondence is accepted would then reject the right matches.
#[must_use]
pub(crate) fn vertex_normals(soup: Soup<'_>) -> Vec<DVec3> {
    let count = soup.vertex_count();
    let mut normals = vec![DVec3::ZERO; count];
    for slice in soup.indices.chunks_exact(3) {
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
            normals[vertex] += face;
        }
    }
    for normal in &mut normals {
        *normal = normal.normalize_or_zero();
    }
    normals
}

/// Bounding-box diagonal of the soup, in millimetres. Zero when nothing is
/// usable — callers treat that as "no plausible motion".
#[must_use]
pub fn extent_of(soup: Soup<'_>) -> f64 {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut seen = false;
    for vertex in 0..soup.vertex_count() {
        if let Some(point) = vertex_at(soup.positions, vertex) {
            min = min.min(point);
            max = max.max(point);
            seen = true;
        }
    }
    if seen {
        (max - min).length()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{extent_of, sample_vertices, vertex_normals};
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
    fn the_extent_is_the_bounding_box_diagonal() {
        let (positions, indices) = quad();
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        assert!((extent_of(soup) - 2.0f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn an_empty_soup_has_no_extent_and_no_samples() {
        let soup = Soup {
            positions: &[],
            indices: &[],
            mask: None,
        };
        assert_eq!(extent_of(soup), 0.0);
        assert!(sample_vertices(soup, 8).is_empty());
    }
}

//! Minimum-area cap triangulation (Barequet & Sharir 1995; the base
//! triangulation of Liepa 2003).
//!
//! The planar ear-clip needs the rim's projection onto its Newell plane to be
//! a simple polygon. A strongly curved rim (a hole wrapping far around a
//! tooth flank or a scan flap) violates that and used to be refused outright.
//! This dynamic program triangulates the CYCLIC rim directly in 3D — no
//! projection at all — by minimizing total triangle area over the classic
//! polygon-triangulation recurrence:
//!
//! `W[i][j] = min over k in (i, j) of W[i][k] + W[k][j] + area(i, k, j)`
//!
//! Cost is O(n^3) time / O(n^2) memory, so it is bounded to modest rims and
//! only runs when the ear-clip has already refused. Ties break on the lowest
//! `k` for determinism. The result is always a full, fan-consistent cover of
//! the rim (`n - 2` triangles); geometric self-piercing on truly pathological
//! rims is caught by the caller's cap guard.

use glam::{DVec3, Vec3};

/// Leaf size for the min-area dynamic program: 256^3 is milliseconds. Rims
/// longer than this are triangulated by [`min_area_triangulation_any`], which
/// recursively splits them into sub-rims at or below this size before running
/// the O(n^3) DP on each leaf.
pub(super) const MIN_WEIGHT_MAX_RIM: usize = 256;

/// Absolute ceiling for the hierarchical min-area path: a socket rim of a few
/// thousand edges closes comfortably, but a pathological rim past this stays
/// refused rather than allocate unboundedly. Matches the selection-scoped
/// boundary-loop ceiling so nothing the walk admits is silently un-cappable.
pub(super) const MIN_WEIGHT_HIER_MAX_RIM: usize = 20_000;

/// Two non-adjacent rim segments count as a crossing only when they approach
/// within this fraction of their LOCAL edge length — a tube whose radius scales
/// with the edges involved, so an honest wiggle stays simple while a real
/// self-crossing (which touches at zero distance) is caught. Using the local
/// (per-pair) scale, not a global mean, is what keeps a rim carrying a few tiny
/// near-coincident seam edges next to long edges from being falsely damaged:
/// a global mean sets the tube far wider than a tiny edge and refuses honest
/// close-but-not-touching seam segments.
const RIM_PROXIMITY_FRACTION: f64 = 1e-3;

/// Whether the 3D rim polyline is simple: no two non-adjacent segments touch
/// or cross. This is the discriminator between the two ear-clip failure
/// modes — a strongly CURVED rim (simple in 3D, self-overlapping only in
/// projection) deserves the minimum-area fallback, while a genuinely
/// self-crossing rim (hourglass damage) must stay refused: capping it would
/// bake the crossing into the surface. O(n^2) segment pairs, n <= 256.
pub(super) fn rim_is_simple_3d(points: &[Vec3]) -> bool {
    let n = points.len();
    if n < 4 {
        return true; // Triangles cannot self-cross.
    }
    let points: Vec<DVec3> = points.iter().map(Vec3::as_dvec3).collect();
    let edge_len: Vec<f64> = (0..n)
        .map(|index| (points[(index + 1) % n] - points[index]).length())
        .collect();
    if edge_len.iter().any(|len| !len.is_finite()) {
        return false;
    }

    for i in 0..n {
        let (a0, a1) = (points[i], points[(i + 1) % n]);
        for j in (i + 2)..n {
            // Skip segments adjacent to segment i (they share an endpoint);
            // (n - 1, 0) wraps around to touch segment 0.
            if i == 0 && j == n - 1 {
                continue;
            }
            let (b0, b1) = (points[j], points[(j + 1) % n]);
            // Local tube: proportional to the SMALLER of the two edges, so the
            // radius never dwarfs a tiny seam edge sitting near a long one.
            let local_scale = edge_len[i].min(edge_len[j]);
            let threshold = local_scale * RIM_PROXIMITY_FRACTION;
            if segment_distance(a0, a1, b0, b1) < threshold {
                // Exactly coincident ENDPOINTS are seam data (dental formats
                // duplicate positions with distinct indices on purpose), not
                // a crossing: only a mid-segment contact is damage.
                let endpoint_touch = a0 == b0 || a0 == b1 || a1 == b0 || a1 == b1;
                if !endpoint_touch {
                    return false;
                }
            }
        }
    }
    true
}

/// Minimum distance between segments `a0..a1` and `b0..b1` (clamped
/// closest-point form, Ericson "Real-Time Collision Detection" §5.1.9).
fn segment_distance(a0: DVec3, a1: DVec3, b0: DVec3, b1: DVec3) -> f64 {
    let dir_a = a1 - a0;
    let dir_b = b1 - b0;
    let offset = a0 - b0;
    let len_a = dir_a.length_squared();
    let len_b = dir_b.length_squared();
    let proj_b = dir_b.dot(offset);
    let (param_a, param_b) = if len_a <= f64::EPSILON && len_b <= f64::EPSILON {
        (0.0, 0.0)
    } else if len_a <= f64::EPSILON {
        (0.0, (proj_b / len_b).clamp(0.0, 1.0))
    } else {
        let proj_a = dir_a.dot(offset);
        if len_b <= f64::EPSILON {
            ((-proj_a / len_a).clamp(0.0, 1.0), 0.0)
        } else {
            let dot_dirs = dir_a.dot(dir_b);
            let denom = len_a * len_b - dot_dirs * dot_dirs;
            let param_a = if denom.abs() > f64::EPSILON {
                ((dot_dirs * proj_b - proj_a * len_b) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let param_b_raw = (dot_dirs * param_a + proj_b) / len_b;
            // Re-clamp the first parameter against the clamped second.
            if param_b_raw < 0.0 {
                ((-proj_a / len_a).clamp(0.0, 1.0), 0.0)
            } else if param_b_raw > 1.0 {
                (((dot_dirs - proj_a) / len_a).clamp(0.0, 1.0), 1.0)
            } else {
                (param_a, param_b_raw)
            }
        }
    };
    ((a0 + dir_a * param_a) - (b0 + dir_b * param_b)).length()
}

/// Triangulate a cyclic rim (positions in ring order) by minimum total area.
/// Returns local-index triangles in the caller's watertight winding
/// convention (`[i, j, k]` with `i < k < j`, matching the ear-clip's
/// reversed-rim-edge emit order), or `None` when the rim is too long, too
/// short, or numerically degenerate.
pub(super) fn min_area_triangulation(points: &[Vec3]) -> Option<Vec<[usize; 3]>> {
    let n = points.len();
    if !(3..=MIN_WEIGHT_MAX_RIM).contains(&n) {
        return None;
    }
    let points: Vec<DVec3> = points.iter().map(Vec3::as_dvec3).collect();
    let area = |i: usize, k: usize, j: usize| -> f64 {
        let ab = points[k] - points[i];
        let ac = points[j] - points[i];
        ab.cross(ac).length() * 0.5
    };

    // W[i][j] over the flattened upper triangle, j > i, gap = j - i >= 2.
    let mut weight = vec![0.0_f64; n * n];
    let mut split = vec![0_usize; n * n];
    for gap in 2..n {
        for i in 0..(n - gap) {
            let j = i + gap;
            let mut best = f64::INFINITY;
            let mut best_k = 0;
            for k in (i + 1)..j {
                let candidate = weight[i * n + k] + weight[k * n + j] + area(i, k, j);
                if candidate < best {
                    best = candidate;
                    best_k = k;
                }
            }
            weight[i * n + j] = best;
            split[i * n + j] = best_k;
        }
    }
    // Full-span weight W[0][n-1]; infinite/NaN means degenerate input.
    if !weight[n - 1].is_finite() {
        return None;
    }

    // Reconstruct with an explicit stack (no recursion in the kernel).
    let mut triangles = Vec::with_capacity(n - 2);
    let mut stack = vec![(0_usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j - i < 2 {
            continue;
        }
        let k = split[i * n + j];
        // Watertight winding: [i, j, k] contains the REVERSED rim edges
        // (k -> i when k = i + 1, j -> k when j = k + 1), the twin of the
        // surrounding faces' directed boundary edges.
        triangles.push([i, j, k]);
        stack.push((i, k));
        stack.push((k, j));
    }
    if triangles.len() != n - 2 {
        return None;
    }
    Some(triangles)
}

/// Triangulate a cyclic rim of ANY size (up to [`MIN_WEIGHT_HIER_MAX_RIM`]) by
/// divide-and-conquer minimum-area capping. Small rims run the DP directly;
/// large ones are split at a near-balanced, most-distant vertex pair into two
/// sub-arcs joined by a shared chord (an interior edge, watertight by
/// construction), recursively until each leaf fits [`MIN_WEIGHT_MAX_RIM`].
///
/// Returns local-index triangles into the ORIGINAL `points` ordering, in the
/// same watertight winding convention as [`min_area_triangulation`], or `None`
/// when the rim is out of range or numerically degenerate. Geometric
/// self-piercing is left to the caller's cap guard, exactly as for the direct
/// DP. Deterministic: the split pair is chosen by a fixed rule and ties break
/// on the lowest index.
pub(super) fn min_area_triangulation_any(points: &[Vec3]) -> Option<Vec<[usize; 3]>> {
    let n = points.len();
    if !(3..=MIN_WEIGHT_HIER_MAX_RIM).contains(&n) {
        return None;
    }
    if n <= MIN_WEIGHT_MAX_RIM {
        return min_area_triangulation(points);
    }
    let dpoints: Vec<DVec3> = points.iter().map(Vec3::as_dvec3).collect();

    // Work items are arcs of the cyclic rim: contiguous runs of ORIGINAL
    // indices in ring order. The arc's two endpoints are joined by an implicit
    // chord, so each arc is the closed polygon (arc + chord) to triangulate.
    let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(n - 2);
    let mut stack: Vec<Vec<usize>> = vec![(0..n).collect()];
    // Every split reduces the largest arc, and both children keep >= 2 edges,
    // so the arc count is bounded by n; the guard only trips on a NaN-position
    // pathology that keeps splitting without shrinking.
    let mut rounds = 0_usize;
    let round_budget = 8 * n;
    while let Some(arc) = stack.pop() {
        rounds += 1;
        if rounds > round_budget {
            return None;
        }
        let m = arc.len();
        if m < 3 {
            // A 2-point arc is just the chord; it contributes no triangle and
            // its edge cancels against the sibling. Anything shorter cannot
            // occur (splits keep >= 2 edges).
            continue;
        }
        if m <= MIN_WEIGHT_MAX_RIM {
            let sub_points: Vec<Vec3> = arc.iter().map(|&idx| points[idx]).collect();
            let leaf = min_area_triangulation(&sub_points)?;
            for [a, b, c] in leaf {
                triangles.push([arc[a], arc[b], arc[c]]);
            }
            continue;
        }
        let (first, second) = balanced_far_split(&dpoints, &arc);
        // arc[first..=second] and arc[second..] + arc[..=first] both keep the
        // split pair as their shared chord endpoints.
        let inner: Vec<usize> = arc[first..=second].to_vec();
        let mut outer: Vec<usize> = arc[second..].to_vec();
        outer.extend_from_slice(&arc[..=first]);
        stack.push(inner);
        stack.push(outer);
    }
    if triangles.len() != n - 2 {
        return None;
    }
    Some(triangles)
}

/// Pick a split of `arc` (positions of its original indices in `points`) into
/// two near-balanced sub-arcs at a far-apart vertex pair. Returns local slot
/// indices `(first, second)` with `first < second`, both sub-arcs keeping at
/// least two edges. Deterministic.
fn balanced_far_split(points: &[DVec3], arc: &[usize]) -> (usize, usize) {
    let m = arc.len();
    // Anchor at the vertex farthest from arc[0], then split the ring between
    // arc[0] and that vertex, clamped into the balanced band [m/4, 3m/4] so
    // each side keeps a healthy share and the recursion always shrinks.
    let p0 = points[arc[0]];
    let mut best_slot = m / 2;
    let mut best_d2 = -1.0_f64;
    for (slot, &idx) in arc.iter().enumerate().skip(1) {
        let d2 = (points[idx] - p0).length_squared();
        if d2 > best_d2 {
            best_d2 = d2;
            best_slot = slot;
        }
    }
    let low = (m / 4).max(1);
    let high = (3 * m / 4).min(m - 1);
    let second = best_slot.clamp(low, high);
    (0, second)
}

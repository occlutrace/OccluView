//! Minimum-weight cap triangulation (Barequet & Sharir 1995; Liepa 2003).
//!
//! The dynamic program triangulates a cyclic rim directly in 3D, avoiding
//! projection for strongly curved loops, using:
//!
//! `W[i][j] = best over k in (i, j) of W[i][k] + W[k][j] + weight(i, k, j)`
//!
//! The weight is Liepa's: how sharply the cover folds at its worst edge, then
//! its area. A fold is measured between two triangles of the cover and, at a
//! rim edge, between the cover and the scan triangle across it. Area alone
//! has no inside and outside. Where a corner of the scan sticks into the
//! hole, the smallest triangle is the one laid back over that corner, with
//! the rest of the cover running on underneath, and a cut line has such a
//! corner every few edges.
//!
//! Cost is O(n^3) time / O(n^2) memory, so the program covers rims up to
//! [`MIN_WEIGHT_MAX_RIM`] points; a longer rim has ears clipped off it first,
//! until what is left is one the program can cover whole. Ties use the lowest
//! `k`; the caller rejects self-piercing results.

use super::cap_fair::RimSupport;
use glam::{DVec3, Vec3, Vec4};
use std::collections::HashSet;

/// The longest rim the minimum-weight dynamic program covers. A longer one is
/// brought down to this size by [`clip_ears`] first; one that cannot be, for
/// want of ears, is split into sub-rims at or below this size.
///
/// A rim split at a chord is covered in two halves that know nothing of each
/// other, and on a rim that is far from flat the halves cross: the program
/// has to hold the rims the tool is for in one piece. On real arches a lasso
/// round one molar leaves a rim of 340 to 510 edges, which a leaf of 256
/// always split; the DP on 510 points takes a quarter of a second.
pub(super) const MIN_WEIGHT_MAX_RIM: usize = 512;

/// Absolute ceiling for the hierarchical minimum-weight path: a socket rim of
/// a few thousand edges closes comfortably, but a pathological rim past this
/// stays refused rather than allocate unboundedly. Matches the selection-scoped
/// boundary-loop ceiling, so no loop the walk admits is refused for size here.
pub(super) const MIN_WEIGHT_HIER_MAX_RIM: usize = 20_000;

/// Relative proximity threshold for non-adjacent rim segments. Scaling by the
/// local pair avoids rejecting short seam edges near long segments.
const RIM_PROXIMITY_FRACTION: f64 = 1e-3;

/// Segment pairs the simplicity test may compare on one rim.
///
/// The test is O(n²) and the ceiling this crate admits is `MIN_WEIGHT_HIER_MAX_RIM`
/// = 20 000 edges, i.e. ~2·10^8 pairs, each running a full f64 closest-point
/// form. Like the other expensive passes here (the large ear clipper spends at
/// most `LARGE_EARCLIP_WORK_BUDGET` reflex checks, the minimum-weight DP spends
/// `MIN_WEIGHT_HIER_MAX_RIM`), this one is budgeted. A rim that exhausts the
/// budget is refused: the test exists to reject unverifiable rims, and
/// refusing keeps the outcome deterministic instead of stretching to minutes
/// on a pathological socket or lasso boundary.
const RIM_SIMPLICITY_PAIR_BUDGET: u64 = 20_000_000;

/// Whether the 3D rim polyline is simple. Curved but simple rims use the
/// minimum-weight fallback; self-crossing rims remain rejected. O(n²) in pair
/// count, bounded by [`RIM_SIMPLICITY_PAIR_BUDGET`].
pub(super) fn rim_is_simple_3d(points: &[Vec3]) -> bool {
    let n = points.len();
    if n < 4 {
        return true; // Triangles cannot self-cross.
    }
    let points: Vec<DVec3> = points.iter().map(|point| point.as_dvec3()).collect();
    let edge_len: Vec<f64> = (0..n)
        .map(|index| (points[(index + 1) % n] - points[index]).length())
        .collect();
    if edge_len.iter().any(|len| !len.is_finite()) {
        return false;
    }

    let mut pairs_left = RIM_SIMPLICITY_PAIR_BUDGET;
    for i in 0..n {
        let (a0, a1) = (points[i], points[(i + 1) % n]);
        for j in (i + 2)..n {
            // Skip segments adjacent to segment i (they share an endpoint);
            // (n - 1, 0) wraps around to touch segment 0.
            if i == 0 && j == n - 1 {
                continue;
            }
            // Refuse rather than stretch: an unbudgeted quadratic scan on a
            // 20 000-edge rim is ~2·10^8 closest-point evaluations.
            if pairs_left == 0 {
                return false;
            }
            pairs_left -= 1;
            let (b0, b1) = (points[j], points[(j + 1) % n]);
            // Local tube: proportional to the smaller of the two edges, so the
            // radius never dwarfs a tiny seam edge sitting near a long one.
            let local_scale = edge_len[i].min(edge_len[j]);
            let threshold = local_scale * RIM_PROXIMITY_FRACTION;
            if segment_distance(a0, a1, b0, b1) < threshold {
                // Exactly coincident endpoints are seam data (dental formats
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
    let (param_a, param_b) = if len_a == 0.0 && len_b == 0.0 {
        (0.0, 0.0)
    } else if len_a == 0.0 {
        (0.0, (proj_b / len_b).clamp(0.0, 1.0))
    } else {
        let proj_a = dir_a.dot(offset);
        if len_b == 0.0 {
            ((-proj_a / len_a).clamp(0.0, 1.0), 0.0)
        } else {
            let dot_dirs = dir_a.dot(dir_b);
            let denom = len_a * len_b - dot_dirs * dot_dirs;
            let param_a = if denom > f64::EPSILON * len_a * len_b {
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

/// The triangles a cap may not use, as ascending triples of rim-local indices:
/// the ones the surface already has on the rim's own vertices. A cap triangle
/// on one of them is its reverse twin.
pub(super) type TakenTriangles = HashSet<[usize; 3]>;

/// What a cover of part of a rim weighs (Liepa 2003).
#[derive(Copy, Clone)]
struct Cover {
    /// Cosine of the sharpest fold in the cover, between two of its triangles
    /// or between one and the scan triangle across its rim edge: one where
    /// everything goes straight on, minus one where a triangle lies back on
    /// its neighbour.
    straightness: f32,
    /// Infinite for a cover that does not exist.
    area: f64,
}

impl Cover {
    const IMPOSSIBLE: Self = Self {
        straightness: f32::NEG_INFINITY,
        area: f64::INFINITY,
    };

    /// Whether this cover is the better one: the one that folds less, and of
    /// two that fold alike the smaller. A cover that is not a number is never
    /// the better one.
    fn beats(self, other: Self) -> bool {
        // Exactly alike is the common case: the sharpest fold of a cover is
        // one number, carried up unchanged through every part that holds it.
        #[allow(clippy::float_cmp)]
        let alike = self.straightness == other.straightness;
        self.straightness > other.straightness || (alike && self.area < other.area)
    }
}

/// The triangle across an edge, as a fold is taken against it: its unit
/// normal with a zero behind it, or all zeros and a one where there is no
/// triangle. The dot product with a triangle's doubled area vector and its
/// length behind it is then that length times the cosine of the fold, and
/// that length times one where nothing folds.
fn across(normal: Vec3) -> Vec4 {
    if normal == Vec3::ZERO {
        Vec4::W
    } else {
        normal.extend(0.0)
    }
}

/// The best covers of every part of a rim, `W[i][j]` at `i * n + j`, and the
/// triangle each puts on the edge between its two end points.
#[derive(Clone)]
struct Covers {
    straightness: Vec<f32>,
    area: Vec<f64>,
    across: Vec<Vec4>,
}

impl Covers {
    /// Every part covered by nothing, with nothing across: the state of a
    /// single edge.
    fn edges(cells: usize) -> Self {
        Self {
            straightness: vec![1.0; cells],
            area: vec![0.0; cells],
            across: vec![Vec4::W; cells],
        }
    }

    fn set(&mut self, cell: usize, cover: Cover, across: Vec4) {
        self.straightness[cell] = cover.straightness;
        self.area[cell] = cover.area;
        self.across[cell] = across;
    }
}

/// Triangulate a cyclic rim (positions in ring order) by minimum weight, using
/// none of the `taken` triangles. `rim_index` gives the rim-local index of
/// each point and `edges[i]` the unit normal of the triangle across the edge
/// from point `i` to the next, the last to the first: a scan triangle, or a
/// triangle the cover already has there; zero where there is none, or none is
/// known. Returns local-index triangles in the caller's watertight winding
/// convention (`[i, j, k]` with `i < k < j`, matching the ear-clip's
/// reversed-rim-edge emit order), or `None` when the rim is too long, too
/// short, numerically degenerate, or has no triangulation free of taken
/// triangles.
fn min_weight_triangulation(
    points: &[Vec3],
    rim_index: &[usize],
    taken: &TakenTriangles,
    edges: &[Vec3],
) -> Option<Vec<[usize; 3]>> {
    let n = points.len();
    if !(3..=MIN_WEIGHT_MAX_RIM).contains(&n) {
        return None;
    }
    let edge = |from: usize| -> Vec4 { edges.get(from).map_or(Vec4::W, |&edge| across(edge)) };
    // The points of taken triangles: only a triangle on three of them needs
    // looking up.
    let in_taken: Vec<bool> = {
        let corners: HashSet<usize> = taken.iter().flatten().copied().collect();
        rim_index
            .iter()
            .map(|index| corners.contains(index))
            .collect()
    };

    // W[i][j], j > i, kept twice, by row and by column, so the inner loop
    // reads both of its operands in step: `W[i][k]` along row `i` and
    // `W[k][j]` along column `j`.
    let mut by_row = Covers::edges(n * n);
    for i in 0..(n - 1) {
        by_row.across[i * n + i + 1] = edge(i);
    }
    let mut by_column = by_row.clone();
    for i in 0..(n - 1) {
        by_column.across[(i + 1) * n + i] = by_row.across[i * n + i + 1];
    }
    let mut split = vec![0_usize; n * n];
    // The last point and the first close the polygon: the triangle on that
    // edge folds against whatever lies across it.
    let closing = edge(n - 1);
    for gap in 2..n {
        for i in 0..(n - gap) {
            let j = i + gap;
            let chord = points[j] - points[i];
            let on_taken = in_taken[i] && in_taken[j];
            let (row, column) = (i * n, j * n);
            let mut best = Cover::IMPOSSIBLE;
            // The winner's doubled area vector, in the winding it is emitted
            // with, `[i, j, k]`.
            let mut best_doubled = Vec3::ZERO;
            let mut best_k = 0;
            for k in (i + 1)..j {
                let (left, right) = (by_row.area[row + k], by_column.area[column + k]);
                if !(left.is_finite() && right.is_finite()) {
                    continue;
                }
                if on_taken && in_taken[k] {
                    let mut triple = [rim_index[i], rim_index[k], rim_index[j]];
                    triple.sort_unstable();
                    if taken.contains(&triple) {
                        continue;
                    }
                }
                let doubled = chord.cross(points[k] - points[i]);
                let length = doubled.length();
                // The triangle's own folds, each times `length`; a triangle
                // with no area folds nowhere.
                let facing = doubled.extend(length);
                let mut fold = facing
                    .dot(by_row.across[row + k])
                    .min(facing.dot(by_column.across[column + k]));
                if gap == n - 1 {
                    fold = fold.min(facing.dot(closing));
                }
                let below = by_row.straightness[row + k].min(by_column.straightness[column + k]);
                let candidate = Cover {
                    straightness: if fold >= below * length {
                        below
                    } else {
                        fold / length
                    },
                    area: left + right + f64::from(length) * 0.5,
                };
                if candidate.beats(best) {
                    best = candidate;
                    best_doubled = doubled;
                    best_k = k;
                }
            }
            let across = across(best_doubled.normalize_or_zero());
            by_row.set(row + j, best, across);
            by_column.set(column + i, best, across);
            split[row + j] = best_k;
        }
    }
    // A complete cover must have positive area, even when an indexed seam
    // requires individual zero-area connector faces.
    let whole = by_row.area[n - 1];
    if !whole.is_finite() || whole <= 0.0 {
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
        // Watertight winding: [i, j, k] contains the reversed rim edges
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

/// An ear is a needle, with no side to it that a fold could be taken against,
/// when its shortest altitude is under this fraction of its longest edge: the
/// measure the cut-line healing uses for a face too thin to carry surface.
const EAR_NEEDLE_FRACTION: f32 = 0.02;

/// A rim being clipped: the points still on it, as a ring, and what lies
/// across the edge from each to the next.
struct Ring {
    prev: Vec<usize>,
    next: Vec<usize>,
    /// Unit normal of the triangle across the edge from a point to the next:
    /// the scan's, or an ear's once one was clipped there. Zero for none.
    across: Vec<Vec3>,
    alive: Vec<bool>,
    len: usize,
}

impl Ring {
    fn new(across: Vec<Vec3>) -> Self {
        let n = across.len();
        Self {
            prev: (0..n).map(|point| (point + n - 1) % n).collect(),
            next: (0..n).map(|point| (point + 1) % n).collect(),
            across,
            alive: vec![true; n],
            len: n,
        }
    }

    /// The ear at `point`, in the winding it is emitted with, its doubled
    /// area vector, and how well it goes on from the triangles across its two
    /// rim edges: the cosine of the sharper of the two folds. `None` for a
    /// needle.
    fn ear(&self, points: &[Vec3], point: usize) -> Option<([usize; 3], Vec3, f32)> {
        let (before, after) = (self.prev[point], self.next[point]);
        let chord = points[after] - points[before];
        let (to_point, from_point) = (
            points[point] - points[before],
            points[after] - points[point],
        );
        let doubled = chord.cross(to_point);
        let longest = chord
            .length_squared()
            .max(to_point.length_squared())
            .max(from_point.length_squared());
        let length = doubled.length();
        // `altitude / longest edge`, with the altitude on the longest edge; a
        // length that is not a number is no ear either.
        let has_a_side = length > EAR_NEEDLE_FRACTION * longest;
        if !has_a_side {
            return None;
        }
        let unit = doubled / length;
        let fold = |across: Vec3| {
            if across == Vec3::ZERO {
                1.0
            } else {
                unit.dot(across)
            }
        };
        let straightness = fold(self.across[before]).min(fold(self.across[point]));
        Some(([before, after, point], doubled, straightness))
    }

    /// Clip the ear at `point`: the edge it leaves has the ear across it.
    fn clip(&mut self, point: usize, doubled: Vec3) {
        let (before, after) = (self.prev[point], self.next[point]);
        self.next[before] = after;
        self.prev[after] = before;
        self.across[before] = doubled.normalize_or_zero();
        self.alive[point] = false;
        self.len -= 1;
    }
}

/// Clip ears off `ring` until `target` points are left on it, or no point has
/// an ear worth clipping, and add them to `triangles`.
///
/// The dynamic program covers a rim whole, and its cost grows with the cube
/// of the rim. A rim it cannot hold used to be split at a chord and covered
/// in two halves that knew nothing of each other; on a rim that is far from
/// flat the halves crossed, or met in a crease from end to end. Ears take a
/// long rim down to the size the program can cover whole instead, and they
/// take off what is small: the saw of a cut line, a layer at a time.
///
/// An ear is clipped only where it goes on from what lies across both of its
/// rim edges without turning back over it, the scan or an ear clipped before.
/// A corner of the scan that sticks into the hole has no such ear, so it is
/// left for the program. A round clips the straightest ears first, no two
/// next to each other, so the rim shrinks evenly all round.
fn clip_ears(
    points: &[Vec3],
    ring: &mut Ring,
    taken: &TakenTriangles,
    target: usize,
    triangles: &mut Vec<[usize; 3]>,
) {
    let target = target.max(3);
    let mut touched = vec![usize::MAX; points.len()];
    let mut round = 0_usize;
    while ring.len > target {
        // Every ear on the ring, the straightest and then the smallest first.
        let mut ears: Vec<(f32, f32, usize)> = (0..points.len())
            .filter(|&point| ring.alive[point])
            .filter_map(|point| {
                let (ear, doubled, straightness) = ring.ear(points, point)?;
                let mut triple = ear;
                triple.sort_unstable();
                (straightness >= 0.0 && !taken.contains(&triple))
                    .then(|| (straightness, doubled.length_squared(), point))
            })
            .collect();
        ears.sort_unstable_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then(left.1.total_cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });
        let before = ring.len;
        for (_, _, point) in ears {
            if ring.len <= target {
                break;
            }
            let (prev, next) = (ring.prev[point], ring.next[point]);
            if [prev, point, next]
                .iter()
                .any(|&near| touched[near] == round)
            {
                continue;
            }
            let Some((ear, doubled, _)) = ring.ear(points, point) else {
                continue;
            };
            triangles.push(ear);
            ring.clip(point, doubled);
            touched[prev] = round;
            touched[next] = round;
        }
        if ring.len == before {
            break;
        }
        round += 1;
    }
}

/// Triangulate a cyclic rim of any size (up to [`MIN_WEIGHT_HIER_MAX_RIM`]) by
/// minimum weight. A rim the dynamic program can hold is covered by it whole;
/// a longer one has ears clipped off it until it can, see [`clip_ears`]. Only
/// a rim that still cannot be held, because too few of its points have an ear,
/// is split at a near-balanced, most-distant vertex pair into two sub-arcs
/// joined by a shared chord (an interior edge, watertight by construction),
/// recursively until each leaf fits [`MIN_WEIGHT_MAX_RIM`].
///
/// `support[i]` is what the scan adds at the rim edge from point `i` to the
/// next; an edge it says nothing about folds against nothing.
///
/// Returns local-index triangles into the original `points` ordering, in the
/// same watertight winding convention as [`min_weight_triangulation`], or
/// `None` when the rim is out of range, numerically degenerate, or cannot be
/// covered without a `taken` triangle. Geometric self-piercing is left to the
/// caller's cap guard, as for the direct DP. Deterministic: ears and the split
/// pair are chosen by fixed rules and ties break on the lowest index.
pub(super) fn min_weight_triangulation_any(
    points: &[Vec3],
    support: &[RimSupport],
    taken: &TakenTriangles,
) -> Option<Vec<[usize; 3]>> {
    let n = points.len();
    if !(3..=MIN_WEIGHT_HIER_MAX_RIM).contains(&n) {
        return None;
    }
    // The scan triangle across a rim edge lies in the plane of the edge and
    // its conormal.
    let scan: Vec<Vec3> = (0..n)
        .map(|index| {
            let conormal = support
                .get(index)
                .map_or(Vec3::ZERO, |support| support.conormal);
            (points[(index + 1) % n] - points[index])
                .cross(conormal)
                .normalize_or_zero()
        })
        .collect();
    let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(n - 2);
    let mut ring = Ring::new(scan);
    clip_ears(points, &mut ring, taken, MIN_WEIGHT_MAX_RIM, &mut triangles);

    // What is left of the rim, in ring order, and what lies across each of
    // its edges.
    let core: Vec<usize> = (0..n).filter(|&point| ring.alive[point]).collect();
    let m = core.len();
    let dpoints: Vec<DVec3> = core.iter().map(|&point| points[point].as_dvec3()).collect();

    // Work items are arcs of the core: contiguous runs of its points in ring
    // order. The arc's two endpoints are joined by an implicit chord, so each
    // arc is the closed polygon (arc + chord) to triangulate; the first is
    // the whole core, whose chord is one of its own edges.
    let mut stack: Vec<Vec<usize>> = vec![(0..m).collect()];
    // Every split reduces the largest arc, and both children keep >= 2 edges,
    // so the arc count is bounded by m; the guard only trips on a NaN-position
    // pathology that keeps splitting without shrinking.
    let mut rounds = 0_usize;
    let round_budget = 8 * m;
    while let Some(arc) = stack.pop() {
        rounds += 1;
        if rounds > round_budget {
            return None;
        }
        let len = arc.len();
        if len < 3 {
            // A 2-point arc is just the chord; it contributes no triangle and
            // its edge cancels against the sibling. Anything shorter cannot
            // occur (splits keep >= 2 edges).
            continue;
        }
        if len <= MIN_WEIGHT_MAX_RIM {
            let sub_points: Vec<Vec3> = arc.iter().map(|&slot| points[core[slot]]).collect();
            let sub_index: Vec<usize> = arc.iter().map(|&slot| core[slot]).collect();
            // A chord of a split core has the other part's cover across it,
            // which is not known here.
            let sub_edges: Vec<Vec3> = (0..len)
                .map(|at| {
                    let (from, to) = (arc[at], arc[(at + 1) % len]);
                    if (from + 1) % m == to {
                        ring.across[core[from]]
                    } else {
                        Vec3::ZERO
                    }
                })
                .collect();
            let leaf = min_weight_triangulation(&sub_points, &sub_index, taken, &sub_edges)?;
            for [a, b, c] in leaf {
                triangles.push([sub_index[a], sub_index[b], sub_index[c]]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rim_simplicity_is_scale_independent() {
        let crossed = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
        ];
        let square = [crossed[0], crossed[3], crossed[1], crossed[2]];
        for scale in [1.0e-20_f32, 1.0e-10, 1.0e-4, 1.0, 1.0e20] {
            assert!(rim_is_simple_3d(&square.map(|point| point * scale)));
            assert!(
                !rim_is_simple_3d(&crossed.map(|point| point * scale)),
                "a crossing remains damage at scale {scale}"
            );
        }
    }

    #[test]
    fn minimum_weight_cap_refuses_a_zero_area_rim() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
        assert!(min_weight_triangulation_any(&points, &[], &TakenTriangles::new()).is_none());
    }

    fn uses(triangles: &[[usize; 3]], wanted: [usize; 3]) -> bool {
        triangles.iter().any(|triangle| {
            let mut triple = *triangle;
            triple.sort_unstable();
            triple == wanted
        })
    }

    /// A long hole in the plane `z = 0`, in the order the scan around it has
    /// its rim, with a corner of the scan standing up into it at vertex 1.
    const SPIKED_RIM: [Vec3; 5] = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.2, 1.0),
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(2.0, 10.0, 0.0),
        Vec3::new(0.0, 10.0, 0.0),
    ];

    /// The scan around [`SPIKED_RIM`]: the two faces of the spike, which meet
    /// behind its tip, and the sheet beyond the other edges.
    fn spiked_rim_support() -> Vec<RimSupport> {
        let rim = SPIKED_RIM;
        let behind = Vec3::new(1.0, -1.0, 0.0);
        vec![
            RimSupport::across(rim[0], rim[1], behind),
            RimSupport::across(rim[1], rim[2], behind),
            RimSupport::across(rim[2], rim[3], Vec3::new(3.0, 5.0, 0.0)),
            RimSupport::across(rim[3], rim[4], Vec3::new(1.0, 11.0, 0.0)),
            RimSupport::across(rim[4], rim[0], Vec3::new(-1.0, 5.0, 0.0)),
        ]
    }

    /// The smallest cover of a rim with a spike in it lays a triangle back
    /// over the spike and runs on underneath: the scan twice in one place.
    /// The cover has to go round the tip instead.
    #[test]
    fn the_cover_does_not_lie_back_over_a_spike_of_the_scan() {
        let area = |triangles: &[[usize; 3]]| -> f32 {
            triangles
                .iter()
                .map(|&[a, b, c]| {
                    (SPIKED_RIM[b] - SPIKED_RIM[a])
                        .cross(SPIKED_RIM[c] - SPIKED_RIM[a])
                        .length()
                        * 0.5
                })
                .sum()
        };
        let cover = min_weight_triangulation_any(
            &SPIKED_RIM,
            &spiked_rim_support(),
            &TakenTriangles::new(),
        )
        .expect("a cover exists");
        assert_eq!(cover.len(), SPIKED_RIM.len() - 2);
        assert!(
            !uses(&cover, [0, 1, 2]),
            "the cover lies on the spike: {cover:?}"
        );
        let over_the_spike = [[0, 2, 1], [0, 3, 2], [0, 4, 3]];
        assert!(
            area(&over_the_spike) < area(&cover),
            "the fixture is one where lying on the spike is the smaller cover"
        );
    }

    /// Three rim vertices in a row that the scan already joins: the cover
    /// leaves that triangle alone whatever it weighs.
    #[test]
    fn the_cover_goes_round_a_triangle_the_surface_already_has() {
        let free =
            min_weight_triangulation_any(&SPIKED_RIM, &[], &TakenTriangles::new()).expect("cover");
        let mut wanted = free[0];
        wanted.sort_unstable();

        let taken = TakenTriangles::from([wanted]);
        let round = min_weight_triangulation_any(&SPIKED_RIM, &[], &taken).expect("a cover exists");
        assert_eq!(round.len(), SPIKED_RIM.len() - 2);
        assert!(
            !uses(&round, wanted),
            "the cover reuses a taken triangle: {round:?}"
        );

        // A rim that is nothing but a taken triangle has no cover at all.
        let taken = TakenTriangles::from([[0, 1, 2]]);
        assert!(min_weight_triangulation_any(&SPIKED_RIM[..3], &[], &taken).is_none());
    }
}

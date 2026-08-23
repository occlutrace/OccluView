use super::Vertex;
use glam::Vec3;
use rayon::prelude::*;

const SMOOTH_DUPLICATE_NORMAL_DOT: f32 = 0.5;

/// Above this many vertices sharing one position, normal agreement is judged
/// against the group mean rather than pairwise.
///
/// Well past any real vertex valence -- a fan around one point is tens of
/// triangles, not hundreds -- so no scan reaches it, and a crafted file cannot
/// spend minutes here.
const MAX_PAIRWISE_DUPLICATE_GROUP: usize = 256;
// The welding tolerance and its key function live in `occlu-mesh-edit`. One
// number has to decide which vertices share a normal at load and after every
// edit, or a scan changes shading the first time it is touched. See
// `occlu_mesh_edit::COINCIDENT_POSITION_EPS_MM`.
use occlu_mesh_edit::coincident_position_key as position_key;
/// A facet is degenerate when its area falls below this fraction of its own
/// longest edge squared -- a scale-invariant test, so a 7 um lab-scanner facet
/// is judged by its shape rather than by an absolute epsilon.
///
/// Four crates need the rule. `occlu-mesh-edit` and `occluview-hps` sit below
/// this one in the layering and cannot depend on it, so they keep their own
/// copy and say so. `occluview-formats` uses this definition.
pub const DEGENERATE_AREA_SIN: f32 = 1e-10;

fn normal_is_usable(normal: [f32; 3]) -> bool {
    let n = Vec3::from_array(normal);
    n.is_finite() && n.length_squared() > f32::EPSILON
}

pub(super) fn repair_missing_normals(vertices: &mut [Vertex], indices: &[u32]) {
    if vertices
        .iter()
        .all(|vertex| !normal_is_usable(vertex.normal))
    {
        compute_smooth_normals(vertices, indices);
        smooth_duplicate_position_normals(vertices);
        return;
    }

    if vertices
        .iter()
        .all(|vertex| normal_is_usable(vertex.normal))
    {
        // STL and some exporters write per-facet normals (one flat normal per
        // triangle). These are "usable" but produce a faceted/specular speckle
        // that disappears under sculpt's full recompute. Only replace when the
        // geometric variance is high (large mesh with many distinct positions);
        // otherwise defer to duplicate-position averaging which preserves soft vs
        // sharp edges on small fixtures.
        if vertices.len() > 100 {
            let smooth = smooth_normals(vertices, indices);
            let mut disagree = 0usize;
            let mut total = 0usize;
            for (vertex, s) in vertices.iter().zip(&smooth) {
                if s.length_squared() > f32::EPSILON {
                    let a = Vec3::from_array(vertex.normal).normalize_or_zero();
                    let b = s.normalize_or_zero();
                    if a.length_squared() > f32::EPSILON && b.length_squared() > f32::EPSILON {
                        total += 1;
                        if a.dot(b) < 0.85 {
                            disagree += 1;
                        }
                    }
                }
            }
            if total > 0 && disagree * 4 > total {
                for (vertex, s) in vertices.iter_mut().zip(smooth) {
                    if s.length_squared() > f32::EPSILON {
                        vertex.normal = s.normalize().to_array();
                    }
                }
            }
        }
        smooth_duplicate_position_normals(vertices);
        return;
    }

    let normals = smooth_normals(vertices, indices);
    for (vertex, normal) in vertices.iter_mut().zip(normals) {
        if !normal_is_usable(vertex.normal) && normal.length_squared() > f32::EPSILON {
            vertex.normal = normal.normalize().to_array();
        }
    }
    smooth_duplicate_position_normals(vertices);
}

fn compute_smooth_normals(vertices: &mut [Vertex], indices: &[u32]) {
    let normals = smooth_normals(vertices, indices);
    for (vertex, normal) in vertices.iter_mut().zip(normals) {
        vertex.normal = if normal.length_squared() > f32::EPSILON {
            normal.normalize().to_array()
        } else {
            Vec3::Z.to_array()
        };
    }
}

fn smooth_normals(vertices: &[Vertex], indices: &[u32]) -> Vec<Vec3> {
    accumulate_smooth_normals(vertices.len(), indices, |index| {
        vertices
            .get(index)
            .map(|vertex| Vec3::from_array(vertex.position))
    })
}

/// Area-weighted vertex normals, from a triangle list and a position lookup.
///
/// The lookup is a closure rather than a `&[Vec3]` so a caller holding
/// interleaved vertices does not have to copy every position out first -- on a
/// six-million-vertex scan that copy would be seventy megabytes to avoid one
/// duplicated loop.
///
/// A triangle with an out-of-range corner is skipped rather than trusted.
#[must_use]
pub fn accumulate_smooth_normals(
    vertex_count: usize,
    indices: &[u32],
    position: impl Fn(usize) -> Option<Vec3>,
) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; vertex_count];
    for triangle in indices.chunks_exact(3) {
        let ia = triangle[0] as usize;
        let ib = triangle[1] as usize;
        let ic = triangle[2] as usize;
        let (Some(a), Some(b), Some(c)) = (position(ia), position(ib), position(ic)) else {
            continue;
        };
        let face_normal = (b - a).cross(c - a);
        let longest_edge_sq = (b - a)
            .length_squared()
            .max((c - b).length_squared())
            .max((a - c).length_squared());
        if face_normal.is_finite()
            && face_normal.length_squared()
                > longest_edge_sq * longest_edge_sq * DEGENERATE_AREA_SIN
        {
            normals[ia] += face_normal;
            normals[ib] += face_normal;
            normals[ic] += face_normal;
        }
    }
    normals
}

fn smooth_duplicate_position_normals(vertices: &mut [Vertex]) {
    if vertices.is_empty() {
        return;
    }

    // Pair each vertex with its quantized position key, then sort by
    // `(key, original_index)`. Equal keys land in a contiguous run — this
    // replaces the old `HashMap<[i32; 3], Vec<usize>>` grouping with a single
    // sort, avoiding one hashmap entry + one `Vec` allocation per group.
    // Sorting on the original index as a tiebreaker keeps each run in
    // ascending vertex-index order, exactly matching the old insertion order
    // (groups were built by iterating vertices 0..n), which is required for
    // bit-identical floating point summation below.
    let mut keyed: Vec<([i32; 3], usize)> = vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| (position_key(vertex.position), index))
        .collect();
    keyed.par_sort_unstable();

    let source_normals: Vec<Vec3> = vertices
        .iter()
        .map(|vertex| {
            let normal = Vec3::from_array(vertex.normal);
            if normal.is_finite() && normal.length_squared() > f32::EPSILON {
                normal.normalize()
            } else {
                Vec3::ZERO
            }
        })
        .collect();
    let mut smoothed = source_normals.clone();

    // Find contiguous equal-key runs (duplicate-position groups). This is a
    // cheap linear scan next to the O(n log n) sort above. Single-member
    // runs need no averaging: `smoothed` already holds their normalized
    // normal, matching the old `filter(|indices| indices.len() > 1)`.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut run_start = 0usize;
    for i in 1..=keyed.len() {
        if i == keyed.len() || keyed[i].0 != keyed[run_start].0 {
            if i - run_start > 1 {
                runs.push((run_start, i));
            }
            run_start = i;
        }
    }

    // Each run touches a disjoint set of vertex indices, so runs can be
    // averaged independently in parallel.
    //
    // One slot per keyed entry, not a collected `Vec<(usize, Vec3)>` of
    // updates. That tuple vector was the largest temporary in the loader: 24
    // bytes per duplicated vertex, and an STL soup duplicates nearly all of
    // them, and rayon's collect builds it per thread before concatenating, so
    // the peak is a multiple again. Slots are sized by total run length, so a
    // welded mesh allocates almost nothing and a soup pays 12 bytes per vertex
    // once, with no unsafe and no concurrent write.
    //
    // `Vec3::ZERO` means "kept its own normal". Unambiguous, because every
    // value written below is normalized.
    let duplicated: usize = runs.iter().map(|&(start, end)| end - start).sum();
    let mut slots: Vec<Vec3> = vec![Vec3::ZERO; duplicated];
    let mut run_slices: Vec<(usize, &mut [Vec3])> = Vec::with_capacity(runs.len());
    {
        let mut rest: &mut [Vec3] = &mut slots;
        for &(start, end) in &runs {
            let (piece, tail) = rest.split_at_mut(end - start);
            run_slices.push((start, piece));
            rest = tail;
        }
    }

    run_slices.into_par_iter().for_each(|(start, out)| {
        let end = start + out.len();
        average_duplicate_run(&keyed[start..end], &source_normals, out);
    });

    let mut written = 0usize;
    for &(start, end) in &runs {
        for (slot, &(_, index)) in slots[written..written + (end - start)]
            .iter()
            .zip(&keyed[start..end])
        {
            if slot.length_squared() > f32::EPSILON {
                smoothed[index] = *slot;
            }
        }
        written += end - start;
    }

    for (vertex, normal) in vertices.iter_mut().zip(smoothed) {
        if normal.length_squared() > f32::EPSILON {
            vertex.normal = normal.to_array();
        }
    }
}

/// How many directions one coincident group may hold before it is left alone.
///
/// A pile that genuinely points sixteen ways is not a surface any averaging can
/// help, and the pass costs one dot product per member per cluster.
const MAX_DUPLICATE_CLUSTERS: usize = 16;

/// Average a large coincident group by clustering it, not by one global mean.
///
/// A single mean is right while the group points one way and wrong the moment
/// it does not. K coincident vertices at a hard crease in a triangle soup form
/// two clusters ninety degrees apart; the mean lands on the bisector, both
/// clusters agree with it to within sixty degrees, and every member is welded
/// to the bisector. The crease is gone. Measured on a 400-member pile split in
/// two: 45 degrees of error on all 400. Exactly opposed clusters are worse
/// still -- the mean cancels and the group is skipped entirely.
///
/// Members join the first cluster they agree with, so a coherent group forms
/// one cluster and gets what the mean would have given it while a crease keeps
/// its two. Cost stays linear in the group for any bounded cluster count.
fn average_by_cluster(members: &[([i32; 3], usize)], source_normals: &[Vec3], out: &mut [Vec3]) {
    let mut sums: Vec<Vec3> = Vec::new();
    let mut assigned: Vec<Option<usize>> = vec![None; members.len()];

    for (slot, &(_, index)) in members.iter().enumerate() {
        let current = source_normals[index];
        if current.length_squared() <= f32::EPSILON {
            continue;
        }
        let existing = sums
            .iter()
            .position(|sum| sum.normalize_or_zero().dot(current) >= SMOOTH_DUPLICATE_NORMAL_DOT);
        if let Some(cluster) = existing {
            sums[cluster] += current;
            assigned[slot] = Some(cluster);
        } else {
            if sums.len() == MAX_DUPLICATE_CLUSTERS {
                // Too many directions to be a surface. Leave them as they
                // arrived; an invented average here smears the crease.
                return;
            }
            sums.push(current);
            assigned[slot] = Some(sums.len() - 1);
        }
    }

    for (slot, cluster) in assigned.iter().enumerate() {
        let Some(cluster) = *cluster else { continue };
        let mean = sums[cluster].normalize_or_zero();
        if mean.length_squared() > f32::EPSILON {
            out[slot] = mean;
        }
    }
}

/// Average one run of coincident vertices into `out`, one slot per member.
///
/// `Vec3::ZERO` is left where a member keeps its own normal; every value
/// written is normalized, so zero is unambiguous.
fn average_duplicate_run(members: &[([i32; 3], usize)], source_normals: &[Vec3], out: &mut [Vec3]) {
    // The exact form below compares every member against every other: fine at
    // real valences, quadratic at absurd ones. Piles of coincident vertices
    // are not hypothetical -- a fan collapsed by bad decimation, a scanner
    // artefact, or a file written to be one. k=2000 costs 19 ms, k=8000 costs
    // 214 ms, k=20000 costs 1.3 s, on the loading thread with no cancellation,
    // or inside `dllhost` holding one of twelve thumbnail lanes long after
    // Explorer has been told the request timed out.
    //
    // Past the threshold, cluster the group in one greedy pass instead.
    if members.len() > MAX_PAIRWISE_DUPLICATE_GROUP {
        average_by_cluster(members, source_normals, out);
        return;
    }

    for (slot, &(_, index)) in members.iter().enumerate() {
        let current = source_normals[index];
        if current.length_squared() <= f32::EPSILON {
            continue;
        }

        let mut normal = Vec3::ZERO;
        for &(_, neighbor) in members {
            let candidate = source_normals[neighbor];
            if candidate.length_squared() > f32::EPSILON
                && candidate.dot(current) >= SMOOTH_DUPLICATE_NORMAL_DOT
            {
                normal += candidate;
            }
        }

        if normal.length_squared() > f32::EPSILON {
            out[slot] = normal.normalize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_crease_survives_a_group_past_the_pairwise_threshold() {
        // Two clusters ninety degrees apart at one position: a hard crease in
        // a triangle soup, where K coincident vertices means K triangles
        // meeting one point. Judging the whole group against its own mean put
        // that mean on the bisector, accepted both clusters into it -- 0.707
        // is above the 0.5 threshold -- and welded the crease flat. Measured
        // before the fix: 45 degrees of error on every member.
        let group = MAX_PAIRWISE_DUPLICATE_GROUP + 144;
        let mut vertices = Vec::with_capacity(group);
        for i in 0..group {
            let mut vertex = Vertex::at(Vec3::ZERO);
            vertex.normal = if i % 2 == 0 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            vertices.push(vertex);
        }

        smooth_duplicate_position_normals(&mut vertices);

        for vertex in &vertices {
            let normal = Vec3::from_array(vertex.normal);
            assert!(
                normal.dot(Vec3::Y) > 0.99 || normal.dot(Vec3::X) > 0.99,
                "a member of the crease came out at {normal:?}: it should \
                 still point the way its own cluster does, not along the \
                 bisector"
            );
        }
        assert!(
            vertices
                .iter()
                .any(|vertex| Vec3::from_array(vertex.normal).dot(Vec3::Y) > 0.99),
            "one side of the crease disappeared"
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| Vec3::from_array(vertex.normal).dot(Vec3::X) > 0.99),
            "the other side of the crease disappeared"
        );
    }

    #[test]
    fn a_coherent_group_past_the_threshold_still_averages_to_one_normal() {
        // The counterweight: clustering must not stop a genuinely coherent
        // pile from being smoothed, which is what the bounded path is for.
        let group = MAX_PAIRWISE_DUPLICATE_GROUP + 144;
        let mut vertices = Vec::with_capacity(group);
        for i in 0..group {
            let angle = i as f32 * 0.0005;
            let mut vertex = Vertex::at(Vec3::ZERO);
            vertex.normal = [angle.sin(), angle.cos(), 0.0];
            vertices.push(vertex);
        }

        smooth_duplicate_position_normals(&mut vertices);

        let first = Vec3::from_array(vertices[0].normal);
        for vertex in &vertices {
            let normal = Vec3::from_array(vertex.normal);
            assert!(
                normal.dot(first) > 0.9999,
                "a coherent group should come out as one normal, got \
                 {normal:?} against {first:?}"
            );
        }
    }

    /// Deterministic LCG (NOT the `rand` crate) so parity tests are
    /// reproducible without a dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            // Numerical Recipes constants.
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 32) as u32
        }

        fn next_f32(&mut self, lo: f32, hi: f32) -> f32 {
            let t = f32::from(u16::try_from(self.next_u32() >> 16).unwrap_or(u16::MAX))
                / f32::from(u16::MAX);
            lo + t * (hi - lo)
        }
    }

    /// Brute-force reference replicating the OLD `HashMap`-based grouping and
    /// averaging algorithm exactly (pre-sort-based-rewrite), used to prove
    /// the new implementation is bit-identical.
    fn brute_force_smooth_duplicate_position_normals(vertices: &mut [Vertex]) {
        let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::with_capacity(vertices.len());
        for (index, vertex) in vertices.iter().enumerate() {
            groups
                .entry(position_key(vertex.position))
                .or_default()
                .push(index);
        }

        let source_normals: Vec<Vec3> = vertices
            .iter()
            .map(|vertex| {
                let normal = Vec3::from_array(vertex.normal);
                if normal.is_finite() && normal.length_squared() > f32::EPSILON {
                    normal.normalize()
                } else {
                    Vec3::ZERO
                }
            })
            .collect();
        let mut smoothed = source_normals.clone();

        for indices in groups.values().filter(|indices| indices.len() > 1) {
            for &index in indices {
                let current = source_normals[index];
                if current.length_squared() <= f32::EPSILON {
                    continue;
                }

                let mut normal = Vec3::ZERO;
                for &neighbor in indices {
                    let candidate = source_normals[neighbor];
                    if candidate.length_squared() > f32::EPSILON
                        && candidate.dot(current) >= SMOOTH_DUPLICATE_NORMAL_DOT
                    {
                        normal += candidate;
                    }
                }

                if normal.length_squared() > f32::EPSILON {
                    smoothed[index] = normal.normalize();
                }
            }
        }

        for (vertex, normal) in vertices.iter_mut().zip(smoothed) {
            if normal.length_squared() > f32::EPSILON {
                vertex.normal = normal.to_array();
            }
        }
    }

    fn assert_bitwise_equal_normals(a: &[Vertex], b: &[Vertex]) {
        assert_eq!(a.len(), b.len());
        for (va, vb) in a.iter().zip(b) {
            for i in 0..3 {
                assert_eq!(
                    va.normal[i].to_bits(),
                    vb.normal[i].to_bits(),
                    "normal component {i} differs: {:?} vs {:?}",
                    va.normal,
                    vb.normal
                );
            }
        }
    }

    #[test]
    fn sort_based_matches_brute_force_on_randomized_soup() {
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        // A small pool of quantized positions reused across many vertices
        // guarantees plenty of duplicate-position groups of varying size.
        let pool: Vec<[f32; 3]> = (0..12)
            .map(|_| {
                [
                    rng.next_f32(-2.0, 2.0),
                    rng.next_f32(-2.0, 2.0),
                    rng.next_f32(-2.0, 2.0),
                ]
            })
            .collect();

        let mut vertices: Vec<Vertex> = Vec::new();
        for _ in 0..500 {
            let pos = pool[(rng.next_u32() as usize) % pool.len()];
            let raw = Vec3::new(
                rng.next_f32(-1.0, 1.0),
                rng.next_f32(-1.0, 1.0),
                rng.next_f32(-1.0, 1.0),
            );
            let normal = if raw.length_squared() > f32::EPSILON {
                raw.normalize()
            } else {
                Vec3::Z
            };
            vertices.push(Vertex::at(Vec3::from_array(pos)).with_normal(normal));
        }

        let mut expected = vertices.clone();
        brute_force_smooth_duplicate_position_normals(&mut expected);
        let mut actual = vertices.clone();
        smooth_duplicate_position_normals(&mut actual);

        assert_bitwise_equal_normals(&expected, &actual);
    }

    #[test]
    fn sort_based_matches_brute_force_with_disagreeing_normals_across_threshold() {
        // Same quantized position, four vertices: two normals that agree
        // with each other (dot above threshold), one that disagrees (dot
        // near zero), and a near-duplicate position within the quantization
        // epsilon — exercises both branches of the dot-threshold check plus
        // the position-quantization tolerance.
        let base = Vec3::new(0.1, -0.2, 0.3);
        let agree_a = Vec3::new(0.0, 0.0, 1.0);
        let agree_b = Vec3::new(0.05, 0.0, 0.999).normalize();
        let disagree = Vec3::new(1.0, 0.0, 0.0);

        let vertices = vec![
            Vertex::at(base).with_normal(agree_a),
            Vertex::at(base).with_normal(agree_b),
            Vertex::at(base).with_normal(disagree),
            Vertex::at(base + Vec3::new(0.0007, -0.0004, 0.0002)).with_normal(agree_a),
        ];

        let mut expected = vertices.clone();
        brute_force_smooth_duplicate_position_normals(&mut expected);
        let mut actual = vertices.clone();
        smooth_duplicate_position_normals(&mut actual);

        assert_bitwise_equal_normals(&expected, &actual);
    }

    #[test]
    fn sort_based_matches_brute_force_on_empty_and_single_triangle() {
        let mut empty: Vec<Vertex> = Vec::new();
        let mut empty_expected: Vec<Vertex> = Vec::new();
        smooth_duplicate_position_normals(&mut empty);
        brute_force_smooth_duplicate_position_normals(&mut empty_expected);
        assert_bitwise_equal_normals(&empty, &empty_expected);

        let vertices = vec![
            Vertex::at(Vec3::ZERO).with_normal(Vec3::Z),
            Vertex::at(Vec3::X).with_normal(Vec3::Z),
            Vertex::at(Vec3::Y).with_normal(Vec3::Z),
        ];
        let mut expected = vertices.clone();
        brute_force_smooth_duplicate_position_normals(&mut expected);
        let mut actual = vertices.clone();
        smooth_duplicate_position_normals(&mut actual);

        assert_bitwise_equal_normals(&expected, &actual);
    }
}

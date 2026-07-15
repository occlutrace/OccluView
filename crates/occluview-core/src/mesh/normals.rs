use super::Vertex;
use glam::Vec3;
use rayon::prelude::*;

const SMOOTH_DUPLICATE_NORMAL_DOT: f32 = 0.5;
const SMOOTH_POSITION_EPS_MM: f32 = 0.002;

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
        // Every vertex already has a usable normal, so the fill-in loop below
        // would be a no-op for all of them — skip the O(triangles) smoothing
        // pass entirely and go straight to duplicate-position averaging.
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
    let mut normals = vec![Vec3::ZERO; vertices.len()];
    for triangle in indices.chunks_exact(3) {
        let ia = triangle[0] as usize;
        let ib = triangle[1] as usize;
        let ic = triangle[2] as usize;
        let a = Vec3::from_array(vertices[ia].position);
        let b = Vec3::from_array(vertices[ib].position);
        let c = Vec3::from_array(vertices[ic].position);
        let face_normal = (b - a).cross(c - a);
        if face_normal.is_finite() && face_normal.length_squared() > f32::EPSILON {
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
    // averaged independently in parallel. Updates are collected rather than
    // written directly, then scattered back serially to avoid unsafe
    // concurrent writes into `smoothed`.
    let updates: Vec<(usize, Vec3)> = runs
        .par_iter()
        .flat_map_iter(|&(start, end)| {
            let members = &keyed[start..end];
            let mut group_updates = Vec::new();
            for &(_, index) in members {
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
                    group_updates.push((index, normal.normalize()));
                }
            }
            group_updates
        })
        .collect();

    for (index, normal) in updates {
        smoothed[index] = normal;
    }

    for (vertex, normal) in vertices.iter_mut().zip(smoothed) {
        if normal.length_squared() > f32::EPSILON {
            vertex.normal = normal.to_array();
        }
    }
}

fn position_key(position: [f32; 3]) -> [i32; 3] {
    [
        position_lane_key(position[0]),
        position_lane_key(position[1]),
        position_lane_key(position[2]),
    ]
}

#[allow(clippy::cast_possible_truncation)]
fn position_lane_key(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }

    let scaled = f64::from(value / SMOOTH_POSITION_EPS_MM).round();
    if scaled <= f64::from(i32::MIN) {
        i32::MIN
    } else if scaled >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        scaled as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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

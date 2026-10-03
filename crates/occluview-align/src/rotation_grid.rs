//! Deterministic Haar-coordinate rotation proposals.
//!
//! Yershova, `LaValle` and Mitchell, "Generating Uniform Incremental Grids on
//! SO(3) Using the Hopf Fibration", <https://lavalle.pl/anna/papers/YerLavMit08.pdf>.
//! This independently derived product grid uses equal increments of squared
//! complex-plane radius and two phases. It is not the paper's incremental
//! grid. Antipodal integer indices are paired before evaluating trigonometry;
//! polar caps close the radial gaps. Independent tests derive a uniform full-grid
//! bound; a truncated prefix makes no such claim.

use crate::SearchProfile;
use glam::DQuat;

/// Visit polar-complete rotations: 444 Standard, 3,480 Extended, none Local.
/// The first 72 nodes are a frozen farthest-first prefix relative to Start;
/// remaining nodes retain integer lexicographic order. Ordinals identify the
/// recipe node, independent of visit order. Incomplete prefixes have no full
/// covering certificate. Returning false stops before visiting another node.
pub(crate) fn hopf_grid(profile: SearchProfile, mut visit: impl FnMut(u32, DQuat) -> bool) {
    let (radial, phases, prefix): (u32, u32, &[u32]) = match profile {
        SearchProfile::Standard => (6, 12, &STANDARD_PREFIX),
        SearchProfile::Extended => (12, 24, &EXTENDED_PREFIX),
        SearchProfile::Local => return,
    };
    for &ordinal in prefix {
        if !visit(ordinal, grid_node(radial, phases, ordinal)) {
            return;
        }
    }
    let total = radial * phases * phases / 2 + phases;
    for ordinal in 0..total {
        if !prefix.contains(&ordinal) && !visit(ordinal, grid_node(radial, phases, ordinal)) {
            return;
        }
    }
}

fn grid_node(radial: u32, phases: u32, ordinal: u32) -> DQuat {
    let interior = radial * phases * phases / 2;
    if ordinal >= interior {
        let cap = ordinal - interior;
        let angle =
            std::f64::consts::TAU * (f64::from(cap % (phases / 2)) + 0.5) / f64::from(phases);
        let (s, c) = angle.sin_cos();
        return canonical_rotation(if cap < phases / 2 {
            DQuat::from_xyzw(s, c, 0., 0.)
        } else {
            DQuat::from_xyzw(0., 0., s, c)
        });
    }
    let radial_id = ordinal / (phases * phases / 2);
    let first_phase_id = ordinal / phases % (phases / 2);
    let second_phase_id = ordinal % phases;
    let radius_squared = (f64::from(radial_id) + 0.5) / f64::from(radial);
    let first_phase = std::f64::consts::TAU * (f64::from(first_phase_id) + 0.5) / f64::from(phases);
    let second_phase =
        std::f64::consts::TAU * (f64::from(second_phase_id) + 0.5) / f64::from(phases);
    canonical_rotation(DQuat::from_xyzw(
        (1. - radius_squared).sqrt() * first_phase.sin(),
        (1. - radius_squared).sqrt() * first_phase.cos(),
        radius_squared.sqrt() * second_phase.sin(),
        radius_squared.sqrt() * second_phase.cos(),
    ))
}

// Greedy farthest-first nodes maximize min antipodal geodesic distance from
// identity and preceding nodes. Dot ties rounded to 12 decimals use ordinal.
const STANDARD_PREFIX: [u32; 72] = [
    432, 435, 374, 232, 235, 238, 268, 274, 265, 271, 168, 216, 86, 442, 101, 104, 132, 135, 140,
    23, 53, 369, 439, 59, 18, 392, 429, 293, 315, 318, 335, 350, 353, 147, 182, 189, 26, 77, 80,
    112, 115, 119, 301, 348, 437, 82, 106, 137, 142, 329, 73, 122, 311, 342, 128, 324, 92, 321,
    339, 344, 225, 161, 192, 197, 218, 242, 285, 156, 360, 171, 236, 88,
];
const EXTENDED_PREFIX: [u32; 72] = [
    3456, 3462, 3473, 1520, 1532, 1664, 1676, 1514, 1526, 1658, 1670, 3476, 1739, 1751, 1883, 1895,
    1445, 1457, 1589, 1601, 3459, 3465, 3170, 2895, 1736, 1748, 1805, 1817, 1874, 1880, 1886, 1892,
    1949, 1961, 2674, 2686, 2818, 2830, 1730, 635, 647, 779, 791, 2941, 1454, 3073, 683, 695, 827,
    839, 2664, 2820, 3003, 3063, 315, 327, 388, 400, 459, 471, 532, 544, 2884, 2886, 2898, 3007,
    3019, 343, 355, 487, 499, 704,
];

/// Canonical antipodal representative of an already finite nonzero rotation.
pub(crate) fn canonical_rotation(rotation: DQuat) -> DQuat {
    let rotation = rotation.normalize();
    let sign = [rotation.w, rotation.z, rotation.y, rotation.x]
        .into_iter()
        .find(|x| *x != 0.)
        .unwrap_or(1.);
    if sign < 0. {
        -rotation
    } else {
        rotation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_explicit_poles_meet_covering_bound() {
        let mut grid = Vec::new();
        hopf_grid(SearchProfile::Extended, |_, q| {
            grid.push(q);
            true
        });
        for q in [DQuat::from_xyzw(1., 0., 0., 0.), DQuat::IDENTITY] {
            let dot = grid.iter().map(|p| p.dot(q).abs()).fold(0., f64::max);
            let error = 2. * dot.clamp(0., 1.).acos().to_degrees();
            assert!(error <= 25., "pole error {error}");
        }
    }

    #[test]
    fn polar_grid_has_unique_nodes_and_uniform_bound() {
        for (profile, radial, phases, limit, prefix) in [
            (SearchProfile::Standard, 6u32, 12u32, 45., &STANDARD_PREFIX),
            (SearchProfile::Extended, 12, 24, 25., &EXTENDED_PREFIX),
        ] {
            let mut nodes = Vec::new();
            hopf_grid(profile, |id, q| {
                nodes.push((id, q));
                true
            });
            let ids: std::collections::BTreeSet<_> = nodes.iter().map(|p| p.0).collect();
            assert_eq!(ids.len(), nodes.len());
            let mut phis = vec![0., std::f64::consts::FRAC_PI_2];
            phis.extend(
                (0..radial).map(|i| ((f64::from(i) + 0.5) / f64::from(radial)).sqrt().asin()),
            );
            phis.sort_by(f64::total_cmp);
            let delta = phis
                .windows(2)
                .map(|p| (p[1] - p[0]) * 0.5)
                .fold(0., f64::max);
            let bound = 2.
                * (delta.cos() * (std::f64::consts::PI / f64::from(phases)).cos())
                    .acos()
                    .to_degrees();
            assert!(bound <= limit, "uniform bound {bound}");
            for i in 0..nodes.len() {
                for j in i + 1..nodes.len() {
                    assert!(nodes[i].1.dot(nodes[j].1).abs() < 1. - 1e-12);
                }
            }
            // Independently reconstruct greedy ordering, with the documented
            // rounded dot tie and ordinal, rather than testing a copied list.
            let mut nearest: Vec<_> = (0..u32::try_from(nodes.len()).unwrap())
                .map(|id| grid_node(radial, phases, id).w.abs())
                .collect();
            for &expected in prefix {
                let selected = nearest
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        (a.1 * 1e12)
                            .round()
                            .total_cmp(&(b.1 * 1e12).round())
                            .then(a.0.cmp(&b.0))
                    })
                    .unwrap()
                    .0;
                assert_eq!(selected, expected as usize);
                let q = grid_node(radial, phases, expected);
                for (i, distance) in nearest.iter_mut().enumerate() {
                    *distance = distance.max(
                        q.dot(grid_node(radial, phases, u32::try_from(i).unwrap()))
                            .abs(),
                    );
                }
            }
        }
    }

    /// ID38: 100,000 frozen Haar rotations, including the near-pole cells.
    #[test]
    fn grid_and_area_sampler_are_measured_grid_part() {
        let mut grids = [Vec::new(), Vec::new()];
        for (grid, profile) in grids
            .iter_mut()
            .zip([SearchProfile::Standard, SearchProfile::Extended])
        {
            hopf_grid(profile, |_, q| {
                grid.push(q);
                true
            });
            for q in grid.iter() {
                assert!((q.length() - 1.).abs() <= 1e-12);
                assert!((glam::DMat3::from_quat(*q).determinant() - 1.).abs() <= 1e-10);
            }
        }
        assert_eq!(grids[0].len(), 444);
        assert_eq!(grids[1].len(), 3_480);
        let mut state = 0x7a21_4351_90aa_831du64;
        let mut uniform = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            #[allow(clippy::cast_precision_loss)]
            {
                ((z ^ (z >> 31)) >> 11) as f64 / 9_007_199_254_740_992.
            }
        };
        let mut maximum = [0f64; 2];
        for _ in 0..100_000 {
            let u = uniform();
            let a = std::f64::consts::TAU * uniform();
            let b = std::f64::consts::TAU * uniform();
            let truth = DQuat::from_xyzw(
                (1. - u).sqrt() * a.sin(),
                (1. - u).sqrt() * a.cos(),
                u.sqrt() * b.sin(),
                u.sqrt() * b.cos(),
            );
            for (i, grid) in grids.iter().enumerate() {
                let dot = grid.iter().map(|q| q.dot(truth).abs()).fold(0., f64::max);
                maximum[i] = maximum[i].max(2. * dot.clamp(0., 1.).acos().to_degrees());
            }
        }
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.refactor-scratch");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("gate38-measurements.txt"),
            format!("sampled_max_degrees={maximum:?}\n"),
        )
        .unwrap();
        assert!(maximum[0] <= 45., "Standard covering error {maximum:?}");
        assert!(maximum[1] <= 25., "Extended covering error {maximum:?}");
        let mut stopped = 0;
        hopf_grid(SearchProfile::Extended, |_, _| {
            stopped += 1;
            stopped < 7
        });
        assert_eq!(stopped, 7);
    }
}

//! Rotation-invariant descriptions of the surface around each cloud point.
//!
//! Rusu, Blodow and Beetz, Fast Point Feature Histograms (FPFH) for 3D
//! Registration, ICRA 2009, <https://doi.org/10.1109/ROBOT.2009.5152473>:
//! the three angles between a point's normal, a neighbour's normal and the
//! line joining them are gathered into histograms, and each point's
//! histograms are blended with those of its neighbours. Here the description
//! is taken at two radii and joined, values are shared between adjacent bins
//! so that a small change of angle is a small change of description, and the
//! stored values are square roots, which makes the plain distance between two
//! descriptions the Hellinger distance between their histograms.

use super::cloud::Cloud;
use super::solve::positive;
use glam::DVec3;
use rayon::prelude::*;

/// Bins per angle.
const BINS: usize = 12;
/// Values of one radius: three angles.
const BLOCK: usize = 3 * BINS;
/// Radii joined into one description.
pub(super) const SCALES: usize = 2;
/// Values of one description.
pub(super) const SIZE: usize = BLOCK * SCALES;

pub(super) type Descriptor = [f32; SIZE];

/// The three angles of one ordered pair, each mapped onto `0..1`.
fn pair_angles(p: DVec3, n: DVec3, q: DVec3, m: DVec3) -> Option<[f64; 3]> {
    let line = q - p;
    let length = line.length();
    if !positive(length) {
        return None;
    }
    let mut line = line / length;
    let (along_first, along_second) = (n.dot(line), m.dot(line));
    // The frame stands on the point whose normal is closer to the line.
    let (first, second, lean) = if along_first.abs() < along_second.abs() {
        line = -line;
        (m, n, -along_second)
    } else {
        (n, m, along_first)
    };
    let side = line.cross(first).try_normalize()?;
    let up = first.cross(side);
    let turn = up.dot(second).atan2(first.dot(second));
    Some([
        (turn + std::f64::consts::PI) / std::f64::consts::TAU,
        f64::midpoint(side.dot(second), 1.),
        f64::midpoint(lean, 1.),
    ])
}

/// Share `weight` between the two bins nearest to `value` in `0..1`. The
/// first angle is periodic; the other two end at their limits.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn deposit(histogram: &mut [f64], value: f64, weight: f64, periodic: bool) {
    let last = (BINS - 1) as f64;
    // Bin centres stand at half steps, so the place is taken half a bin up
    // and stays above zero.
    let place = value.clamp(0., 1.) * BINS as f64 + 0.5;
    let low = place.floor();
    let share = place - low;
    let bin = |above: f64| {
        let index = above - 1.;
        if periodic {
            (if index < 0. {
                last
            } else {
                index % BINS as f64
            }) as usize
        } else {
            index.clamp(0., last) as usize
        }
    };
    histogram[bin(low)] += weight * (1. - share);
    histogram[bin(low + 1.)] += weight * share;
}

/// Neighbours of every point within `radius`, with their distances.
fn neighbours(cloud: &Cloud, radius: f64) -> Vec<Vec<(u32, f64)>> {
    cloud
        .points
        .par_iter()
        .enumerate()
        .map(|(ordinal, point)| {
            let mut found = Vec::new();
            cloud.within(point.position, radius, |other, squared| {
                if other as usize != ordinal {
                    found.push((other, squared.sqrt()));
                }
            });
            found.sort_unstable_by_key(|&(other, _)| other);
            found
        })
        .collect()
}

/// One radius: every point's own histograms, then the blend with its
/// neighbours' histograms, nearer neighbours counting more.
fn blended(cloud: &Cloud, sign: f64, radius: f64) -> Vec<[f64; BLOCK]> {
    let near = neighbours(cloud, radius);
    let own: Vec<[f64; BLOCK]> = near
        .par_iter()
        .enumerate()
        .map(|(ordinal, found)| {
            let point = &cloud.points[ordinal];
            let mut histogram = [0f64; BLOCK];
            for &(other, _) in found {
                let neighbour = &cloud.points[other as usize];
                let Some(angles) = pair_angles(
                    point.position,
                    point.normal * sign,
                    neighbour.position,
                    neighbour.normal * sign,
                ) else {
                    continue;
                };
                for (slot, &value) in angles.iter().enumerate() {
                    deposit(
                        &mut histogram[slot * BINS..(slot + 1) * BINS],
                        value,
                        neighbour.area,
                        slot == 0,
                    );
                }
            }
            normalize(&mut histogram);
            histogram
        })
        .collect();
    near.par_iter()
        .enumerate()
        .map(|(ordinal, found)| {
            let mut histogram = [0f64; BLOCK];
            let mut total = 0.;
            for &(other, distance) in found {
                let weight = cloud.points[other as usize].area / (distance + cloud.spacing * 0.5);
                total += weight;
                for (value, &theirs) in histogram.iter_mut().zip(&own[other as usize]) {
                    *value += theirs * weight;
                }
            }
            if total > 0. {
                for value in &mut histogram {
                    *value /= total;
                }
            }
            for (value, &mine) in histogram.iter_mut().zip(&own[ordinal]) {
                *value += mine;
            }
            normalize(&mut histogram);
            histogram
        })
        .collect()
}

/// Bring each angle's histogram to unit sum; an empty one stays empty.
fn normalize(histogram: &mut [f64; BLOCK]) {
    for angle in histogram.as_chunks_mut::<BINS>().0 {
        let sum: f64 = angle.iter().sum();
        if sum > 0. {
            for value in angle {
                *value /= sum;
            }
        }
    }
}

/// Describe every point of the cloud at the given radii. `sign` is -1 when
/// the surface is to be read facing the other way.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn describe(cloud: &Cloud, sign: f64, radii: [f64; SCALES]) -> Vec<Descriptor> {
    let mut out = vec![[0f32; SIZE]; cloud.points.len()];
    for (scale, radius) in radii.into_iter().enumerate() {
        for (descriptor, histogram) in out.iter_mut().zip(blended(cloud, sign, radius)) {
            for (value, bin) in descriptor[scale * BLOCK..(scale + 1) * BLOCK]
                .iter_mut()
                .zip(histogram)
            {
                *value = bin.sqrt() as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_precision_loss)]
    use super::super::cloud::Gather;
    use super::*;
    use glam::DQuat;

    /// A bumpy sheet: enough shape for the description to vary.
    fn bumpy(turn: DQuat, shift: DVec3) -> Cloud {
        let height = |x: f64, y: f64| (x * 0.9).sin() * 1.5 + (y * 0.6).cos() * (x * 0.3).sin();
        let at = |i: usize, j: usize| {
            let (x, y) = (i as f64 * 0.5, j as f64 * 0.5);
            turn * DVec3::new(x, y, height(x, y)) + shift
        };
        let mut gather = Gather::new(1.);
        for i in 0..40 {
            for j in 0..40 {
                gather.add_triangle([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                gather.add_triangle([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        gather.finish()
    }

    #[test]
    fn pair_angles_do_not_change_under_a_rigid_motion() {
        let turn = DQuat::from_axis_angle(DVec3::new(0.3, -0.5, 0.8).normalize(), 1.1);
        let shift = DVec3::new(4., -7., 2.);
        let (p, n) = (
            DVec3::new(0.2, 0.1, 0.),
            DVec3::new(0.1, 0.2, 1.).normalize(),
        );
        let (q, m) = (
            DVec3::new(1.4, -0.3, 0.6),
            DVec3::new(-0.4, 0.1, 0.9).normalize(),
        );
        let here = pair_angles(p, n, q, m).unwrap();
        let there = pair_angles(turn * p + shift, turn * n, turn * q + shift, turn * m).unwrap();
        for (a, b) in here.iter().zip(there) {
            assert!((a - b).abs() < 1e-9, "{a} {b}");
        }
        // The pair reads the same from either end.
        let back = pair_angles(q, m, p, n).unwrap();
        for (a, b) in here.iter().zip(back) {
            assert!((a - b).abs() < 1e-9, "{a} {b}");
        }
    }

    #[test]
    fn a_moved_surface_keeps_its_descriptions() {
        let still = bumpy(DQuat::IDENTITY, DVec3::ZERO);
        // A motion that maps the grid cells onto themselves keeps the cloud's
        // points, so their descriptions must agree closely.
        let moved = bumpy(
            DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
            DVec3::new(100., 0., 0.),
        );
        let a = describe(&still, 1., [3., 6.]);
        let b = describe(&moved, 1., [3., 6.]);
        assert_eq!(a.len(), b.len());
        let mut matched = 0;
        for (ordinal, descriptor) in a.iter().enumerate() {
            let here = still.points[ordinal].position;
            let there = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2) * here
                + DVec3::new(100., 0., 0.);
            let Some((other, _)) = moved.nearest(there, 0.3) else {
                continue;
            };
            let distance: f32 = descriptor
                .iter()
                .zip(&b[other as usize])
                .map(|(x, y)| (x - y) * (x - y))
                .sum();
            assert!(distance < 0.05, "{distance}");
            matched += 1;
        }
        assert!(matched > a.len() / 2, "{matched} of {}", a.len());
    }

    #[test]
    fn a_surface_read_the_other_way_is_described_differently() {
        let cloud = bumpy(DQuat::IDENTITY, DVec3::ZERO);
        let front = describe(&cloud, 1., [3., 6.]);
        let back = describe(&cloud, -1., [3., 6.]);
        let differing = front
            .iter()
            .zip(&back)
            .filter(|(a, b)| {
                a.iter()
                    .zip(b.iter())
                    .map(|(x, y)| (x - y) * (x - y))
                    .sum::<f32>()
                    > 0.05
            })
            .count();
        assert!(differing > front.len() / 2, "{differing}");
    }
}

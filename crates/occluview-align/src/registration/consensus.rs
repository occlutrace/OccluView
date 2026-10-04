//! Motions on which many look-alike pairs agree.
//!
//! A rigid motion keeps the distance between two points and the angles
//! between their normals and the line joining them (the point-pair features
//! of Drost et al., CVPR 2010, <https://doi.org/10.1109/CVPR.2010.5540108>).
//! Two pairs that keep all of these may belong to one motion; two that do not
//! cannot. Pairs that belong to the true motion all agree with one another,
//! while wrong pairs agree only by chance, so a true pair shares many agreeing
//! partners with each of its partners. That second-order count (Chen et al.,
//! SC2-PCR, CVPR 2022, <https://doi.org/10.1109/CVPR52688.2022.01287>) picks
//! the seeds and their supporters here. Nothing is drawn at random.

use super::solve::rigid_from_pairs;
use crate::Rigid;
use glam::DVec3;
use rayon::prelude::*;

/// Pairs closer than this many spacings carry no usable angle.
const MIN_APART: f64 = 4.;
/// Largest change of distance between two agreeing pairs, in spacings.
const LENGTH_TOLERANCE: f64 = 1.2;
/// Largest change of a cosine between two agreeing pairs.
const ANGLE_TOLERANCE: f64 = 0.35;
/// Supporters taken for the first fit of a seed.
const SUPPORTERS: usize = 16;
/// A pair within this many spacings of its partner follows the motion.
const FOLLOW: f64 = 1.5;
/// Fits repeated on the pairs that follow.
const REFITS: usize = 3;
/// A seed with fewer agreeing partners than this is not a motion.
const MIN_PARTNERS: u32 = 3;
/// Seeds whose order is refined by their agreeing triples.
const LEADING: usize = 192;

/// A moving point and the fixed point it may correspond to.
#[derive(Clone, Copy, Debug)]
pub(super) struct Pairing {
    pub moving: DVec3,
    pub moving_normal: DVec3,
    pub fixed: DVec3,
    pub fixed_normal: DVec3,
}

/// A motion and the number of pairs that follow it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hypothesis {
    pub pose: Rigid,
    pub support: u32,
}

fn agree(a: &Pairing, b: &Pairing, spacing: f64) -> bool {
    let (moving, fixed) = (b.moving - a.moving, b.fixed - a.fixed);
    let (from, to) = (moving.length(), fixed.length());
    if from < MIN_APART * spacing
        || to < MIN_APART * spacing
        || (from - to).abs() > LENGTH_TOLERANCE * spacing
    {
        return false;
    }
    let (moving, fixed) = (moving / from, fixed / to);
    let near = |x: f64, y: f64| (x - y).abs() <= ANGLE_TOLERANCE;
    near(a.moving_normal.dot(moving), a.fixed_normal.dot(fixed))
        && near(b.moving_normal.dot(moving), b.fixed_normal.dot(fixed))
        && near(
            a.moving_normal.dot(b.moving_normal),
            a.fixed_normal.dot(b.fixed_normal),
        )
}

/// Which pairs agree with which, one bit per ordered pair of pairs.
struct Agreement {
    words: usize,
    bits: Vec<u64>,
}

impl Agreement {
    fn new(pairs: &[Pairing], spacing: f64) -> Self {
        let words = pairs.len().div_ceil(64).max(1);
        let mut bits = vec![0u64; pairs.len() * words];
        bits.par_chunks_mut(words).enumerate().for_each(|(i, row)| {
            for (j, other) in pairs.iter().enumerate() {
                if i != j && agree(&pairs[i], other, spacing) {
                    row[j / 64] |= 1 << (j % 64);
                }
            }
        });
        Self { words, bits }
    }

    fn row(&self, i: usize) -> &[u64] {
        &self.bits[i * self.words..(i + 1) * self.words]
    }

    fn has(&self, i: usize, j: usize) -> bool {
        self.row(i)[j / 64] >> (j % 64) & 1 == 1
    }

    fn partners(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.row(i).iter().enumerate().flat_map(|(word, &bits)| {
            (0..64)
                .filter(move |bit| bits >> bit & 1 == 1)
                .map(move |bit| word * 64 + bit)
        })
    }

    /// Partners that `i` and `j` have in common.
    fn shared(&self, i: usize, j: usize) -> u32 {
        self.row(i)
            .iter()
            .zip(self.row(j))
            .map(|(a, b)| (a & b).count_ones())
            .sum()
    }
}

/// Up to `limit` distinct motions, best supported first. `stop` is asked
/// between seeds; the motions found so far are returned when it says so.
pub(super) fn consensus(
    pairs: &[Pairing],
    spacing: f64,
    limit: usize,
    stop: &(dyn Fn() -> bool + Sync),
) -> Vec<Hypothesis> {
    if pairs.len() < 3 || stop() {
        return Vec::new();
    }
    let agreement = Agreement::new(pairs, spacing);
    // Seeds in the order of their agreeing partners. Among the best of them
    // the order is refined by how strongly each is embedded among pairs that
    // agree with one another: the number of agreeing triples it belongs to.
    let partners: Vec<u32> = (0..pairs.len())
        .map(|i| agreement.row(i).iter().map(|word| word.count_ones()).sum())
        .collect();
    let mut seeds: Vec<usize> = (0..pairs.len()).collect();
    seeds.sort_by(|&a, &b| partners[b].cmp(&partners[a]).then(a.cmp(&b)));
    let leading = seeds.len().min(LEADING);
    let embedded: Vec<u64> = seeds[..leading]
        .par_iter()
        .map(|&i| {
            agreement
                .partners(i)
                .map(|j| u64::from(agreement.shared(i, j)))
                .sum()
        })
        .collect();
    let mut order: Vec<usize> = (0..leading).collect();
    order.sort_by(|&a, &b| embedded[b].cmp(&embedded[a]).then(a.cmp(&b)));
    let led: Vec<usize> = order.iter().map(|&slot| seeds[slot]).collect();
    seeds[..leading].copy_from_slice(&led);
    let mut taken = vec![false; pairs.len()];
    let mut found: Vec<Hypothesis> = Vec::new();
    let follow = FOLLOW * spacing;
    for seed in seeds {
        if found.len() >= limit || partners[seed] < MIN_PARTNERS || stop() {
            break;
        }
        if taken[seed] {
            continue;
        }
        taken[seed] = true;
        let mut partners: Vec<(u32, usize)> = agreement
            .partners(seed)
            .map(|j| (agreement.shared(seed, j), j))
            .collect();
        partners.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if partners.first().is_none_or(|best| best.0 < MIN_PARTNERS) {
            continue;
        }
        // The seed's best supporters that also agree with one another.
        let mut group = vec![seed];
        for &(_, j) in &partners {
            if group.iter().all(|&member| agreement.has(member, j)) {
                group.push(j);
                if group.len() > SUPPORTERS {
                    break;
                }
            }
        }
        let Some(mut pose) =
            rigid_from_pairs(group.iter().map(|&i| (pairs[i].moving, pairs[i].fixed, 1.)))
        else {
            continue;
        };
        let mut followers: Vec<usize> = Vec::new();
        for _ in 0..REFITS {
            followers.clear();
            followers.extend((0..pairs.len()).filter(|&i| {
                pose.apply(pairs[i].moving).distance_squared(pairs[i].fixed) <= follow * follow
            }));
            match rigid_from_pairs(
                followers
                    .iter()
                    .map(|&i| (pairs[i].moving, pairs[i].fixed, 1.)),
            ) {
                Some(next) => pose = next,
                None => break,
            }
        }
        for &i in &followers {
            taken[i] = true;
        }
        let support = u32::try_from(followers.len()).unwrap_or(u32::MAX);
        // A motion found again from another seed is the same answer.
        let again = found.iter().any(|other| {
            pairs
                .iter()
                .step_by(pairs.len().div_ceil(64).max(1))
                .all(|pair| {
                    other
                        .pose
                        .apply(pair.moving)
                        .distance_squared(pose.apply(pair.moving))
                        <= follow * follow
                })
        });
        if support >= MIN_PARTNERS && !again {
            found.push(Hypothesis { pose, support });
        }
    }
    found.sort_by_key(|motion| std::cmp::Reverse(motion.support));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    fn surface(index: u32) -> (DVec3, DVec3) {
        let t = f64::from(index);
        let point = DVec3::new(
            (t * 0.61).sin() * 25.,
            (t * 0.37).cos() * 18.,
            (t * 0.83).sin() * 9.,
        );
        (point, (point + DVec3::new(0., 0., 40.)).normalize())
    }

    fn pairs(truth: Rigid, right: u32, wrong: u32) -> Vec<Pairing> {
        (0..right + wrong)
            .map(|i| {
                let (moving, moving_normal) = surface(i);
                // A wrong pair points at an unrelated place of the surface.
                let (to, to_normal) = if i < right {
                    (moving, moving_normal)
                } else {
                    surface(i * 7 + 13)
                };
                Pairing {
                    moving,
                    moving_normal,
                    fixed: truth.apply(to),
                    fixed_normal: truth.rotation * to_normal,
                }
            })
            .collect()
    }

    #[test]
    fn the_motion_of_a_few_right_pairs_is_found_among_many_wrong_ones() {
        let truth = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.6, -0.2, 0.7).normalize(), 2.4),
            DVec3::new(30., -12., 8.),
        );
        let all = pairs(truth, 40, 760);
        let found = consensus(&all, 1., 8, &|| false);
        let best = found.first().unwrap();
        assert!(best.support >= 40, "{}", best.support);
        for i in 0..40 {
            let (point, _) = surface(i);
            // A wrong pair that follows the motion by chance may nudge the fit.
            assert!(best.pose.apply(point).distance(truth.apply(point)) < 0.5);
        }
    }

    #[test]
    fn pairs_without_a_common_motion_give_nothing_strong() {
        let truth = Rigid::IDENTITY;
        let all = pairs(truth, 0, 400);
        let found = consensus(&all, 1., 8, &|| false);
        assert!(found.iter().all(|h| h.support < 12), "{found:?}");
    }

    #[test]
    fn a_stop_request_ends_the_search() {
        let all = pairs(Rigid::IDENTITY, 40, 100);
        assert!(consensus(&all, 1., 8, &|| true).is_empty());
    }
}

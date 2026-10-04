//! Pairs of points whose surroundings look alike.
//!
//! Every point of each cloud is paired with the point of the other cloud
//! whose description is closest. Most such pairs are wrong on partly alike
//! scans; the consensus stage decides which of them agree on one motion.

use super::descriptor::{Descriptor, SIZE};
use rayon::prelude::*;

/// A moving point and the fixed point that looks most like it, or the other
/// way round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Match {
    pub moving: u32,
    pub fixed: u32,
    /// Squared distance between the two descriptions.
    pub distance: f32,
    /// Each is the other's closest.
    pub mutual: bool,
}

/// Values compared between two looks at the running distance.
const STRETCH: usize = 24;
/// Sums kept side by side inside a stretch, so the compiler may take them
/// together; their fixed order keeps the result the same on every machine.
const LANES: usize = 8;
const _: () = assert!(SIZE.is_multiple_of(STRETCH) && STRETCH.is_multiple_of(LANES));

/// Squared distance between two descriptions, given up as soon as it has
/// passed `limit`.
fn squared(a: &Descriptor, b: &Descriptor, limit: f32) -> f32 {
    let mut total = 0f32;
    for (left, right) in a
        .as_chunks::<STRETCH>()
        .0
        .iter()
        .zip(b.as_chunks::<STRETCH>().0)
    {
        let mut lanes = [0f32; LANES];
        for (xs, ys) in left
            .as_chunks::<LANES>()
            .0
            .iter()
            .zip(right.as_chunks::<LANES>().0)
        {
            for lane in 0..LANES {
                let difference = xs[lane] - ys[lane];
                lanes[lane] += difference * difference;
            }
        }
        total += lanes.iter().sum::<f32>();
        if total > limit {
            break;
        }
    }
    total
}

/// For every `stride`-th description in `from`, the ordinal of the closest
/// one in `to`. Equal distances resolve to the lower ordinal.
fn closest(from: &[Descriptor], to: &[Descriptor], stride: usize) -> Vec<Option<(u32, f32)>> {
    from.par_iter()
        .enumerate()
        .map(|(ordinal, description)| {
            if !ordinal.is_multiple_of(stride) {
                return None;
            }
            let mut best: Option<(u32, f32)> = None;
            for (other, theirs) in to.iter().enumerate() {
                let limit = best.map_or(f32::INFINITY, |(_, found)| found);
                let distance = squared(description, theirs, limit);
                if distance < limit {
                    best = Some((u32::try_from(other).unwrap_or(u32::MAX), distance));
                }
            }
            best
        })
        .collect()
}

/// At most `limit` pairs: the mutual ones first, then the rest, each group
/// by rising description distance. Each side asks for at most `asked` of its
/// points, taken at an even stride.
pub(super) fn match_descriptions(
    moving: &[Descriptor],
    fixed: &[Descriptor],
    asked: usize,
    limit: usize,
) -> Vec<Match> {
    let stride = |count: usize| count.div_ceil(asked.max(1)).max(1);
    let forward = closest(moving, fixed, stride(moving.len()));
    let backward = closest(fixed, moving, stride(fixed.len()));
    let mut matches = Vec::with_capacity(moving.len() + fixed.len());
    for (ordinal, found) in forward.iter().enumerate() {
        let Some((other, distance)) = *found else {
            continue;
        };
        let moving = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let mutual = backward
            .get(other as usize)
            .is_some_and(|back| back.is_some_and(|(back, _)| back == moving));
        matches.push(Match {
            moving,
            fixed: other,
            distance,
            mutual,
        });
    }
    for (ordinal, found) in backward.iter().enumerate() {
        let Some((other, distance)) = *found else {
            continue;
        };
        let fixed = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let mutual = forward
            .get(other as usize)
            .is_some_and(|there| there.is_some_and(|(there, _)| there == fixed));
        // A mutual pair is already listed from the moving side.
        if !mutual {
            matches.push(Match {
                moving: other,
                fixed,
                distance,
                mutual,
            });
        }
    }
    matches.sort_by(|a, b| {
        b.mutual
            .cmp(&a.mutual)
            .then(a.distance.total_cmp(&b.distance))
            .then(a.moving.cmp(&b.moving))
            .then(a.fixed.cmp(&b.fixed))
    });
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    use super::*;

    fn description(seed: u32) -> Descriptor {
        let mut out = [0f32; SIZE];
        for (slot, value) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            {
                *value = (((seed * 31 + slot as u32 * 17) % 97) as f32) / 97.;
            }
        }
        out
    }

    #[test]
    fn equal_descriptions_pair_mutually_and_come_first() {
        let moving: Vec<_> = (0..20).map(description).collect();
        let fixed: Vec<_> = (5..40).rev().map(description).collect();
        let matches = match_descriptions(&moving, &fixed, 1_000, 1_000);
        // The fifteen descriptions both sides have pair mutually at no
        // distance, and lead the list.
        for found in &matches[..15] {
            assert!(found.mutual);
            assert_eq!(found.distance, 0.);
            assert_eq!(moving[found.moving as usize], fixed[found.fixed as usize]);
        }
        assert!(matches[15..].iter().all(|found| found.distance > 0.));
        assert_eq!(match_descriptions(&moving, &fixed, 1_000, 7).len(), 7);
        assert_eq!(matches, match_descriptions(&moving, &fixed, 1_000, 1_000));
    }

    #[test]
    fn an_empty_side_gives_no_pairs() {
        let some: Vec<_> = (0..4).map(description).collect();
        assert!(match_descriptions(&some, &[], 10, 10).is_empty());
        assert!(match_descriptions(&[], &some, 10, 10).is_empty());
    }
}
